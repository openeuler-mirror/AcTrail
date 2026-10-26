#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

struct workload {
    const char *kind;
    const char *path;
    const char *iterations_arg;
    uint64_t operations;
    uint64_t block_bytes;
    uint64_t task_iterations;
    uint64_t bytes;
    uint64_t checksum;
    uint64_t io_calls;
};

static volatile uint64_t task_sink;

static int usage(void) {
    fputs("usage: workload fork|exec|read|write|stdio --operations N "
          "[--task-iterations N] [--block-bytes N --path FILE]\n", stderr);
    return 2;
}

static int parse_positive(const char *text, uint64_t *value) {
    char *end;
    if (*text < '0' || *text > '9')
        return -1;
    errno = 0;
    unsigned long long parsed = strtoull(text, &end, 10);
    if (errno || *end || parsed == 0)
        return -1;
    *value = (uint64_t)parsed;
    return 0;
}

static int parse_args(struct workload *work, int argc, char **argv) {
    if (argc < 2)
        return -1;
    work->kind = argv[1];
    for (int i = 2; i < argc; i += 2) {
        if (i + 1 == argc)
            return -1;
        if (strcmp(argv[i], "--path") == 0) {
            if (work->path || !*argv[i + 1])
                return -1;
            work->path = argv[i + 1];
            continue;
        }
        uint64_t *field;
        if (strcmp(argv[i], "--operations") == 0)
            field = &work->operations;
        else if (strcmp(argv[i], "--block-bytes") == 0)
            field = &work->block_bytes;
        else if (strcmp(argv[i], "--task-iterations") == 0) {
            field = &work->task_iterations;
            work->iterations_arg = argv[i + 1];
        } else
            return -1;
        if (*field || parse_positive(argv[i + 1], field) != 0)
            return -1;
    }
    if (strcmp(work->kind, "child") == 0)
        return work->task_iterations && !work->operations && !work->path &&
                       !work->block_bytes ? 0 : -1;
    if (!work->operations)
        return -1;
    if (strcmp(work->kind, "fork") == 0 || strcmp(work->kind, "stdio") == 0)
        return !work->path && !work->block_bytes && !work->task_iterations ? 0 : -1;
    if (strcmp(work->kind, "exec") == 0)
        return work->task_iterations && !work->path && !work->block_bytes ? 0 : -1;
    if (strcmp(work->kind, "read") != 0 && strcmp(work->kind, "write") != 0)
        return -1;
    if (!work->path || !work->block_bytes || work->task_iterations ||
        work->block_bytes > SIZE_MAX || work->block_bytes > SSIZE_MAX ||
        work->operations > INT64_MAX / work->block_bytes)
        return -1;
    return 0;
}

static uint64_t run_task(uint64_t iterations) {
    uint64_t state = UINT64_C(0x9e3779b97f4a7c15);
    for (uint64_t i = 0; i < iterations; ++i) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
    }
    task_sink = state;
    return state;
}

static int run_processes(struct workload *work) {
    int with_exec = strcmp(work->kind, "exec") == 0;
    char executable[PATH_MAX];
    if (with_exec) {
        ssize_t size = readlink("/proc/self/exe", executable, sizeof(executable) - 1);
        if (size < 0 || (size_t)size == sizeof(executable) - 1) {
            fputs("cannot resolve workload executable\n", stderr);
            return 1;
        }
        executable[size] = '\0';
    }
    for (uint64_t i = 0; i < work->operations; ++i) {
        pid_t child = fork();
        if (child < 0) {
            perror("fork");
            return 1;
        }
        if (child == 0) {
            if (with_exec) {
                char *args[] = {executable, "child", "--task-iterations",
                                (char *)work->iterations_arg, NULL};
                execv(executable, args);
                perror("execv");
                _exit(127);
            }
            _exit(0);
        }
        if (with_exec)
            work->checksum += run_task(work->task_iterations);
        int status;
        pid_t waited;
        do {
            waited = waitpid(child, &status, 0);
        } while (waited < 0 && errno == EINTR);
        if (waited < 0) {
            perror("waitpid");
            return 1;
        }
        if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
            fprintf(stderr, "child failed: wait status=%d\n", status);
            return 1;
        }
    }
    return 0;
}

static int run_io(struct workload *work) {
    int writing = strcmp(work->kind, "write") == 0;
    int fd = open(work->path, writing ? O_WRONLY : O_RDONLY);
    if (fd < 0) {
        perror("open");
        return 1;
    }
    unsigned char *buffer = malloc((size_t)work->block_bytes);
    if (!buffer) {
        perror("malloc");
        close(fd);
        return 1;
    }
    memset(buffer, 0x5a, (size_t)work->block_bytes);
    int result = 0;
    for (uint64_t i = 0; i < work->operations; ++i) {
        size_t offset = 0;
        while (offset < work->block_bytes) {
            size_t remaining = (size_t)work->block_bytes - offset;
            ++work->io_calls;
            ssize_t count = writing ? write(fd, buffer + offset, remaining)
                                    : read(fd, buffer + offset, remaining);
            if (count < 0 && errno == EINTR)
                continue;
            if (count <= 0) {
                if (count < 0)
                    perror(writing ? "write" : "read");
                else
                    fputs("I/O ended before requested bytes completed\n", stderr);
                result = 1;
                goto finish;
            }
            offset += (size_t)count;
        }
        work->bytes += work->block_bytes;
        /* Sample two bytes per block; avoid making checksum CPU dominate I/O. */
        work->checksum += buffer[0] + buffer[work->block_bytes - 1];
    }
finish:
    free(buffer);
    if (close(fd) != 0) {
        perror("close");
        result = 1;
    }
    return result;
}

static int run_stdio(struct workload *work) {
    const char data[] = "ZZZZZZZZZ\n";
    for (uint64_t i = 0; i < work->operations; ++i) {
        size_t offset = 0;
        while (offset < sizeof(data) - 1) {
            ++work->io_calls;
            ssize_t count = write(STDOUT_FILENO, data + offset, sizeof(data) - 1 - offset);
            if (count < 0 && errno == EINTR)
                continue;
            if (count <= 0) {
                perror("stdio write");
                return 1;
            }
            offset += (size_t)count;
            work->bytes += (uint64_t)count;
        }
    }
    return 0;
}

int main(int argc, char **argv) {
    struct workload work = {0};
    if (parse_args(&work, argc, argv) != 0)
        return usage();
    if (strcmp(work.kind, "child") == 0) {
        run_task(work.task_iterations);
        return 0;
    }
    int result = strcmp(work.kind, "fork") == 0 || strcmp(work.kind, "exec") == 0
                     ? run_processes(&work)
                     : strcmp(work.kind, "stdio") == 0 ? run_stdio(&work) : run_io(&work);
    if (result)
        return result;
    if (printf("{\"kind\":\"%s\",\"operations\":%" PRIu64
               ",\"bytes\":%" PRIu64 ",\"checksum\":%" PRIu64
               ",\"io_calls\":%" PRIu64 "}\n",
               work.kind, work.operations, work.bytes, work.checksum,
               work.io_calls) < 0 || fflush(stdout) != 0) {
        perror("stdout");
        return 1;
    }
    return 0;
}
