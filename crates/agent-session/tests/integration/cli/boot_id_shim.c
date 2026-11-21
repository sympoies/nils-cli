/* Fixture-only LD_PRELOAD shim: redirect the boot-id read in the spawned test
 * daemon. No system files or other processes are changed. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>

static const char *fixture_path(const char *path) {
    const char *replacement = getenv("AGENT_SESSION_FIXTURE_BOOT_ID_FILE");
    return replacement && strcmp(path, "/proc/sys/kernel/random/boot_id") == 0
        ? replacement : path;
}

#define WRAP_OPEN(name) \
int name(const char *path, int flags, ...) { \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || ((flags & O_TMPFILE) == O_TMPFILE)) { va_list ap; va_start(ap, flags); mode = va_arg(ap, int); va_end(ap); } \
    int (*real_open)(const char *, int, ...) = dlsym(RTLD_NEXT, #name); \
    return real_open(fixture_path(path), flags, mode); \
}
#define WRAP_OPENAT(name) \
int name(int dirfd, const char *path, int flags, ...) { \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || ((flags & O_TMPFILE) == O_TMPFILE)) { va_list ap; va_start(ap, flags); mode = va_arg(ap, int); va_end(ap); } \
    int (*real_openat)(int, const char *, int, ...) = dlsym(RTLD_NEXT, #name); \
    return real_openat(dirfd, fixture_path(path), flags, mode); \
}
WRAP_OPEN(open)
WRAP_OPEN(open64)
WRAP_OPENAT(openat)
WRAP_OPENAT(openat64)
