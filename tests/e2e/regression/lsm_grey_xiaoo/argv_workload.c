/* Execute ELF images while a thread changes the original shared argv. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

struct argv_workload {
    volatile char *payload;
    size_t length;
    atomic_int stop;
};

static void *mutate(void *data)
{
    struct argv_workload *work = data;
    unsigned int round = 0;
    while (!atomic_load(&work->stop)) {
        char byte = round++ % 2 ? 'A' : 'B';
        for (size_t i = 0; i < work->length; i++)
            work->payload[i] = byte;
    }
    return NULL;
}

int main(int argc, char **argv)
{
    if (argc >= 4 && !strcmp(argv[1], "--argv-probe")) {
        /* Report the actual argv delivered to the new image. */
        printf("{\"pid\":%d,\"errno\":0,\"syscall\":\"%s\",\"argc\":%d,\"payload\":\"%s\"}\n",
               getpid(), argv[3], argc, argv[2]);
        return 0;
    }
    if (argc != 6)
        return 2;
    unsigned int rounds = strtoul(argv[2], NULL, 10);
    unsigned int length = strtoul(argv[3], NULL, 10);
    unsigned int arguments = strtoul(argv[4], NULL, 10);
    int racing = atoi(argv[5]);
    long page_size = sysconf(_SC_PAGESIZE);
    if (!rounds || length > 256 || arguments < 4 || arguments > 17 || page_size < 256)
        return 2;
    char *mapping = mmap(NULL, page_size * 2, PROT_READ | PROT_WRITE,
                         MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (mapping == MAP_FAILED)
        return 3;
    struct argv_workload work = {.payload = mapping + page_size - 100, .length = length};
    memset((char *)work.payload, 'A', length);
    work.payload[length] = 0;
    pthread_t writer;
    if (racing && pthread_create(&writer, NULL, mutate, &work))
        return 4;
    for (unsigned int round = 0; round < rounds; round++) {
        char *args[18] = {argv[1], "--argv-probe", (char *)work.payload,
                           round % 2 ? "execveat" : "execve"};
        for (unsigned int i = 4; i < arguments; i++)
            args[i] = "filler";
        pid_t child = fork();
        if (child < 0)
            return 5;
        if (!child) {
            char *environment[] = {NULL};
            if (round % 2)
                syscall(SYS_execveat, AT_FDCWD, argv[1], args, environment, 0);
            else
                execve(argv[1], args, environment);
            int error = errno;
            dprintf(STDOUT_FILENO, "{\"pid\":%d,\"errno\":%d,\"syscall\":\"%s\"}\n",
                    getpid(), error, args[3]);
            _exit(0);
        }
        int status;
        if (waitpid(child, &status, 0) != child || !WIFEXITED(status) || WEXITSTATUS(status))
            return 6;
    }
    atomic_store(&work.stop, 1);
    if (racing)
        pthread_join(writer, NULL);
    munmap(mapping, page_size * 2);
    return 0;
}
