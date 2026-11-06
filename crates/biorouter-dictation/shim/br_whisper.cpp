// The shim between Biorouter and whisper.cpp; see br_whisper.h.
//
// Every exported function catches every C++ exception and returns an error value, because
// an exception must never unwind into Rust.

#include "br_whisper.h"

#include "ggml-backend.h"
#include "ggml.h"
#include "whisper.h"

#include <algorithm>
#include <cfloat>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>
#include <vector>

namespace {

using clock_type = std::chrono::steady_clock;

double ms_since(clock_type::time_point t0) {
    return std::chrono::duration<double, std::milli>(clock_type::now() - t0).count();
}

// The abort flag is an AtomicI32 on the Rust side; a relaxed load is all a poll needs.
bool flag_set(const volatile int32_t *flag) {
    if (flag == nullptr) {
        return false;
    }
#if defined(_MSC_VER)
    return *flag != 0;
#else
    return __atomic_load_n(const_cast<const int32_t *>(flag), __ATOMIC_RELAXED) != 0;
#endif
}

// ---- model readers --------------------------------------------------------------------

// Wraps a br_reader so close runs exactly once whatever whisper.cpp does with it:
// whisper_init_with_params closes on success and on failure, whisper_vad_init_with_params
// never closes.
struct reader_guard {
    const br_reader *reader;
    bool closed;

    void close() {
        if (!closed) {
            closed = true;
            reader->close(reader->ctx);
        }
    }
};

size_t reader_read(void *ctx, void *out, size_t n) {
    auto *g = static_cast<reader_guard *>(ctx);
    return g->closed ? 0 : g->reader->read(g->reader->ctx, out, n);
}

bool reader_eof(void *ctx) {
    auto *g = static_cast<reader_guard *>(ctx);
    return g->closed || g->reader->eof(g->reader->ctx);
}

void reader_close(void *ctx) {
    static_cast<reader_guard *>(ctx)->close();
}

whisper_model_loader loader_for(reader_guard &guard) {
    whisper_model_loader loader = {};
    loader.context = &guard;
    loader.read = reader_read;
    loader.eof = reader_eof;
    loader.close = reader_close;
    return loader;
}

// ---- logging (DD-13) ------------------------------------------------------------------

std::mutex g_log_mutex;

void log_callback(enum ggml_log_level level, const char *text, void * /*user*/) {
    if (!br_log_should_forward(static_cast<int32_t>(level), text)) {
        return;
    }
    std::lock_guard<std::mutex> lock(g_log_mutex);
    std::fputs(text, stderr);
    std::fflush(stderr);
}

// ---- parameters (DD-47, DD-48) --------------------------------------------------------

whisper_vad_params vad_params_from(const br_vad *v) {
    whisper_vad_params p = whisper_vad_default_params();
    p.threshold = v->threshold;
    p.min_speech_duration_ms = v->min_speech_ms;
    p.min_silence_duration_ms = v->min_silence_ms;
    p.max_speech_duration_s = v->max_speech_s;
    p.speech_pad_ms = v->speech_pad_ms;
    p.samples_overlap = v->samples_overlap_s;
    return p;
}

// One builder for br_full, br_warmup and br_params_snapshot, so the snapshot shows what a
// decode really uses.
whisper_full_params full_params_from(const br_decode *d, const br_vad *v) {
    whisper_full_params p = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    p.n_threads = d->n_threads;
    p.language = d->language;
    p.detect_language = false;
    p.greedy.best_of = d->best_of;
    p.temperature = d->temperature;
    p.temperature_inc = d->temperature_inc;
    p.entropy_thold = d->entropy_thold;
    p.logprob_thold = d->logprob_thold;
    p.no_speech_thold = d->no_speech_thold;
    p.translate = d->translate;
    p.no_context = d->no_context;
    p.no_timestamps = d->no_timestamps;
    p.single_segment = d->single_segment;
    p.suppress_blank = d->suppress_blank;
    p.suppress_nst = d->suppress_nst;
    p.audio_ctx = d->audio_ctx;
    p.max_tokens = d->max_tokens;
    // DD-13: nothing of the transcript or the prompt is ever printed.
    p.print_special = false;
    p.print_progress = false;
    p.print_realtime = false;
    p.print_timestamps = false;
    p.token_timestamps = false;
    p.initial_prompt = nullptr;
    p.carry_initial_prompt = false;
    p.prompt_tokens = nullptr;
    p.prompt_n_tokens = 0;
    p.vad = v->enabled;
    p.vad_model_path = v->model_file;
    p.vad_params = vad_params_from(v);
    return p;
}

// ---- callbacks of br_full -------------------------------------------------------------

struct full_state {
    const volatile int32_t *abort_flag;
    clock_type::time_point t0;
    bool seen_encoder;
    double first_encoder_ms;
    br_logits_cb cb;
    void *cb_user;
    // Set when this shim told whisper.cpp to stop. A stop at an encoder begin makes
    // whisper_full_with_state leave its loop and return 0, so its return code alone
    // cannot tell a cancelled decode from a finished one.
    bool aborted;
};

bool on_encoder_begin(struct whisper_context *, struct whisper_state *, void *user) {
    auto *s = static_cast<full_state *>(user);
    if (!s->seen_encoder) {
        s->seen_encoder = true;
        s->first_encoder_ms = ms_since(s->t0);
    }
    if (flag_set(s->abort_flag)) {
        s->aborted = true;
        return false;
    }
    return true;
}

bool on_abort_poll(void *user) {
    auto *s = static_cast<full_state *>(user);
    if (flag_set(s->abort_flag)) {
        s->aborted = true;
        return true;
    }
    return false;
}

thread_local std::vector<int32_t> t_token_ids;

void on_logits(struct whisper_context *ctx, struct whisper_state *, const whisper_token_data *tokens,
               int n_tokens, float *logits, void *user) {
    auto *s = static_cast<full_state *>(user);
    try {
        t_token_ids.resize(n_tokens > 0 ? static_cast<size_t>(n_tokens) : 0);
        for (int i = 0; i < n_tokens; i++) {
            t_token_ids[i] = tokens[i].id;
        }
        s->cb(t_token_ids.data(), n_tokens, logits, whisper_n_vocab(ctx), s->cb_user);
    } catch (...) {
        // A bonus that cannot be applied leaves the logits as they are.
    }
}

}  // namespace

extern "C" {

struct whisper_context *br_init(const br_reader *model, bool use_gpu, bool flash_attn) {
    if (model == nullptr) {
        return nullptr;
    }
    reader_guard guard{model, false};
    struct whisper_context *ctx = nullptr;
    try {
        whisper_context_params cp = whisper_context_default_params();
        cp.use_gpu = use_gpu;
        cp.flash_attn = flash_attn;
        cp.gpu_device = 0;
        cp.dtw_token_timestamps = false;
        whisper_model_loader loader = loader_for(guard);
        ctx = whisper_init_with_params(&loader, cp);
    } catch (...) {
        ctx = nullptr;
    }
    guard.close();
    return ctx;
}

void br_free(struct whisper_context *ctx) {
    if (ctx == nullptr) {
        return;
    }
    try {
        whisper_free(ctx);
    } catch (...) {
    }
}

int32_t br_gpu_device(char *buf, int32_t cap) {
    try {
        for (size_t i = 0; i < ggml_backend_dev_count(); i++) {
            ggml_backend_dev_t dev = ggml_backend_dev_get(i);
            enum ggml_backend_dev_type type = ggml_backend_dev_type(dev);
            if (type != GGML_BACKEND_DEVICE_TYPE_GPU && type != GGML_BACKEND_DEVICE_TYPE_IGPU) {
                continue;
            }
            const char *desc = ggml_backend_dev_description(dev);
            if (desc == nullptr || desc[0] == '\0') {
                desc = ggml_backend_dev_name(dev);
            }
            if (desc == nullptr || desc[0] == '\0') {
                desc = "GPU";
            }
            const size_t len = std::strlen(desc);
            if (buf != nullptr && cap > 0) {
                const size_t n = std::min(len, static_cast<size_t>(cap - 1));
                std::memcpy(buf, desc, n);
                buf[n] = '\0';
                return static_cast<int32_t>(n);
            }
            return static_cast<int32_t>(len);
        }
    } catch (...) {
    }
    return 0;
}

int32_t br_n_vocab(struct whisper_context *ctx) {
    if (ctx == nullptr) {
        return 0;
    }
    try {
        return whisper_n_vocab(ctx);
    } catch (...) {
        return 0;
    }
}

bool br_is_multilingual(struct whisper_context *ctx) {
    if (ctx == nullptr) {
        return false;
    }
    try {
        return whisper_is_multilingual(ctx) != 0;
    } catch (...) {
        return false;
    }
}

int32_t br_token_eot(struct whisper_context *ctx) {
    if (ctx == nullptr) {
        return -1;
    }
    try {
        return whisper_token_eot(ctx);
    } catch (...) {
        return -1;
    }
}

int32_t br_tokenize(struct whisper_context *ctx, const char *text, int32_t *tokens, int32_t cap) {
    if (ctx == nullptr || text == nullptr || tokens == nullptr || cap <= 0) {
        return -1;
    }
    try {
        const int n = whisper_tokenize(ctx, text, tokens, cap);
        return n < 0 ? -1 : n;
    } catch (...) {
        return -1;
    }
}

int32_t br_token_bytes(struct whisper_context *ctx, int32_t id, char *buf, int32_t cap) {
    if (ctx == nullptr || id < 0 || id >= whisper_n_vocab(ctx)) {
        return -1;
    }
    try {
        const char *s = whisper_token_to_str(ctx, id);
        if (s == nullptr) {
            return -1;
        }
        const size_t len = std::strlen(s);
        if (buf != nullptr && cap > 0) {
            std::memcpy(buf, s, std::min(len, static_cast<size_t>(cap)));
        }
        return static_cast<int32_t>(len);
    } catch (...) {
        return -1;
    }
}

int32_t br_full(struct whisper_context *ctx, const br_decode *d, const br_vad *v, const float *pcm,
                int32_t n_samples, const int32_t *prompt, int32_t n_prompt, br_logits_cb cb,
                void *cb_user, const volatile int32_t *abort_flag, br_timings *out) {
    if (out != nullptr) {
        out->vad_ms = 0.0f;
        out->encode_ms = 0.0f;
        out->decode_ms = 0.0f;
    }
    if (ctx == nullptr || d == nullptr || v == nullptr || d->language == nullptr || n_samples < 0 ||
        (pcm == nullptr && n_samples > 0) || n_prompt < 0 || (prompt == nullptr && n_prompt > 0) ||
        (v->enabled && v->model_file == nullptr)) {
        return BR_INVALID;
    }
    if (n_samples > BR_MAX_SAMPLES) {
        return BR_TOO_LONG;
    }
    try {
        whisper_full_params p = full_params_from(d, v);
        if (n_prompt > 0) {
            p.prompt_tokens = prompt;
            p.prompt_n_tokens = n_prompt;
        }

        full_state s{abort_flag, clock_type::now(), false, 0.0, cb, cb_user, false};
        p.encoder_begin_callback = on_encoder_begin;
        p.encoder_begin_callback_user_data = &s;
        p.abort_callback = on_abort_poll;
        p.abort_callback_user_data = &s;
        if (cb != nullptr) {
            p.logits_filter_callback = on_logits;
            p.logits_filter_callback_user_data = &s;
        }

        whisper_reset_timings(ctx);
        s.t0 = clock_type::now();
        const int rc = whisper_full(ctx, p, pcm, n_samples);
        const double wall = ms_since(s.t0);

        if (out != nullptr) {
            float encode = 0.0f;
            if (s.seen_encoder) {
                whisper_timings *t = whisper_get_timings(ctx);
                if (t != nullptr) {
                    encode = t->encode_ms;
                    // Upstream allocates it with new and has no free function.
                    delete t;
                }
            }
            const double vad = s.seen_encoder ? s.first_encoder_ms : wall;
            out->vad_ms = static_cast<float>(vad);
            out->encode_ms = encode;
            out->decode_ms = static_cast<float>(std::max(0.0, wall - vad - encode));
        }

        // A set flag wins whatever whisper.cpp returned: a stop before the first encoder
        // pass comes back as 0 with no segments, and a cancelled job must never read as
        // a finished transcription of silence.
        if (s.aborted || flag_set(abort_flag)) {
            return BR_ABORTED;
        }
        if (rc != 0) {
            return rc;
        }
        return BR_OK;
    } catch (...) {
        return BR_EXCEPTION;
    }
}

int32_t br_n_segments(struct whisper_context *ctx) {
    if (ctx == nullptr) {
        return 0;
    }
    try {
        return whisper_full_n_segments(ctx);
    } catch (...) {
        return 0;
    }
}

const char *br_segment_text(struct whisper_context *ctx, int32_t i) {
    if (ctx == nullptr || i < 0) {
        return "";
    }
    try {
        if (i >= whisper_full_n_segments(ctx)) {
            return "";
        }
        const char *text = whisper_full_get_segment_text(ctx, i);
        return text != nullptr ? text : "";
    } catch (...) {
        return "";
    }
}

void br_segment_span(struct whisper_context *ctx, int32_t i, int64_t *t0_ms, int64_t *t1_ms) {
    int64_t t0 = 0;
    int64_t t1 = 0;
    try {
        if (ctx != nullptr && i >= 0 && i < whisper_full_n_segments(ctx)) {
            // Centiseconds, already mapped back to the original timeline when the
            // detector ran.
            t0 = whisper_full_get_segment_t0(ctx, i) * 10;
            t1 = whisper_full_get_segment_t1(ctx, i) * 10;
        }
    } catch (...) {
    }
    if (t0_ms != nullptr) {
        *t0_ms = t0;
    }
    if (t1_ms != nullptr) {
        *t1_ms = t1;
    }
}

int32_t br_warmup(struct whisper_context *ctx, const br_vad *v, int32_t n_threads, bool encoder) {
    if (ctx == nullptr || v == nullptr || (v->enabled && v->model_file == nullptr)) {
        return BR_INVALID;
    }
    try {
        std::vector<float> silence(16000, 0.0f);
        br_decode d = {};
        d.language = "en";
        d.n_threads = n_threads;
        d.best_of = 1;
        d.temperature = 0.0f;
        d.temperature_inc = 0.0f;
        d.entropy_thold = 2.4f;
        d.logprob_thold = -1.0f;
        d.no_speech_thold = 0.6f;
        d.no_context = true;
        d.suppress_blank = true;

        if (v->enabled) {
            // The integrated detector is opened lazily, per state, at the first
            // whisper_full with vad on; half a second of silence opens it and returns
            // before any encoder pass, because the detector finds no speech.
            whisper_full_params p = full_params_from(&d, v);
            if (whisper_full(ctx, p, silence.data(), 8000) != 0) {
                return BR_DETECTOR_FAILED;
            }
        }
        if (encoder) {
            // One encoder pass and one decoder step, so Metal builds its pipelines and the
            // compute buffers now rather than in the first real request.
            br_vad off = *v;
            off.enabled = false;
            d.single_segment = true;
            d.max_tokens = 1;
            whisper_full_params p = full_params_from(&d, &off);
            const int rc = whisper_full(ctx, p, silence.data(), static_cast<int>(silence.size()));
            if (rc != 0) {
                return rc;
            }
        }
        return BR_OK;
    } catch (...) {
        return BR_EXCEPTION;
    }
}

void br_params_snapshot(const br_decode *d, const br_vad *v, br_params_view *out) {
    if (d == nullptr || v == nullptr || out == nullptr) {
        return;
    }
    try {
        const whisper_full_params p = full_params_from(d, v);
        out->strategy = static_cast<int32_t>(p.strategy);
        out->best_of = p.greedy.best_of;
        out->no_context = p.no_context;
        out->no_timestamps = p.no_timestamps;
        out->single_segment = p.single_segment;
        out->print_special = p.print_special;
        out->print_progress = p.print_progress;
        out->print_realtime = p.print_realtime;
        out->print_timestamps = p.print_timestamps;
        out->vad = p.vad;
        out->temperature_inc = p.temperature_inc;
        out->suppress_nst = p.suppress_nst;
        out->n_threads = p.n_threads;
    } catch (...) {
    }
}

void *br_vad_init(const br_reader *model, int32_t n_threads) {
    if (model == nullptr) {
        return nullptr;
    }
    reader_guard guard{model, false};
    struct whisper_vad_context *vctx = nullptr;
    try {
        whisper_vad_context_params vp = whisper_vad_default_context_params();
        vp.n_threads = n_threads > 0 ? n_threads : 1;
        // The detector stays on the CPU: it is small, and on the GPU it would queue
        // behind the decoder (at the pin a Metal detector also aborts in the scheduler).
        vp.use_gpu = false;
        whisper_model_loader loader = loader_for(guard);
        vctx = whisper_vad_init_with_params(&loader, vp);
    } catch (...) {
        vctx = nullptr;
    }
    guard.close();
    return vctx;
}

int32_t br_vad_spans(void *vad, const float *pcm, int32_t n_samples, const br_vad *params,
                     int64_t *spans_ms, int32_t cap_pairs) {
    if (vad == nullptr || params == nullptr || n_samples < 0 || (pcm == nullptr && n_samples > 0) ||
        cap_pairs < 0 || (spans_ms == nullptr && cap_pairs > 0)) {
        return BR_INVALID;
    }
    if (n_samples == 0) {
        return 0;
    }
    try {
        whisper_vad_segments *seg = whisper_vad_segments_from_samples(
            static_cast<whisper_vad_context *>(vad), vad_params_from(params), pcm, n_samples);
        if (seg == nullptr) {
            return -1;
        }
        const int n = whisper_vad_segments_n_segments(seg);
        for (int i = 0; i < n && i < cap_pairs; i++) {
            // Centiseconds as floats.
            spans_ms[2 * i] = static_cast<int64_t>(whisper_vad_segments_get_segment_t0(seg, i) * 10.0f + 0.5f);
            spans_ms[2 * i + 1] = static_cast<int64_t>(whisper_vad_segments_get_segment_t1(seg, i) * 10.0f + 0.5f);
        }
        whisper_vad_free_segments(seg);
        return n;
    } catch (...) {
        return BR_EXCEPTION;
    }
}

void br_vad_free(void *vad) {
    if (vad == nullptr) {
        return;
    }
    try {
        whisper_vad_free(static_cast<whisper_vad_context *>(vad));
    } catch (...) {
    }
}

void br_log_install(void) {
    // whisper_log_set also installs the callback into ggml; ggml_log_set says so plainly,
    // and whisper_backend_init_gpu re-installs whisper's callback, which is then this one.
    whisper_log_set(log_callback, nullptr);
    ggml_log_set(log_callback, nullptr);
}

bool br_log_should_forward(int32_t level, const char *text) {
    if (text == nullptr) {
        return false;
    }
    if (level != GGML_LOG_LEVEL_WARN && level != GGML_LOG_LEVEL_ERROR) {
        return false;
    }
    // Any line over 200 bytes is dropped, whatever its level: a long line is the kind
    // that could carry a path, a prompt or text (DD-13, P-12).
    for (size_t i = 0; i <= 200; i++) {
        if (text[i] == '\0') {
            return true;
        }
    }
    return false;
}

const char *br_system_info(void) {
    try {
        return whisper_print_system_info();
    } catch (...) {
        return "";
    }
}

const char *br_whisper_version(void) {
    return whisper_version();
}

size_t br_sizeof(int32_t which) {
    switch (which) {
        case 0:
            return sizeof(br_reader);
        case 1:
            return sizeof(br_decode);
        case 2:
            return sizeof(br_vad);
        case 3:
            return sizeof(br_timings);
        case 4:
            return sizeof(br_params_view);
        default:
            return 0;
    }
}

}  // extern "C"
