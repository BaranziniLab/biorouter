/*
 * The C interface between Biorouter's Rust code and whisper.cpp (DD-2).
 *
 * The shim owns `whisper_full_params`, which whisper.cpp passes by value, so Rust needs
 * no struct layout from whisper.h. `src/engine/ffi.rs` declares every function here by
 * hand (build rule 8). Every function is extern "C" and catches every C++ exception, so
 * none can unwind into Rust.
 */
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct whisper_context;

/* A model reader. whisper.cpp calls read until it has what it needs and eof after a
   short read. The shim calls close exactly once, whether loading succeeded or not. */
typedef struct br_reader {
    void *ctx;
    size_t (*read)(void *ctx, void *out, size_t n); /* bytes read; fewer than n only at the end */
    bool (*eof)(void *ctx);                         /* true once a read returned fewer than requested */
    void (*close)(void *ctx);                       /* frees ctx */
} br_reader;

/* DD-47. Every field is set by the Rust parameter builder; nothing here has a default. */
typedef struct br_decode {
    const char *language; /* "en" for a .en model; the configured code otherwise; never NULL */
    int32_t n_threads;
    int32_t best_of;         /* 5 */
    float temperature;       /* 0.0 */
    float temperature_inc;   /* 0.2 until M-22 */
    float entropy_thold;     /* 2.4 */
    float logprob_thold;     /* -1.0 */
    float no_speech_thold;   /* 0.6 */
    bool translate;          /* false */
    bool no_context;         /* true */
    bool no_timestamps;      /* false: timestamps on, stripped from the text */
    bool single_segment;     /* false */
    bool suppress_blank;     /* true */
    bool suppress_nst;       /* false */
    int32_t audio_ctx;       /* 0 = the full 30 s window */
    int32_t max_tokens;      /* 0 = no limit */
} br_decode;

/* DD-48, plus the two fields whisper_vad_params has at the pin. */
typedef struct br_vad {
    bool enabled;            /* the integrated detector inside whisper_full */
    const char *model_file;  /* bare ASCII file name, opened relative to the current directory */
    float threshold;         /* 0.50 */
    int32_t min_speech_ms;   /* 250 */
    int32_t min_silence_ms;  /* 100 */
    float max_speech_s;      /* upstream default FLT_MAX; the parent splits at 25 s */
    int32_t speech_pad_ms;   /* 30 */
    float samples_overlap_s; /* upstream default 0.1 */
} br_vad;

/* vad_ms: the detector plus the mel spectrogram, up to the first encoder pass.
   encode_ms: whisper's own figure, per encoder pass. decode_ms: the rest of the call. */
typedef struct br_timings {
    float vad_ms;
    float encode_ms;
    float decode_ms;
} br_timings;

/* PR-14 installs a decoder bonus through this; PR-09 and PR-10 always pass NULL. */
typedef void (*br_logits_cb)(const int32_t *token_ids, int32_t n_tokens, float *logits,
                             int32_t n_vocab, void *user);

/* What the parameter builder put into whisper_full_params, for the DD-13 tests. */
typedef struct br_params_view {
    int32_t strategy;
    int32_t best_of;
    bool no_context;
    bool no_timestamps;
    bool single_segment;
    bool print_special;
    bool print_progress;
    bool print_realtime;
    bool print_timestamps;
    bool vad;
    float temperature_inc;
    bool suppress_nst;
    int32_t n_threads;
} br_params_view;

#define BR_OK 0
#define BR_ABORTED (-1000)          /* the abort flag was set, whatever whisper.cpp returned */
#define BR_EXCEPTION (-1001)        /* a C++ exception was caught */
#define BR_TOO_LONG (-1002)         /* more than 25 s of audio (DD-47) */
#define BR_INVALID (-1003)          /* a required pointer was NULL or a count was negative */
#define BR_DETECTOR_FAILED (-1004)  /* the integrated detector could not be opened */

#define BR_MAX_SAMPLES (25 * 16000)

/* lifecycle */
struct whisper_context *br_init(const br_reader *model, bool use_gpu, bool flash_attn); /* NULL on failure */
void br_free(struct whisper_context *ctx);
int32_t br_gpu_device(char *buf, int32_t cap); /* length of the first GPU device's description; 0 when none */
int32_t br_n_vocab(struct whisper_context *ctx);
bool br_is_multilingual(struct whisper_context *ctx);
int32_t br_token_eot(struct whisper_context *ctx);
int32_t br_token_bytes(struct whisper_context *ctx, int32_t id, char *buf, int32_t cap); /* the token's length; -1 for an unknown id */

/* decoding: at most 25 s of samples per call */
int32_t br_full(struct whisper_context *ctx, const br_decode *d, const br_vad *v,
                const float *pcm, int32_t n_samples,
                const int32_t *prompt, int32_t n_prompt,
                br_logits_cb cb, void *cb_user,
                const volatile int32_t *abort_flag, br_timings *out);
int32_t br_n_segments(struct whisper_context *ctx);
const char *br_segment_text(struct whisper_context *ctx, int32_t i);
void br_segment_span(struct whisper_context *ctx, int32_t i, int64_t *t0_ms, int64_t *t1_ms);
/* Opens the integrated detector (when v->enabled) and, when encoder is true, runs one
   encoder pass and one decoder step on silence, so the first real request finds Metal's
   pipelines and the compute buffers ready. BR_OK, BR_DETECTOR_FAILED, or whisper's code. */
int32_t br_warmup(struct whisper_context *ctx, const br_vad *v, int32_t n_threads, bool encoder);
void br_params_snapshot(const br_decode *d, const br_vad *v, br_params_view *out);

/* the separate detector for vad_segments; always on the CPU. Pass 1 thread: without
   OpenMP, ggml starts and joins a thread pool for every 32 ms chunk of audio, so more
   threads only make Silero slower. */
void *br_vad_init(const br_reader *model, int32_t n_threads);
/* Writes up to cap_pairs (t0_ms, t1_ms) pairs and returns how many spans were found:
   more than cap_pairs means call again with a larger buffer; negative on failure. */
int32_t br_vad_spans(void *vad, const float *pcm, int32_t n_samples, const br_vad *params,
                     int64_t *spans_ms, int32_t cap_pairs);
void br_vad_free(void *vad);

/* logging and build facts */
void br_log_install(void);                                /* whisper_log_set and ggml_log_set, one filter */
bool br_log_should_forward(int32_t level, const char *text); /* the DD-13 decision, exposed for the test */
const char *br_system_info(void);
const char *br_whisper_version(void);
/* sizeof the shared structs, so a test can hold the Rust mirrors to the C layout:
   0 br_reader, 1 br_decode, 2 br_vad, 3 br_timings, 4 br_params_view; 0 otherwise */
size_t br_sizeof(int32_t which);

#ifdef __cplusplus
}
#endif
