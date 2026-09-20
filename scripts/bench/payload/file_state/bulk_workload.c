#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

static void require(int valid, const char *operation) {
    if (!valid) { perror(operation); exit(1); }
}

static void read_file(const char *path) {
    int fd = open(path, O_RDONLY);
    char value;
    require(fd >= 0 && read(fd, &value, 1) == 1 && value == 'X', "read file");
    require(close(fd) == 0, "close file");
}

static void missing_file(const char *path) {
    errno = 0;
    require(open(path, O_RDONLY) == -1 && errno == ENOENT, "missing file");
}

int main(void) {
    require(mkdir("bulk-chain", 0700) == 0 && chdir("bulk-chain") == 0, "workspace");
    const char *paths[] = {"A", "B", "C", "boundary"};
    for (unsigned int index = 0; index < sizeof(paths) / sizeof(paths[0]); index++) {
        int fd = open(paths[index], O_CREAT | O_WRONLY | O_TRUNC, 0600);
        require(fd >= 0 && write(fd, "X", 1) == 1 && close(fd) == 0, "prepare file");
    }
    read_file("A");
    missing_file("before");
    read_file("B");
    missing_file("after");
    read_file("C");
    FILE *proof = fopen("../file-proof.json", "w");
    require(proof != NULL, "proof");
    fprintf(proof, "{\"pid\":%d,\"status\":\"passed\",\"reads\":3,\"errors\":2}\n", getpid());
    require(fclose(proof) == 0, "proof close");
    return 0;
}
