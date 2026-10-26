#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/openat2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static void require(int valid, const char *operation) {
    if (!valid) { perror(operation); exit(1); }
}

int main(int argc, char **argv) {
    if (argc == 5 && strcmp(argv[1], "child") == 0) {
        int inherited = atoi(argv[2]);
        int closed = atoi(argv[3]);
        errno = 0;
        require(fcntl(closed, F_GETFD) == -1 && errno == EBADF, "exec cloexec");
        errno = 0;
        require(openat(atoi(argv[4]), "closed-directory-child", O_RDONLY) == -1 && errno == EBADF,
                "exec closed directory");
        require(write(inherited, "E", 1) == 1, "exec inherited write");
        return 0;
    }
    char executable[PATH_MAX];
    require(realpath(argv[0], executable) != NULL, "helper executable");
    int root = open(".", O_RDONLY | O_DIRECTORY);
    require(root >= 0 && mkdir("file-chain", 0700) == 0, "workspace");
    require(chdir("file-chain") == 0, "chdir");
    int source = open("A", O_CREAT | O_RDWR | O_TRUNC, 0600);
    require(source >= 0 && write(source, "A", 1) == 1, "open/write A");
    for (int index = 0; index < 128; index++) {
        require(fcntl(source, F_GETFD) >= 0, "get descriptor flags");
        int flags = fcntl(source, F_GETFL);
        require(flags >= 0 && fcntl(source, F_SETFL, flags) == 0, "file status flags");
    }
    int alias = fcntl(source, F_DUPFD, 100);
    int cloexec = fcntl(source, F_DUPFD_CLOEXEC, 110);
    require(alias >= 100 && cloexec >= 110, "fcntl duplicate");
    require(fcntl(alias, F_SETFD, FD_CLOEXEC) == 0, "set cloexec");
    require(fcntl(alias, F_SETFD, 0) == 0, "clear cloexec");
    require(close(source) == 0 && write(alias, "D", 1) == 1, "alias write");
    require(ftruncate(alias, 4096) == 0, "truncate");
    char *mapping = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, alias, 0);
    require(mapping != MAP_FAILED, "mmap");
    mapping[2] = 'M';
    require(msync(mapping, 4096, MS_SYNC) == 0, "msync");
    char data[5];
    require(lseek(alias, 0, SEEK_SET) == 0 && read(alias, data, 3) == 3, "alias read");
    require(memcmp(data, "ADM", 3) == 0, "mapped data");
    for (int index = 0; index < 32; index++) {
        void *private_mapping = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE, alias, 0);
        require(private_mapping != MAP_FAILED, "private mmap");
        require(munmap(private_mapping, 4096) == 0, "private munmap");
    }
    char *anonymous = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    require(anonymous != MAP_FAILED, "shared anonymous mmap");
    anonymous[0] = 'X';
    require(munmap(anonymous, 4096) == 0, "anonymous munmap");
    int directory = open(".", O_RDONLY | O_DIRECTORY);
    require(directory >= 0 && fchdir(root) == 0, "directory fd");
    struct open_how how = { .flags = O_RDWR };
    int relative = syscall(SYS_openat2, directory, "A", &how, sizeof(how));
    require(relative >= 0 && write(relative, "R", 1) == 1, "relative openat2/write");
    require(close(relative) == 0, "close relative");
    int closed_directory = fcntl(directory, F_DUPFD, 120);
    require(closed_directory >= 120 && fcntl(closed_directory, F_SETFD, FD_CLOEXEC) == 0,
            "directory cloexec");
    pid_t child = fork();
    require(child >= 0, "fork");
    if (child == 0) {
        require(write(alias, "F", 1) == 1, "fork inherited write");
        char inherited_arg[32], closed_arg[32], directory_arg[32];
        snprintf(inherited_arg, sizeof(inherited_arg), "%d", alias);
        snprintf(closed_arg, sizeof(closed_arg), "%d", cloexec);
        snprintf(directory_arg, sizeof(directory_arg), "%d", closed_directory);
        execl(executable, executable, "child", inherited_arg, closed_arg, directory_arg, NULL);
        perror("exec helper");
        _exit(1);
    }
    int status;
    require(waitpid(child, &status, 0) == child && WIFEXITED(status) && WEXITSTATUS(status) == 0,
            "child completed");
    int replacement = openat(directory, "B", O_CREAT | O_RDWR | O_TRUNC, 0600);
    require(replacement >= 0 && dup2(replacement, alias) == alias, "fd reuse");
    require(write(alias, "B", 1) == 1, "reused fd write");
    errno = 0;
    require(fcntl(-1, F_DUPFD, 0) == -1 && errno == EBADF, "failed fcntl");
    errno = 0;
    require(close(-1) == -1 && errno == EBADF, "failed close");
    require(renameat(directory, "B", directory, "B-renamed") == 0, "renameat");
    require(unlinkat(directory, "B-renamed", 0) == 0, "unlinkat");
    errno = 0;
    require(openat(directory, "missing", O_RDONLY) == -1 && errno == ENOENT, "failed openat");
    int check = openat(directory, "A", O_RDONLY);
    require(check >= 0 && read(check, data, sizeof(data)) == sizeof(data), "final A read");
    require(memcmp(data, "RDMFE", sizeof(data)) == 0, "final A content");
    FILE *proof = fopen("file-proof.json", "w");
    require(proof != NULL, "proof");
    fprintf(proof, "{\"pid\":%d,\"child_pid\":%d,\"source_fd\":%d,\"alias_fd\":%d,"
            "\"cloexec_fd\":%d,\"relative_fd\":%d,\"replacement_fd\":%d,"
            "\"closed_directory_fd\":%d,\"status\":\"passed\"}\n",
            getpid(), child, source, alias, cloexec, relative, replacement, closed_directory);
    require(fclose(proof) == 0, "proof close");
    munmap(mapping, 4096);
    close(check); close(alias); close(cloexec); close(replacement); close(directory); close(root);
    return 0;
}
