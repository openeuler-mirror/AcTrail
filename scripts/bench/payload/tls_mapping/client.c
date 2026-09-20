#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <openssl/ssl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <unistd.h>

static void fail(const char *message) { perror(message); exit(2); }

static int connect_local(int port) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in address = {.sin_family = AF_INET, .sin_port = htons(port)};
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (fd < 0 || connect(fd, (struct sockaddr *)&address, sizeof(address))) fail("connect");
    return fd;
}

static void acknowledge(FILE *control, int expected) {
    fflush(control);
    if (fgetc(control) != expected) { fprintf(stderr, "coordination failed\n"); exit(2); }
}

static void *symbol(void *library, const char *name) {
    void *value = dlsym(library, name);
    if (!value) { fprintf(stderr, "%s: %s\n", name, dlerror()); exit(2); }
    return value;
}

int main(int argc, char **argv) {
    if (argc != 6) return 2;
    signal(SIGPIPE, SIG_IGN);
    setbuf(stdout, NULL);
    int file = open(argv[1], O_RDONLY);
    struct stat metadata;
    if (file < 0 || fstat(file, &metadata)) fail("library");
    size_t length = (size_t)metadata.st_size;
    void *mapping = mmap(NULL, length, PROT_READ, MAP_PRIVATE, file, 0);
    if (mapping == MAP_FAILED) fail("mmap");
    int control_fd = connect_local(atoi(argv[3]));
    FILE *control = fdopen(control_fd, "r+");
    if (!control) fail("fdopen");
    setbuf(control, NULL);
    fprintf(control, "{\"phase\":\"readonly\",\"pid\":%d,\"inode\":%llu,\"start\":%llu,\"length\":%zu,\"mode\":\"%s\"}\n",
            getpid(), (unsigned long long)metadata.st_ino,
            (unsigned long long)(uintptr_t)mapping, length, argv[2]);
    acknowledge(control, 'X');
    if (!strcmp(argv[2], "unlink") && unlink(argv[1])) fail("unlink");
    if (mprotect(mapping, length, PROT_READ | PROT_EXEC)) fail("mprotect");
    fprintf(control, "{\"phase\":\"executable\",\"pid\":%d,\"inode\":%llu}\n",
            getpid(), (unsigned long long)metadata.st_ino);
    acknowledge(control, 'D');
    char path[64];
    snprintf(path, sizeof(path), "/proc/self/fd/%d", file);
    void *library = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (!library) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 2; }
#define LOAD(result, name, arguments) result (*call_##name) arguments = symbol(library, #name)
    LOAD(const SSL_METHOD *, TLS_client_method, (void));
    LOAD(SSL_CTX *, SSL_CTX_new, (const SSL_METHOD *));
    LOAD(void, SSL_CTX_set_verify, (SSL_CTX *, int, SSL_verify_cb));
    LOAD(SSL *, SSL_new, (SSL_CTX *));
    LOAD(int, SSL_set_fd, (SSL *, int));
    LOAD(int, SSL_connect, (SSL *));
    LOAD(int, SSL_write, (SSL *, const void *, int));
    LOAD(int, SSL_read, (SSL *, void *, int));
    LOAD(void, SSL_free, (SSL *));
    LOAD(void, SSL_CTX_free, (SSL_CTX *));
    SSL_CTX *context = call_SSL_CTX_new(call_TLS_client_method());
    if (!context) return 2;
    call_SSL_CTX_set_verify(context, SSL_VERIFY_NONE, NULL);
    SSL *ssl = call_SSL_new(context);
    if (!ssl) return 2;
    int network = connect_local(atoi(argv[4]));
    if (call_SSL_set_fd(ssl, network) != 1 || call_SSL_connect(ssl) != 1) return 2;
    const char *body = "{\"model\":\"deepseek-v4-flash\",\"stream\":true,\"messages\":[{\"role\":\"user\",\"content\":\"Return the configured response.\"}]}";
    char request[2048];
    int count = snprintf(request, sizeof(request), "POST %s HTTP/1.1\r\nHost: 127.0.0.1:%s\r\nContent-Type: application/json\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n%s", argv[5], argv[4], strlen(body), body);
    if (count <= 0 || count >= (int)sizeof(request)) return 2;
    for (int sent = 0; sent < count;) {
        int result = call_SSL_write(ssl, request + sent, count - sent);
        if (result <= 0) return 2;
        sent += result;
    }
    char response[8192];
    int received;
    while ((received = call_SSL_read(ssl, response, sizeof(response))) > 0) {
        fwrite(response, 1, (size_t)received, stdout);
    }
    call_SSL_free(ssl);
    call_SSL_CTX_free(context);
    close(network);
    dlclose(library);
    munmap(mapping, length);
    close(file);
    fprintf(control, "{\"phase\":\"done\",\"pid\":%d}\n", getpid());
    acknowledge(control, 'A');
    fclose(control);
    return 0;
}
