#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define REQUEST_BYTES 4096
#define RESPONSE_BYTES 4096

#if defined(ACTRAIL_USE_GNUTLS)
#include <sys/types.h>
ssize_t gnutls_record_send(void *session, const void *data, size_t data_size);
ssize_t gnutls_record_recv(void *session, void *data, size_t data_size);
#elif defined(ACTRAIL_USE_NSPR)
#include <stdint.h>
typedef int32_t PRInt32;
typedef int32_t PRIntn;
typedef uint32_t PRIntervalTime;
typedef struct PRFileDesc PRFileDesc;
PRInt32 PR_Write(PRFileDesc *fd, const void *buf, PRInt32 amount);
PRInt32 PR_Send(PRFileDesc *fd, const void *buf, PRInt32 amount, PRIntn flags,
                PRIntervalTime timeout);
PRInt32 PR_Read(PRFileDesc *fd, void *buf, PRInt32 amount);
PRInt32 PR_Recv(PRFileDesc *fd, void *buf, PRInt32 amount, PRIntn flags,
                PRIntervalTime timeout);
#else
#error "ACTRAIL_USE_GNUTLS or ACTRAIL_USE_NSPR must be defined"
#endif

static int sleep_from_env(const char *name) {
  const char *raw = getenv(name);
  if (raw == NULL || raw[0] == '\0') {
    return 0;
  }
  errno = 0;
  char *end = NULL;
  long millis = strtol(raw, &end, 10);
  if (errno != 0 || end == raw || *end != '\0' || millis < 0) {
    fprintf(stderr, "invalid %s=%s\n", name, raw);
    return 1;
  }
  struct timespec remaining = {
      .tv_sec = millis / 1000,
      .tv_nsec = (millis % 1000) * 1000000L,
  };
  while (remaining.tv_sec != 0 || remaining.tv_nsec != 0) {
    if (nanosleep(&remaining, &remaining) == 0) {
      return 0;
    }
    if (errno != EINTR) {
      perror("nanosleep");
      return 1;
    }
  }
  return 0;
}

static int build_request(char *buffer, size_t size, const char *model, const char *prompt) {
  char body[2048];
  int body_len = snprintf(
      body,
      sizeof(body),
      "{\"model\":\"%s\",\"messages\":[{\"role\":\"user\",\"content\":\"%s\"}],\"stream\":false}",
      model,
      prompt);
  if (body_len < 0 || (size_t)body_len >= sizeof(body)) {
    return -1;
  }
  int request_len = snprintf(
      buffer,
      size,
      "POST /v1/chat/completions HTTP/1.1\r\n"
      "Host: legacy-tls.local\r\n"
      "Content-Type: application/json\r\n"
      "Authorization: Bearer test-key\r\n"
      "Content-Length: %d\r\n"
      "Connection: close\r\n"
      "\r\n"
      "%s",
      body_len,
      body);
  if (request_len < 0 || (size_t)request_len >= size) {
    return -1;
  }
  return request_len;
}

int main(int argc, char **argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: %s MODEL PROMPT\n", argv[0]);
    return 2;
  }
  char request[REQUEST_BYTES];
  char response[RESPONSE_BYTES];
  int request_len = build_request(request, sizeof(request), argv[1], argv[2]);
  if (request_len <= 0) {
    fprintf(stderr, "failed to build request\n");
    return 3;
  }
  if (sleep_from_env("ACTRAIL_LEGACY_TLS_LLM_PRE_PAYLOAD_SLEEP_MS") != 0) {
    return 6;
  }
  int stream_key = 7;
#if defined(ACTRAIL_USE_GNUTLS)
  ssize_t written = gnutls_record_send(&stream_key, request, (size_t)request_len);
  if (written != request_len) {
    fprintf(stderr, "gnutls_record_send returned %zd for %d bytes\n", written, request_len);
    return 4;
  }
  ssize_t read = gnutls_record_recv(&stream_key, response, sizeof(response));
  if (read <= 0) {
    fprintf(stderr, "gnutls_record_recv returned %zd\n", read);
    return 5;
  }
  printf("legacy-tls-provider=gnutls response-bytes=%zd\n", read);
#elif defined(ACTRAIL_USE_NSPR_SEND_RECV)
  PRInt32 written =
      PR_Send((PRFileDesc *)&stream_key, request, (PRInt32)request_len, 0, 0);
  if (written != request_len) {
    fprintf(stderr, "PR_Send returned %d for %d bytes\n", written, request_len);
    return 4;
  }
  PRInt32 read =
      PR_Recv((PRFileDesc *)&stream_key, response, (PRInt32)sizeof(response), 0, 0);
  if (read <= 0) {
    fprintf(stderr, "PR_Recv returned %d\n", read);
    return 5;
  }
  printf("legacy-tls-provider=nss-sendrecv response-bytes=%d\n", read);
#else
  PRInt32 written = PR_Write((PRFileDesc *)&stream_key, request, (PRInt32)request_len);
  if (written != request_len) {
    fprintf(stderr, "PR_Write returned %d for %d bytes\n", written, request_len);
    return 4;
  }
  PRInt32 read = PR_Read((PRFileDesc *)&stream_key, response, (PRInt32)sizeof(response));
  if (read <= 0) {
    fprintf(stderr, "PR_Read returned %d\n", read);
    return 5;
  }
  printf("legacy-tls-provider=nss response-bytes=%d\n", read);
#endif
  printf("legacy-tls-response=%.*s\n", 64, response);
  return sleep_from_env("ACTRAIL_LEGACY_TLS_LLM_POST_PAYLOAD_SLEEP_MS");
}
