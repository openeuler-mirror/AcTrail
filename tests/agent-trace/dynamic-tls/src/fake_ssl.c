#include <stddef.h>
#include <stdlib.h>
#include <string.h>

static const char *fallback_reply(void) {
  const char *reply = getenv("ACTRAIL_DYNAMIC_TLS_REPLY");
  return reply == NULL ? "" : reply;
}

int SSL_write(void *ssl, const void *buffer, int length) {
  (void)ssl;
  (void)buffer;
  return length;
}

int SSL_write_ex(void *ssl, const void *buffer, size_t length, size_t *written) {
  (void)ssl;
  (void)buffer;
  if (written != NULL) {
    *written = length;
  }
  return 1;
}

int SSL_read(void *ssl, void *buffer, int length) {
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

int SSL_read_ex(void *ssl, void *buffer, size_t length, size_t *read_bytes) {
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
