#define _GNU_SOURCE

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static ssize_t (*real_write)(int, const void *, size_t);
static int (*real_fsync)(int);
static int faulted;

static void resolve_symbols(void) {
    if (!real_write) {
        real_write = dlsym(RTLD_NEXT, "write");
    }
    if (!real_fsync) {
        real_fsync = dlsym(RTLD_NEXT, "fsync");
    }
}

static int target_fd(int fd) {
    const char *suffix = getenv("CREW_FAULT_SUFFIX");
    if (!suffix) {
        return 0;
    }
    char link_path[64];
    char target[PATH_MAX];
    int link_len = snprintf(link_path, sizeof(link_path), "/proc/self/fd/%d", fd);
    if (link_len <= 0 || (size_t)link_len >= sizeof(link_path)) {
        return 0;
    }
    ssize_t target_len = readlink(link_path, target, sizeof(target) - 1);
    if (target_len <= 0) {
        return 0;
    }
    target[target_len] = '\0';
    size_t suffix_len = strlen(suffix);
    return (size_t)target_len >= suffix_len
        && strcmp(target + target_len - suffix_len, suffix) == 0;
}

static int armed(void) {
    const char *marker = getenv("CREW_FAULT_MARKER");
    return marker && access(marker, F_OK) == 0;
}

static void record_hit(void) {
    const char *path = getenv("CREW_FAULT_HIT_FILE");
    if (!path) {
        return;
    }
    int fd = open(path, O_CREAT | O_WRONLY | O_APPEND | O_CLOEXEC, 0600);
    if (fd >= 0) {
        static const char line[] = "hit\n";
        real_write(fd, line, sizeof(line) - 1);
        close(fd);
    }
}

static int should_fail(int fd, const char *call) {
    const char *mode = getenv("CREW_FAULT_CALL");
    if (faulted || !mode || strcmp(mode, call) != 0 || !armed() || !target_fd(fd)) {
        return 0;
    }
    faulted = 1;
    record_hit();
    const char *errno_name = getenv("CREW_FAULT_ERRNO");
    errno = errno_name && strcmp(errno_name, "ENOSPC") == 0 ? ENOSPC : EIO;
    return 1;
}

ssize_t write(int fd, const void *buf, size_t count) {
    resolve_symbols();
    if (should_fail(fd, "write")) {
        return -1;
    }
    return real_write(fd, buf, count);
}

int fsync(int fd) {
    resolve_symbols();
    if (should_fail(fd, "fsync")) {
        return -1;
    }
    return real_fsync(fd);
}
