/* A stand-in for librns: keeps the event counter in the area directory and logs every call there,
 * so tests can drive the backend without the kernel module. The structures follow rns_api.h. */
#include <errno.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

struct rns_api_create_options {
    int main_pid;
    const char *const *exclude_paths;
    size_t exclude_count;
    int workspace_fd;
    bool use_workspace_fd;
    const char *area_name;
};

struct rns_api_restore_target {
    unsigned action;
    uint64_t request_id[2];
    unsigned kind;
    uint32_t turns_back;
    uint64_t target_event_id;
};

struct rns_api_restore_result {
    uint64_t valid_fields;
    uint32_t phase;
    int operation_errno;
    uint64_t event_id_out, unrestored_count, overlap_count;
    void *tasks;
    size_t task_capacity;
    uint64_t task_total_count;
    size_t task_written_count;
    bool task_truncated;
};

enum { RESTORE_START = 1, RESTORE_QUERY = 2, RESTORE_CONTINUE = 3 };
enum { RESTORE_REPLAY = 2, RESTORE_DONE = 4 };
enum { VALID_EVENT = 1 << 0, VALID_PHASE = 1 << 1, VALID_ERROR = 1 << 5 };

static int area_file(char *out, size_t size, const char *workspace, const char *name)
{
    int written = snprintf(out, size, "%s/%s", workspace, name);
    if (written < 0 || (size_t)written >= size) {
        errno = ENAMETOOLONG;
        return -1;
    }
    return 0;
}

static bool marker_exists(const char *workspace, const char *name)
{
    char path[4096];
    return area_file(path, sizeof path, workspace, name) == 0 && access(path, F_OK) == 0;
}

static int note(const char *workspace, const char *format, ...)
{
    char path[4096];
    if (area_file(path, sizeof path, workspace, "calls") < 0)
        return -1;
    FILE *file = fopen(path, "a");
    if (!file)
        return -1;
    va_list args;
    va_start(args, format);
    vfprintf(file, format, args);
    va_end(args);
    fputc('\n', file);
    return fclose(file);
}

static int load_event(const char *workspace, uint64_t *event)
{
    char path[4096];
    unsigned long long value = 0;
    if (area_file(path, sizeof path, workspace, "event") < 0)
        return -1;
    FILE *file = fopen(path, "r");
    if (!file) {
        *event = 0;
        return errno == ENOENT ? 0 : -1;
    }
    int matched = fscanf(file, "%llu", &value);
    fclose(file);
    if (matched != 1) {
        errno = EIO;
        return -1;
    }
    *event = value;
    return 0;
}

static int store_event(const char *workspace, uint64_t event)
{
    char path[4096];
    if (area_file(path, sizeof path, workspace, "event") < 0)
        return -1;
    FILE *file = fopen(path, "w");
    if (!file)
        return -1;
    fprintf(file, "%llu\n", (unsigned long long)event);
    return fclose(file);
}

int rns_create(const char *workspace, const char *session,
               const struct rns_api_create_options *options)
{
    char excludes[8192] = "";
    for (size_t index = 0; index < options->exclude_count; index++) {
        strncat(excludes, options->exclude_paths[index], sizeof excludes - strlen(excludes) - 2);
        strcat(excludes, ",");
    }
    return note(workspace, "create %s %d %s", session, options->main_pid, excludes);
}

int rns_capabilities(uint64_t *capabilities_out)
{
    *capabilities_out = 3;
    return 0;
}

int rns_create_with_flags(const char *workspace, const char *session,
                          const struct rns_api_create_options *options, unsigned flags)
{
    (void)flags;
    return rns_create(workspace, session, options);
}

int rns_attach(const char *workspace, const char *session, int target_tid, unsigned task_scope,
               uint64_t event_id)
{
    return note(workspace, "attach %s %d %u %llu", session, target_tid, task_scope,
                (unsigned long long)event_id);
}

int rns_ckpt(const char *workspace, const char *session, const char *label,
             uint64_t *event_id_out)
{
    if (marker_exists(workspace, "pending-restore")) {
        errno = EBUSY;
        return -1;
    }
    uint64_t event;
    if (load_event(workspace, &event) < 0 || store_event(workspace, event + 1) < 0)
        return -1;
    *event_id_out = event + 1;
    return note(workspace, "ckpt %s %s", session, label);
}

struct saved_restore {
    uint64_t event;
    unsigned phase;
    int error;
    unsigned flags;
};

static int request_path(char *out, size_t size, const char *workspace, const uint64_t id[2])
{
    char name[64];
    snprintf(name, sizeof name, "restore-%016llx-%016llx",
             (unsigned long long)id[0], (unsigned long long)id[1]);
    return area_file(out, size, workspace, name);
}

/* 1 = an existing request, 0 = no request, -1 = a fixture storage error. */
static int load_restore(const char *path, struct saved_restore *saved)
{
    FILE *file = fopen(path, "r");
    if (!file)
        return errno == ENOENT ? 0 : -1;
    unsigned long long event;
    int fields = fscanf(file, "%llu %u %d %u", &event, &saved->phase, &saved->error, &saved->flags);
    fclose(file);
    if (fields != 4) {
        errno = EIO;
        return -1;
    }
    saved->event = event;
    return 1;
}

static int store_restore(const char *path, const struct saved_restore *saved)
{
    FILE *file = fopen(path, "w");
    if (!file)
        return -1;
    fprintf(file, "%llu %u %d %u\n", (unsigned long long)saved->event,
            saved->phase, saved->error, saved->flags);
    return fclose(file);
}

/* Reject before request registration; neither a receipt nor a valid phase is returned. */
static int reject_restore(const char *workspace)
{
    char path[4096], reason[32];
    if (area_file(path, sizeof path, workspace, "reject-restore") < 0)
        return -1;
    FILE *file = fopen(path, "r");
    if (!file)
        return errno == ENOENT ? 0 : -1;
    int fields = fscanf(file, "%31s", reason);
    fclose(file);
    if (fields == 1 && strcmp(reason, "busy") == 0)
        errno = EBUSY;
    else if (fields == 1 && strcmp(reason, "fault") == 0)
        errno = EFAULT;
    else
        errno = ENOENT;
    return -1;
}

static int restore_result(const char *workspace, const struct saved_restore *saved,
                          unsigned action, struct rns_api_restore_result *result)
{
    if (marker_exists(workspace, "lose-restore-result")) {
        errno = EFAULT;
        return -1;
    }
    result->valid_fields = VALID_EVENT | VALID_PHASE | VALID_ERROR;
    result->phase = saved->phase;
    result->operation_errno = saved->error;
    result->event_id_out = saved->phase == RESTORE_DONE ? saved->event : 0;
    if (action != RESTORE_QUERY && saved->error) {
        errno = saved->error;
        return -1;
    }
    return 0;
}

static int mark_pending(const char *workspace, bool pending)
{
    char path[4096];
    if (area_file(path, sizeof path, workspace, "pending-restore") < 0)
        return -1;
    if (!pending)
        return unlink(path) == 0 || errno == ENOENT ? 0 : -1;
    FILE *file = fopen(path, "w");
    return file ? fclose(file) : -1;
}

int rns_restore(const char *workspace, const char *session,
                const struct rns_api_restore_target *target, unsigned flags,
                struct rns_api_restore_result *result)
{
    memset(result, 0, sizeof *result);
    char path[4096];
    struct saved_restore saved;
    if (request_path(path, sizeof path, workspace, target->request_id) < 0)
        return -1;
    int found = load_restore(path, &saved);
    if (found < 0)
        return -1;
    if (found) {
        if (saved.flags != flags ||
            (target->action == RESTORE_START && saved.event != target->target_event_id)) {
            errno = EINVAL;
            return -1;
        }
        /* START is idempotent and reads the retained result; it never continues the plan. */
        if (target->action != RESTORE_CONTINUE || saved.phase == RESTORE_DONE) {
            const char *action = target->action == RESTORE_QUERY ? "query" : "start-result";
            if (note(workspace, "%s %s", action, session) < 0)
                return -1;
            return restore_result(workspace, &saved, target->action, result);
        }
    } else {
        if (target->action != RESTORE_START) {
            note(workspace, "unknown-request %s", session);
            errno = ENOENT;
            return -1;
        }
        if (reject_restore(workspace) < 0)
            return -1;
        if (marker_exists(workspace, "pending-restore")) {
            errno = EBUSY;
            return -1;
        }
        uint64_t current;
        if (load_event(workspace, &current) < 0)
            return -1;
        if (target->target_event_id > current) {
            errno = ENOENT;
            return -1;
        }
        saved = (struct saved_restore) { .event = target->target_event_id, .flags = flags };
    }

    if (target->action == RESTORE_CONTINUE) {
        if (note(workspace, "continue %s", session) < 0)
            return -1;
    } else if (note(workspace, "restore %s %llu", session,
                    (unsigned long long)saved.event) < 0) {
        return -1;
    }
    bool failed = marker_exists(workspace, "fail-restore");
    saved.phase = failed ? RESTORE_REPLAY : RESTORE_DONE;
    saved.error = failed ? EIO : 0;
    if (store_restore(path, &saved) < 0 || mark_pending(workspace, failed) < 0)
        return -1;
    /* Event allocation stays monotonic; rollback does not reuse old checkpoint identities. */
    return restore_result(workspace, &saved, target->action, result);
}

/* Leaves `<area>.del` next to the area directory, which the backend removes after this call. */
int rns_del(const char *workspace)
{
    char path[4096];
    int written = snprintf(path, sizeof path, "%s.del", workspace);
    if (written < 0 || (size_t)written >= sizeof path) {
        errno = ENAMETOOLONG;
        return -1;
    }
    FILE *file = fopen(path, "w");
    if (!file)
        return -1;
    return fclose(file);
}
