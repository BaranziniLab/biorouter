/*
 * br_capture.c: the terminal's microphone, over the vendored miniaudio 0.11.24 (DD-73).
 *
 * A handful of functions, declared by hand in src/lib.rs: one audio context per child, its
 * capture devices listed without opening one, one capture stream of 16 kHz mono float
 * samples handed to a callback, and the device closed again. build.rs compiles only the
 * backends dictation uses (CoreAudio, WASAPI, PulseAudio, ALSA), so miniaudio's Null
 * backend, which records silence where no audio system exists, is not in the binary.
 *
 * Nothing here writes audio anywhere: samples go to the callback and nowhere else.
 */
#define MINIAUDIO_IMPLEMENTATION
#include "miniaudio.h"

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#if defined(__linux__)
#include <dlfcn.h>
#endif

typedef void (*br_capture_on_samples)(void *user, const float *samples, uint32_t frames);
typedef void (*br_capture_on_device)(void *user, const char *name, int is_default);

typedef struct br_capture {
    ma_context context;
    ma_device device;
    int device_open;
    ma_atomic_bool32 stopping;
    ma_atomic_bool32 lost;
    br_capture_on_samples on_samples;
    void *user;
} br_capture;

#if defined(__linux__)
/* ALSA prints its configuration errors on stderr (about 40 lines of "ALSA lib confmisc.c"
 * on a server without a sound card, measured in the build spike). The parent never shows
 * the child's stderr, and this handler keeps the lines out of it too. */
static void br_alsa_silent(const char *file, int line, const char *function, int err, const char *fmt, ...)
{
    (void)file; (void)line; (void)function; (void)err; (void)fmt;
}

typedef int (*br_snd_lib_error_set_handler_proc)(void (*)(const char *, int, const char *, int, const char *, ...));

static void br_silence_alsa(void)
{
    static const char *names[] = {"libasound.so.2", "libasound.so"};
    size_t i;
    for (i = 0; i < sizeof(names) / sizeof(names[0]); i += 1) {
        void *library = dlopen(names[i], RTLD_NOW | RTLD_GLOBAL);
        if (library != NULL) {
            br_snd_lib_error_set_handler_proc set =
                (br_snd_lib_error_set_handler_proc)dlsym(library, "snd_lib_error_set_handler");
            if (set != NULL) {
                set(br_alsa_silent);
            }
            /* The handle stays open: miniaudio opens the same library by name, and the
             * handler lives in it. */
            return;
        }
    }
}
#endif

/* The backends this build can use, comma separated; the tests read it. */
const char *br_capture_compiled_backends(void)
{
    static char names[256];
    ma_backend backends[ma_backend_null + 1];
    size_t count = 0;
    size_t i;
    names[0] = '\0';
    if (ma_get_enabled_backends(backends, sizeof(backends) / sizeof(backends[0]), &count) != MA_SUCCESS) {
        return names;
    }
    for (i = 0; i < count; i += 1) {
        if (i > 0) {
            strncat(names, ",", sizeof(names) - strlen(names) - 1);
        }
        strncat(names, ma_get_backend_name(backends[i]), sizeof(names) - strlen(names) - 1);
    }
    return names;
}

/* A new audio context, or NULL with *result set: MA_NO_BACKEND when no audio system
 * answers. A context whose backend is Null is refused the same way. */
br_capture *br_capture_new(int *result)
{
    ma_context_config config;
    ma_result r;
    br_capture *capture = (br_capture *)calloc(1, sizeof(br_capture));
    if (capture == NULL) {
        *result = MA_OUT_OF_MEMORY;
        return NULL;
    }
    ma_atomic_bool32_set(&capture->stopping, MA_FALSE);
    ma_atomic_bool32_set(&capture->lost, MA_FALSE);
#if defined(__linux__)
    br_silence_alsa();
#endif
    config = ma_context_config_init();
    config.pulse.pApplicationName = "Biorouter";
    r = ma_context_init(NULL, 0, &config, &capture->context);
    if (r != MA_SUCCESS) {
        free(capture);
        *result = r;
        return NULL;
    }
    if (capture->context.backend == ma_backend_null) {
        ma_context_uninit(&capture->context);
        free(capture);
        *result = MA_NO_BACKEND;
        return NULL;
    }
    *result = MA_SUCCESS;
    return capture;
}

const char *br_capture_backend_name(const br_capture *capture)
{
    return ma_get_backend_name(capture->context.backend);
}

/* ALSA's "null" PCM is listed as a capture device and delivers zeros. */
static int br_is_null_device(const br_capture *capture, const ma_device_info *info)
{
#if defined(MA_HAS_ALSA)
    if (capture->context.backend == ma_backend_alsa && strcmp(info->id.alsa, "null") == 0) {
        return 1;
    }
#else
    (void)capture;
    (void)info;
#endif
    return 0;
}

/* Lists the capture devices without opening any; returns a ma_result. */
int br_capture_devices(br_capture *capture, br_capture_on_device on_device, void *user)
{
    ma_device_info *playback = NULL;
    ma_device_info *devices = NULL;
    ma_uint32 playback_count = 0;
    ma_uint32 count = 0;
    ma_uint32 i;
    ma_result r = ma_context_get_devices(&capture->context, &playback, &playback_count, &devices, &count);
    if (r != MA_SUCCESS) {
        return r;
    }
    for (i = 0; i < count; i += 1) {
        if (br_is_null_device(capture, &devices[i])) {
            continue;
        }
        on_device(user, devices[i].name, devices[i].isDefault ? 1 : 0);
    }
    return MA_SUCCESS;
}

static void br_on_data(ma_device *device, void *output, const void *input, ma_uint32 frames)
{
    br_capture *capture = (br_capture *)device->pUserData;
    (void)output;
    if (capture != NULL && input != NULL && frames > 0 &&
        !ma_atomic_bool32_get(&capture->stopping)) {
        capture->on_samples(capture->user, (const float *)input, frames);
    }
}

static void br_on_notification(const ma_device_notification *notification)
{
    br_capture *capture = (br_capture *)notification->pDevice->pUserData;
    if (capture != NULL && notification->type == ma_device_notification_type_stopped &&
        !ma_atomic_bool32_get(&capture->stopping)) {
        ma_atomic_bool32_set(&capture->lost, MA_TRUE);
    }
}

/* Opens the device named `name` (the system default when NULL) for 16 kHz mono float
 * samples in periods of `period_ms`, and starts it. Returns a ma_result: MA_DOES_NOT_EXIST
 * when no device has that name. */
int br_capture_start(br_capture *capture, const char *name, uint32_t rate, uint32_t period_ms,
                     br_capture_on_samples on_samples, void *user)
{
    ma_device_config config;
    ma_device_id chosen;
    ma_result r;
    if (capture->device_open) {
        return MA_INVALID_OPERATION;
    }
    config = ma_device_config_init(ma_device_type_capture);
    config.capture.format = ma_format_f32;
    config.capture.channels = 1;
    config.capture.shareMode = ma_share_mode_shared;
    config.sampleRate = rate;
    config.periodSizeInMilliseconds = period_ms;
    config.dataCallback = br_on_data;
    config.notificationCallback = br_on_notification;
    config.pUserData = capture;
    if (name != NULL) {
        ma_device_info *playback = NULL;
        ma_device_info *devices = NULL;
        ma_uint32 playback_count = 0;
        ma_uint32 count = 0;
        ma_uint32 i;
        int found = 0;
        r = ma_context_get_devices(&capture->context, &playback, &playback_count, &devices, &count);
        if (r != MA_SUCCESS) {
            return r;
        }
        for (i = 0; i < count; i += 1) {
            if (!br_is_null_device(capture, &devices[i]) && strcmp(devices[i].name, name) == 0) {
                chosen = devices[i].id;
                found = 1;
                break;
            }
        }
        if (!found) {
            return MA_DOES_NOT_EXIST;
        }
        config.capture.pDeviceID = &chosen;
    }
    capture->on_samples = on_samples;
    capture->user = user;
    ma_atomic_bool32_set(&capture->stopping, MA_FALSE);
    ma_atomic_bool32_set(&capture->lost, MA_FALSE);
    r = ma_device_init(&capture->context, &config, &capture->device);
    if (r != MA_SUCCESS) {
        return r;
    }
    capture->device_open = 1;
    r = ma_device_start(&capture->device);
    if (r != MA_SUCCESS) {
        ma_atomic_bool32_set(&capture->stopping, MA_TRUE);
        ma_device_uninit(&capture->device);
        capture->device_open = 0;
        return r;
    }
    return MA_SUCCESS;
}

const char *br_capture_device_name(const br_capture *capture)
{
    return capture->device_open ? capture->device.capture.name : "";
}

uint32_t br_capture_device_rate(const br_capture *capture)
{
    return capture->device_open ? capture->device.capture.internalSampleRate : 0;
}

/* 1 when the device stopped without being asked to: unplugged, or taken by the system. */
int br_capture_lost(const br_capture *capture)
{
    /* miniaudio's atomic get takes a mutable pointer, though it only loads. */
    return ma_atomic_bool32_get((ma_atomic_bool32 *)&capture->lost);
}

/* Stops the stream and closes the device. No callback runs once this returns. */
void br_capture_stop(br_capture *capture)
{
    if (capture->device_open) {
        ma_atomic_bool32_set(&capture->stopping, MA_TRUE);
        ma_device_uninit(&capture->device);
        capture->device_open = 0;
    }
}

void br_capture_free(br_capture *capture)
{
    if (capture == NULL) {
        return;
    }
    br_capture_stop(capture);
    ma_context_uninit(&capture->context);
    free(capture);
}
