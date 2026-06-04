#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <openssl/err.h>
#include <openssl/ssl.h>
#include <netdb.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static const int HTTP_SUCCESS_MIN = 200;
static const int HTTP_SUCCESS_MAX_EXCLUSIVE = 300;
static const int CHILD_EXEC_FAILED = 127;
static const int MSEC_PER_SEC = 1000;
static const int NSEC_PER_MSEC = 1000000;

struct Buffer {
    char *data;
    size_t len;
};

struct Args {
    const char *api_key_env;
    const char *api_host;
    const char *api_port;
    const char *api_path;
    const char *model;
    const char *agent_a_prompt;
    const char *script_b;
    const char *xiaoo_prompt;
    const char *xiaoo_provider;
    const char *xiaoo_model;
    const char *xiaoo_max_turns;
    const char *xiaoo_no_tools;
    const char *expected_a_output;
    uint64_t child_timeout_seconds;
    uint64_t child_poll_interval_millis;
    size_t io_chunk_bytes;
};

static void die(const char *message) {
    fprintf(stderr, "agent A failed: %s\n", message);
    exit(EXIT_FAILURE);
}

static void die_errno(const char *message) {
    fprintf(stderr, "agent A failed: %s: %s\n", message, strerror(errno));
    exit(EXIT_FAILURE);
}

static void buffer_append(struct Buffer *buffer, const char *data, size_t len) {
    size_t next_len = buffer->len + len;
    char *next = realloc(buffer->data, next_len + 1);
    if (next == NULL) {
        die("out of memory");
    }
    memcpy(next + buffer->len, data, len);
    next[next_len] = '\0';
    buffer->data = next;
    buffer->len = next_len;
}

static void buffer_append_cstr(struct Buffer *buffer, const char *value) {
    buffer_append(buffer, value, strlen(value));
}

static void buffer_append_json_string(struct Buffer *buffer, const char *value) {
    static const char hex[] = "0123456789abcdef";
    buffer_append_cstr(buffer, "\"");
    for (const unsigned char *cursor = (const unsigned char *)value; *cursor != '\0'; cursor++) {
        unsigned char byte = *cursor;
        if (byte == '"' || byte == '\\') {
            char escaped[] = {'\\', (char)byte};
            buffer_append(buffer, escaped, sizeof(escaped));
        } else if (byte == '\n') {
            buffer_append_cstr(buffer, "\\n");
        } else if (byte == '\r') {
            buffer_append_cstr(buffer, "\\r");
        } else if (byte == '\t') {
            buffer_append_cstr(buffer, "\\t");
        } else if (byte < 0x20) {
            char escaped[] = {'\\', 'u', '0', '0', hex[byte >> 4], hex[byte & 0x0f]};
            buffer_append(buffer, escaped, sizeof(escaped));
        } else {
            char raw[] = {(char)byte};
            buffer_append(buffer, raw, sizeof(raw));
        }
    }
    buffer_append_cstr(buffer, "\"");
}

static const char *option_value(int argc, char **argv, int *index) {
    if (*index + 1 >= argc) {
        die("option is missing a value");
    }
    *index += 1;
    return argv[*index];
}

static uint64_t parse_u64(const char *value, const char *label) {
    errno = 0;
    char *end = NULL;
    unsigned long long parsed = strtoull(value, &end, 10);
    if (errno != 0 || end == value || *end != '\0') {
        fprintf(stderr, "agent A failed: invalid %s: %s\n", label, value);
        exit(EXIT_FAILURE);
    }
    return (uint64_t)parsed;
}

static struct Args parse_args(int argc, char **argv) {
    struct Args args = {0};
    for (int index = 1; index < argc; index++) {
        const char *option = argv[index];
        if (strcmp(option, "--api-key-env") == 0) {
            args.api_key_env = option_value(argc, argv, &index);
        } else if (strcmp(option, "--api-host") == 0) {
            args.api_host = option_value(argc, argv, &index);
        } else if (strcmp(option, "--api-port") == 0) {
            args.api_port = option_value(argc, argv, &index);
        } else if (strcmp(option, "--api-path") == 0) {
            args.api_path = option_value(argc, argv, &index);
        } else if (strcmp(option, "--model") == 0) {
            args.model = option_value(argc, argv, &index);
        } else if (strcmp(option, "--agent-a-prompt") == 0) {
            args.agent_a_prompt = option_value(argc, argv, &index);
        } else if (strcmp(option, "--script-b") == 0) {
            args.script_b = option_value(argc, argv, &index);
        } else if (strcmp(option, "--xiaoo-prompt") == 0) {
            args.xiaoo_prompt = option_value(argc, argv, &index);
        } else if (strcmp(option, "--xiaoo-provider") == 0) {
            args.xiaoo_provider = option_value(argc, argv, &index);
        } else if (strcmp(option, "--xiaoo-model") == 0) {
            args.xiaoo_model = option_value(argc, argv, &index);
        } else if (strcmp(option, "--xiaoo-max-turns") == 0) {
            args.xiaoo_max_turns = option_value(argc, argv, &index);
        } else if (strcmp(option, "--xiaoo-no-tools") == 0) {
            args.xiaoo_no_tools = option_value(argc, argv, &index);
        } else if (strcmp(option, "--child-timeout-seconds") == 0) {
            args.child_timeout_seconds = parse_u64(option_value(argc, argv, &index), option + 2);
        } else if (strcmp(option, "--child-poll-interval-millis") == 0) {
            args.child_poll_interval_millis =
                parse_u64(option_value(argc, argv, &index), option + 2);
        } else if (strcmp(option, "--io-chunk-bytes") == 0) {
            args.io_chunk_bytes = (size_t)parse_u64(option_value(argc, argv, &index), option + 2);
        } else if (strcmp(option, "--expected-a-output") == 0) {
            args.expected_a_output = option_value(argc, argv, &index);
        } else {
            fprintf(stderr, "agent A failed: unknown option %s\n", option);
            exit(EXIT_FAILURE);
        }
    }
    if (args.api_key_env == NULL || args.api_host == NULL || args.api_port == NULL ||
        args.api_path == NULL || args.model == NULL || args.agent_a_prompt == NULL ||
        args.script_b == NULL || args.xiaoo_prompt == NULL || args.xiaoo_provider == NULL ||
        args.xiaoo_model == NULL || args.xiaoo_max_turns == NULL || args.xiaoo_no_tools == NULL ||
        args.expected_a_output == NULL || args.child_timeout_seconds == 0 ||
        args.child_poll_interval_millis == 0 || args.io_chunk_bytes == 0) {
        die("missing required option");
    }
    return args;
}

static struct Buffer llm_request_body(const struct Args *args) {
    struct Buffer body = {0};
    buffer_append_cstr(&body, "{\"model\":");
    buffer_append_json_string(&body, args->model);
    buffer_append_cstr(&body, ",\"messages\":[{\"role\":\"user\",\"content\":");
    buffer_append_json_string(&body, args->agent_a_prompt);
    buffer_append_cstr(&body, "}],\"stream\":false,\"thinking\":{\"type\":\"disabled\"}}");
    return body;
}

static struct Buffer http_request(const struct Args *args, const char *api_key) {
    struct Buffer body = llm_request_body(args);
    struct Buffer request = {0};
    char length[64];
    snprintf(length, sizeof(length), "%zu", body.len);
    buffer_append_cstr(&request, "POST ");
    buffer_append_cstr(&request, args->api_path);
    buffer_append_cstr(&request, " HTTP/1.1\r\nHost: ");
    buffer_append_cstr(&request, args->api_host);
    buffer_append_cstr(&request, "\r\nContent-Type: application/json\r\nAuthorization: Bearer ");
    buffer_append_cstr(&request, api_key);
    buffer_append_cstr(&request, "\r\nContent-Length: ");
    buffer_append_cstr(&request, length);
    buffer_append_cstr(&request, "\r\nConnection: close\r\n\r\n");
    buffer_append(&request, body.data, body.len);
    free(body.data);
    return request;
}

static int connect_tcp(const char *host, const char *port) {
    struct addrinfo hints = {0};
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    struct addrinfo *addresses = NULL;
    int result = getaddrinfo(host, port, &hints, &addresses);
    if (result != 0) {
        fprintf(stderr, "agent A failed: resolve %s:%s: %s\n", host, port, gai_strerror(result));
        exit(EXIT_FAILURE);
    }

    int fd = -1;
    int last_errno = 0;
    for (struct addrinfo *cursor = addresses; cursor != NULL; cursor = cursor->ai_next) {
        fd = socket(cursor->ai_family, cursor->ai_socktype, cursor->ai_protocol);
        if (fd < 0) {
            last_errno = errno;
            continue;
        }
        if (connect(fd, cursor->ai_addr, cursor->ai_addrlen) == 0) {
            break;
        }
        last_errno = errno;
        close(fd);
        fd = -1;
    }
    freeaddrinfo(addresses);
    if (fd < 0) {
        errno = last_errno;
        die_errno("TCP connect failed");
    }
    return fd;
}

static void ssl_write_all(SSL *ssl, const char *data, size_t len) {
    size_t written = 0;
    while (written < len) {
        int result = SSL_write(ssl, data + written, (int)(len - written));
        if (result > 0) {
            written += (size_t)result;
            continue;
        }
        die("HTTPS write failed");
    }
}

static struct Buffer https_post(const struct Args *args, const char *api_key) {
    int fd = connect_tcp(args->api_host, args->api_port);
    SSL_CTX *ctx = SSL_CTX_new(TLS_client_method());
    if (ctx == NULL) {
        die("create SSL context failed");
    }
    SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, NULL);
    if (SSL_CTX_set_default_verify_paths(ctx) != 1) {
        die("load default TLS trust store failed");
    }

    SSL *ssl = SSL_new(ctx);
    if (ssl == NULL) {
        close(fd);
        SSL_CTX_free(ctx);
        die("create SSL handle failed");
    }
    if (SSL_set_tlsext_host_name(ssl, args->api_host) != 1 ||
        SSL_set1_host(ssl, args->api_host) != 1) {
        SSL_free(ssl);
        close(fd);
        SSL_CTX_free(ctx);
        die("configure TLS hostname failed");
    }
    if (SSL_set_fd(ssl, fd) != 1 || SSL_connect(ssl) != 1) {
        ERR_print_errors_fp(stderr);
        SSL_free(ssl);
        close(fd);
        SSL_CTX_free(ctx);
        die("HTTPS connect failed");
    }

    struct Buffer request = http_request(args, api_key);
    ssl_write_all(ssl, request.data, request.len);
    free(request.data);

    char *chunk = malloc(args->io_chunk_bytes);
    if (chunk == NULL) {
        die("out of memory");
    }
    struct Buffer response = {0};
    for (;;) {
        int read_bytes = SSL_read(ssl, chunk, (int)args->io_chunk_bytes);
        if (read_bytes > 0) {
            buffer_append(&response, chunk, (size_t)read_bytes);
            continue;
        }
        if (read_bytes == 0) {
            break;
        }
        break;
    }
    free(chunk);
    SSL_shutdown(ssl);
    SSL_free(ssl);
    close(fd);
    SSL_CTX_free(ctx);
    return response;
}

static int http_status(const struct Buffer *response) {
    const char *space = strchr(response->data == NULL ? "" : response->data, ' ');
    if (space == NULL) {
        return 0;
    }
    return atoi(space + 1);
}

static int64_t monotonic_millis(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
        die_errno("clock_gettime");
    }
    return ((int64_t)now.tv_sec * MSEC_PER_SEC) + (now.tv_nsec / NSEC_PER_MSEC);
}

static void sleep_millis(uint64_t millis) {
    struct timespec delay = {
        .tv_sec = (time_t)(millis / MSEC_PER_SEC),
        .tv_nsec = (long)((millis % MSEC_PER_SEC) * NSEC_PER_MSEC),
    };
    while (nanosleep(&delay, &delay) != 0 && errno == EINTR) {
    }
}

static void run_script_b(const struct Args *args) {
    pid_t child = fork();
    if (child < 0) {
        die_errno("fork script B");
    }
    if (child == 0) {
        setpgid(0, 0);
        execlp(
            "bash",
            "bash",
            args->script_b,
            args->xiaoo_provider,
            args->xiaoo_model,
            args->xiaoo_max_turns,
            args->xiaoo_no_tools,
            args->xiaoo_prompt,
            (char *)NULL);
        _exit(CHILD_EXEC_FAILED);
    }

    int status = 0;
    int64_t deadline = monotonic_millis() + ((int64_t)args->child_timeout_seconds * MSEC_PER_SEC);
    for (;;) {
        pid_t result = waitpid(child, &status, WNOHANG);
        if (result == child) {
            break;
        }
        if (result < 0) {
            die_errno("wait script B");
        }
        if (monotonic_millis() >= deadline) {
            kill(-child, SIGKILL);
            waitpid(child, &status, 0);
            die("script B timed out");
        }
        sleep_millis(args->child_poll_interval_millis);
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        die("script B exited unsuccessfully");
    }
}

int main(int argc, char **argv) {
    struct Args args = parse_args(argc, argv);
    const char *api_key = getenv(args.api_key_env);
    if (api_key == NULL || api_key[0] == '\0') {
        die("missing API key environment variable");
    }

    SSL_library_init();
    SSL_load_error_strings();
    struct Buffer response = https_post(&args, api_key);
    int status = http_status(&response);
    if (status < HTTP_SUCCESS_MIN || status >= HTTP_SUCCESS_MAX_EXCLUSIVE) {
        fprintf(stderr, "agent A failed: LLM HTTP status=%d\n", status);
        free(response.data);
        return EXIT_FAILURE;
    }
    if (response.data == NULL || strstr(response.data, args.expected_a_output) == NULL) {
        free(response.data);
        die("LLM response did not contain expected marker");
    }
    free(response.data);
    printf("%s\n", args.expected_a_output);
    fflush(stdout);
    run_script_b(&args);
    return EXIT_SUCCESS;
}
