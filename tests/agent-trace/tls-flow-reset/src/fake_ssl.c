#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(__GNUC__)
#define ACTRAIL_PATCHABLE_ENTRY __attribute__((patchable_function_entry(32, 0)))
#else
#define ACTRAIL_PATCHABLE_ENTRY
#endif

static int read_count = 0;

static const unsigned char binary_response[] =
    "HTTP/1.1 200 OK\r\n"
    "Content-Type: application/octet-stream\r\n"
    "Content-Length: 4\r\n"
    "\r\n"
    "\x7f"
    "ELF";

static const char *text_marker(void) {
  const char *value = getenv("ACTRAIL_TLS_FLOW_RESET_TEXT_MARKER");
  return value == NULL ? "ACTRAIL_TLS_FLOW_RESET_TEXT_RESPONSE" : value;
}

static int copy_bytes(void *buffer, int length, const unsigned char *bytes,
                      size_t byte_count) {
  if (buffer == NULL || length <= 0) {
    return 0;
  }
  size_t limit = (size_t)length;
  size_t copied = byte_count < limit ? byte_count : limit;
  memcpy(buffer, bytes, copied);
  return (int)copied;
}

ACTRAIL_PATCHABLE_ENTRY int SSL_write(void *ssl, const void *buffer, int length) {
  (void)ssl;
  (void)buffer;
  return length;
}

ACTRAIL_PATCHABLE_ENTRY int SSL_write_ex(void *ssl, const void *buffer, size_t length,
                                         size_t *written) {
  (void)ssl;
  (void)buffer;
  if (written != NULL) {
    *written = length;
  }
  return 1;
}

ACTRAIL_PATCHABLE_ENTRY int SSL_write_ex2(void *ssl, const void *buffer, size_t length,
                                          uint64_t flags, size_t *written) {
  (void)ssl;
  (void)buffer;
  (void)flags;
  if (written != NULL) {
    *written = length;
  }
  return 1;
}

ACTRAIL_PATCHABLE_ENTRY int SSL_read(void *ssl, void *buffer, int length) {
  (void)ssl;
  int current = read_count++;
  if (current == 0) {
    return copy_bytes(buffer, length, binary_response, sizeof(binary_response) - 1);
  }

  char response[512];
  const char *marker = text_marker();
  int body_len = snprintf(NULL, 0, "{\"marker\":\"%s\"}", marker);
  if (body_len <= 0) {
    return 0;
  }
  int response_len = snprintf(response, sizeof(response),
                              "HTTP/1.1 200 OK\r\n"
                              "Content-Type: application/json\r\n"
                              "Content-Length: %d\r\n"
                              "\r\n"
                              "{\"marker\":\"%s\"}",
                              body_len, marker);
  if (response_len <= 0 || (size_t)response_len >= sizeof(response)) {
    return 0;
  }
  return copy_bytes(buffer, length, (const unsigned char *)response, (size_t)response_len);
}

ACTRAIL_PATCHABLE_ENTRY int SSL_read_ex(void *ssl, void *buffer, size_t length,
                                        size_t *read_bytes) {
  if (read_bytes != NULL) {
    *read_bytes = 0;
  }
  if (length > (size_t)INT32_MAX) {
    return 0;
  }
  int read = SSL_read(ssl, buffer, (int)length);
  if (read <= 0) {
    return 0;
  }
  if (read_bytes != NULL) {
    *read_bytes = (size_t)read;
  }
  return 1;
}
