#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>

#if defined(__GNUC__)
#define ACTRAIL_PATCHABLE_ENTRY __attribute__((patchable_function_entry(32, 0)))
#else
#define ACTRAIL_PATCHABLE_ENTRY
#endif

static const char *response_payload(void) {
  const char *value = getenv("ACTRAIL_LEGACY_TLS_LLM_RESPONSE");
  return value == NULL ? "" : value;
}

ACTRAIL_PATCHABLE_ENTRY ssize_t gnutls_record_send(void *session,
                                                   const void *data,
                                                   size_t data_size) {
  (void)session;
  (void)data;
  return (ssize_t)data_size;
}

ACTRAIL_PATCHABLE_ENTRY ssize_t gnutls_record_recv(void *session,
                                                   void *data,
                                                   size_t data_size) {
  (void)session;
  const char *response = response_payload();
  size_t response_len = strlen(response);
  if (data == NULL || data_size == 0 || response_len == 0) {
    return 0;
  }
  size_t copied = response_len < data_size ? response_len : data_size;
  memcpy(data, response, copied);
  return (ssize_t)copied;
}
