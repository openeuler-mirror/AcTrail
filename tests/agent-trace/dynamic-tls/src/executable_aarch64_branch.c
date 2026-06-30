#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "post_payload_sleep.h"
#include "workload_exit.h"

#if defined(__GNUC__)
#define ACTRAIL_PATCHABLE_ENTRY __attribute__((patchable_function_entry(32, 0)))
#else
#define ACTRAIL_PATCHABLE_ENTRY
#endif

enum {
  NEGATIVE_BRANCH_RESULT = -7,
  READ_BUFFER_BYTES = 512,
};

static const char *fallback_reply(void) {
  const char *reply = getenv("ACTRAIL_DYNAMIC_TLS_REPLY");
  return reply == NULL ? "" : reply;
}

#if defined(__aarch64__) && defined(__GNUC__)
int SSL_write(void *ssl, const void *buffer, int length);
__asm__(
    ".text\n\t"
    ".balign 4\n\t"
    ".global SSL_write\n\t"
    ".type SSL_write, %function\n\t"
    "SSL_write:\n\t"
    "cmp w2, #0\n\t"
    "b.lt 1f\n\t"
    "mov w0, w2\n\t"
    "nop\n\t"
    "nop\n\t"
    "nop\n\t"
    "ret\n\t"
    "1:\n\t"
    "mov w0, #-7\n\t"
    "ret\n\t"
    ".size SSL_write, .-SSL_write\n\t");
#else
int SSL_write(void *ssl, const void *buffer, int length) {
  (void)ssl;
  (void)buffer;
  return length < 0 ? NEGATIVE_BRANCH_RESULT : length;
}
#endif

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
  const char *reply = fallback_reply();
  size_t reply_len = strlen(reply);
  if (buffer == NULL || length <= 0 || reply_len == 0) {
    return 0;
  }
  size_t limit = (size_t)length;
  size_t copied = reply_len < limit ? reply_len : limit;
  memcpy(buffer, reply, copied);
  return (int)copied;
}

ACTRAIL_PATCHABLE_ENTRY int SSL_read_ex(void *ssl, void *buffer, size_t length,
                                         size_t *read_bytes) {
  (void)ssl;
  const char *reply = fallback_reply();
  size_t reply_len = strlen(reply);
  if (read_bytes != NULL) {
    *read_bytes = 0;
  }
  if (buffer == NULL || length == 0 || reply_len == 0) {
    return 0;
  }
  size_t copied = reply_len < length ? reply_len : length;
  memcpy(buffer, reply, copied);
  if (read_bytes != NULL) {
    *read_bytes = copied;
  }
  return 1;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s PAYLOAD\n", argv[0]);
    return DYN_TLS_EXIT_USAGE;
  }
  const char *payload = argv[1];
  size_t payload_len = strlen(payload);
  int written = SSL_write((void *)payload, payload, (int)payload_len);
  if (written != (int)payload_len) {
    fprintf(stderr, "SSL_write returned %d for %zu bytes\n", written, payload_len);
    return DYN_TLS_EXIT_SSL_WRITE;
  }
  int negative = SSL_write((void *)payload, payload, -1);
  if (negative != NEGATIVE_BRANCH_RESULT) {
    fprintf(stderr, "SSL_write negative branch returned %d\n", negative);
    return DYN_TLS_EXIT_SSL_WRITE_BRANCH;
  }
  char reply[READ_BUFFER_BYTES];
  size_t read = 0;
  int read_ok = SSL_read_ex((void *)payload, reply, sizeof(reply), &read);
  if (read_ok != 1 || read == 0) {
    fprintf(stderr, "SSL_read_ex returned %d for %zu bytes\n", read_ok, read);
    return DYN_TLS_EXIT_SSL_READ;
  }
  printf("dynamic-executable-aarch64-branch-reply=%.*s\n", (int)read, reply);
  if (actrail_sleep_after_payload() != 0) {
    return DYN_TLS_EXIT_POST_PAYLOAD_SLEEP;
  }
  return EXIT_SUCCESS;
}
