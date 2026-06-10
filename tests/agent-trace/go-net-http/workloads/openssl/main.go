package main

/*
#cgo pkg-config: openssl
#include <openssl/err.h>
#include <openssl/ssl.h>
#include <netdb.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

enum {
    ACTRAIL_OPENSSL_ERROR_BYTES = 256,
    ACTRAIL_OPENSSL_READ_CHUNK_BYTES = 4096
};

struct actrail_openssl_response {
    char *data;
    long len;
    int code;
    char error[ACTRAIL_OPENSSL_ERROR_BYTES];
};

static void actrail_set_error(struct actrail_openssl_response *response, const char *message) {
    snprintf(response->error, ACTRAIL_OPENSSL_ERROR_BYTES, "%s", message);
    response->code = 1;
}

static int actrail_append_response(
    struct actrail_openssl_response *response,
    const char *chunk,
    int chunk_len
) {
    char *next = realloc(response->data, response->len + chunk_len);
    if (!next) {
        actrail_set_error(response, "realloc response buffer failed");
        return 0;
    }
    response->data = next;
    memcpy(response->data + response->len, chunk, chunk_len);
    response->len += chunk_len;
    return 1;
}

static int actrail_connect_tcp(
    const char *host,
    const char *port,
    struct actrail_openssl_response *response
) {
    struct addrinfo hints = {};
    struct addrinfo *result = NULL;
    int fd = -1;

    hints.ai_socktype = SOCK_STREAM;
    hints.ai_family = AF_UNSPEC;
    if (getaddrinfo(host, port, &hints, &result) != 0) {
        actrail_set_error(response, "getaddrinfo failed");
        return -1;
    }
    for (struct addrinfo *item = result; item; item = item->ai_next) {
        fd = socket(item->ai_family, item->ai_socktype, item->ai_protocol);
        if (fd < 0) {
            continue;
        }
        if (connect(fd, item->ai_addr, item->ai_addrlen) == 0) {
            freeaddrinfo(result);
            return fd;
        }
        close(fd);
        fd = -1;
    }
    freeaddrinfo(result);
    actrail_set_error(response, "connect failed");
    return -1;
}

static struct actrail_openssl_response actrail_openssl_post(
    const char *host,
    const char *port,
    const char *request,
    size_t request_len
) {
    struct actrail_openssl_response response = {};
    SSL_CTX *ctx = SSL_CTX_new(TLS_client_method());
    SSL *ssl = NULL;
    int fd = -1;
    size_t written = 0;

    if (!ctx) {
        actrail_set_error(&response, "SSL_CTX_new failed");
        return response;
    }
    if (SSL_CTX_set_default_verify_paths(ctx) != 1) {
        actrail_set_error(&response, "SSL_CTX_set_default_verify_paths failed");
        SSL_CTX_free(ctx);
        return response;
    }
    ssl = SSL_new(ctx);
    if (!ssl) {
        actrail_set_error(&response, "SSL_new failed");
        SSL_CTX_free(ctx);
        return response;
    }
    SSL_set_mode(ssl, SSL_MODE_AUTO_RETRY);
    if (SSL_set_tlsext_host_name(ssl, host) != 1) {
        actrail_set_error(&response, "SSL_set_tlsext_host_name failed");
        SSL_free(ssl);
        SSL_CTX_free(ctx);
        return response;
    }
    fd = actrail_connect_tcp(host, port, &response);
    if (fd < 0) {
        SSL_free(ssl);
        SSL_CTX_free(ctx);
        return response;
    }
    if (SSL_set_fd(ssl, fd) != 1) {
        actrail_set_error(&response, "SSL_set_fd failed");
        SSL_free(ssl);
        close(fd);
        SSL_CTX_free(ctx);
        return response;
    }
    if (SSL_connect(ssl) != 1) {
        actrail_set_error(&response, "SSL_connect failed");
        SSL_free(ssl);
        close(fd);
        SSL_CTX_free(ctx);
        return response;
    }
    if (SSL_get_verify_result(ssl) != X509_V_OK) {
        actrail_set_error(&response, "certificate verification failed");
        SSL_free(ssl);
        close(fd);
        SSL_CTX_free(ctx);
        return response;
    }
    while (written < request_len) {
        int result = SSL_write(ssl, request + written, (int)(request_len - written));
        if (result <= 0) {
            actrail_set_error(&response, "SSL_write failed");
            SSL_free(ssl);
            close(fd);
            SSL_CTX_free(ctx);
            return response;
        }
        written += result;
    }
    for (;;) {
        char chunk[ACTRAIL_OPENSSL_READ_CHUNK_BYTES];
        int read = SSL_read(ssl, chunk, sizeof(chunk));
        if (read > 0) {
            if (!actrail_append_response(&response, chunk, read)) {
                SSL_free(ssl);
                close(fd);
                SSL_CTX_free(ctx);
                return response;
            }
            continue;
        }
        int ssl_error = SSL_get_error(ssl, read);
        if (ssl_error == SSL_ERROR_WANT_READ || ssl_error == SSL_ERROR_WANT_WRITE) {
            continue;
        }
        if (read == 0 || ssl_error == SSL_ERROR_ZERO_RETURN || ssl_error == SSL_ERROR_SYSCALL) {
            break;
        }
        actrail_set_error(&response, "SSL_read failed");
        SSL_free(ssl);
        close(fd);
        SSL_CTX_free(ctx);
        return response;
    }
    SSL_shutdown(ssl);
    SSL_free(ssl);
    close(fd);
    SSL_CTX_free(ctx);
    return response;
}
*/
import "C"

import (
	"encoding/json"
	"flag"
	"fmt"
	"net/url"
	"os"
	"strconv"
	"strings"
	"unsafe"
)

func main() {
	apiURL := flag.String("api-url", "", "OpenAI-compatible chat completions URL")
	apiKeyEnv := flag.String("api-key-env", "", "environment variable containing the API key")
	model := flag.String("model", "", "model name")
	prompt := flag.String("prompt", "", "user prompt")
	flag.Parse()
	requireFlag("api-url", *apiURL)
	requireFlag("api-key-env", *apiKeyEnv)
	requireFlag("model", *model)
	requireFlag("prompt", *prompt)
	apiKey := os.Getenv(*apiKeyEnv)
	if apiKey == "" {
		fmt.Fprintf(os.Stderr, "%s is required\n", *apiKeyEnv)
		os.Exit(2)
	}
	body, err := json.Marshal(map[string]any{
		"model": *model,
		"messages": []map[string]string{
			{"role": "user", "content": *prompt},
		},
		"stream": false,
	})
	if err != nil {
		panic(err)
	}
	response, err := postWithOpenSSL(*apiURL, apiKey, body)
	if err != nil {
		panic(err)
	}
	status, err := httpStatus(response)
	if err != nil {
		panic(err)
	}
	fmt.Printf("go-openssl-status=%d bytes=%d\n", status, len(response))
}

func postWithOpenSSL(apiURL string, apiKey string, body []byte) ([]byte, error) {
	parsed, err := url.Parse(apiURL)
	if err != nil {
		return nil, err
	}
	if parsed.Scheme != "https" {
		return nil, fmt.Errorf("OpenSSL workload requires https URL")
	}
	host := parsed.Hostname()
	port := parsed.Port()
	if port == "" {
		port = "443"
	}
	target := parsed.RequestURI()
	if target == "" {
		target = "/"
	}
	request := strings.Join([]string{
		"POST " + target + " HTTP/1.1",
		"Host: " + parsed.Host,
		"Authorization: Bearer " + apiKey,
		"Content-Type: application/json",
		"Content-Length: " + strconv.Itoa(len(body)),
		"Connection: close",
		"",
		string(body),
	}, "\r\n")
	cHost := C.CString(host)
	defer C.free(unsafe.Pointer(cHost))
	cPort := C.CString(port)
	defer C.free(unsafe.Pointer(cPort))
	cRequest := C.CBytes([]byte(request))
	defer C.free(cRequest)
	response := C.actrail_openssl_post(
		cHost,
		cPort,
		(*C.char)(cRequest),
		C.size_t(len(request)),
	)
	if response.code != 0 {
		return nil, fmt.Errorf(C.GoString(&response.error[0]))
	}
	defer C.free(unsafe.Pointer(response.data))
	return C.GoBytes(unsafe.Pointer(response.data), C.int(response.len)), nil
}

func httpStatus(response []byte) (int, error) {
	line, _, _ := strings.Cut(string(response), "\r\n")
	fields := strings.Fields(line)
	if len(fields) < 2 {
		return 0, fmt.Errorf("missing HTTP status line")
	}
	return strconv.Atoi(fields[1])
}

func requireFlag(name string, value string) {
	if value == "" {
		fmt.Fprintf(os.Stderr, "-%s is required\n", name)
		os.Exit(2)
	}
}
