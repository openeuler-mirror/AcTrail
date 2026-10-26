#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <openssl/err.h>
#include <openssl/ssl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static int connect_local(int port) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in address = {.sin_family = AF_INET,
                                 .sin_port = htons(port)};
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (fd < 0 || connect(fd, (struct sockaddr *)&address, sizeof(address))) {
        perror("connect");
        exit(2);
    }
    return fd;
}

int main(int argc, char **argv) {
    if (argc != 4) return 2;
    signal(SIGPIPE, SIG_IGN);
    setbuf(stdout, NULL);
    int fd = connect_local(atoi(argv[1]));
    SSL_CTX *context = SSL_CTX_new(TLS_client_method());
    SSL_CTX_set_verify(context, SSL_VERIFY_NONE, NULL);
    SSL *ssl = SSL_new(context);
    SSL_set_fd(ssl, fd);
    int limited = strcmp(argv[3], "limited") == 0;
    if (!limited) SSL_set_mode(ssl, SSL_MODE_ENABLE_PARTIAL_WRITE);
    if (SSL_connect(ssl) != 1) { ERR_print_errors_fp(stderr); return 2; }
    int failure = strcmp(argv[3], "failure") == 0;
    if (failure) {
        int size = 4096;
        setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &size, sizeof(size));
        fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK);
    }
    const int requested = 131072;
    char *buffer = malloc(requested);
    if (!buffer) return 2;
    memset(buffer, 'x', requested);
    int saw_want = 0, saw_failure = 0, saw_short = 0, saw_full = 0;
    for (int attempt = 0; attempt < 10000; attempt++) {
        ERR_clear_error();
        errno = 0;
        int result = SSL_write(ssl, buffer, requested);
        int saved_errno = errno;
        int error = SSL_get_error(ssl, result);
        printf("{\"pid\":%d,\"attempt\":%d,\"requested\":%d,\"result\":%d,"
               "\"ssl_error\":%d,\"errno\":%d}\n",
               getpid(), attempt, requested, result, error, saved_errno);
        if (result > 0 && result < requested) saw_short = 1;
        if (result == requested) saw_full = 1;
        if (!failure) break;
        if (result <= 0 && error != SSL_ERROR_WANT_WRITE && error != SSL_ERROR_WANT_READ) {
            saw_failure = 1;
            break;
        }
        if (error == SSL_ERROR_WANT_WRITE && !saw_want) {
            saw_want = 1;
            int control = connect_local(atoi(argv[2]));
            char ack;
            if (read(control, &ack, 1) != 1) return 2;
            close(control);
            usleep(100000);
        }
        if (error == SSL_ERROR_WANT_WRITE || error == SSL_ERROR_WANT_READ) usleep(1000);
    }
    if (!failure) {
        int control = connect_local(atoi(argv[2]));
        char ack;
        if (read(control, &ack, 1) != 1) return 2;
        close(control);
    }
    free(buffer);
    SSL_free(ssl);
    SSL_CTX_free(context);
    close(fd);
    return failure ? !(saw_want && saw_failure) : (limited ? !saw_full : !saw_short);
}
