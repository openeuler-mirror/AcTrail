#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#if defined(__GNUC__)
#define ACTRAIL_PATCHABLE_ENTRY __attribute__((patchable_function_entry(32, 0)))
#else
#define ACTRAIL_PATCHABLE_ENTRY
#endif

typedef int32_t PRInt32;
typedef int32_t PRIntn;
typedef uint32_t PRIntervalTime;
typedef struct PRFileDesc PRFileDesc;

static const char *response_payload(void) {
  const char *value = getenv("ACTRAIL_LEGACY_TLS_LLM_RESPONSE");
  return value == NULL ? "" : value;
}

ACTRAIL_PATCHABLE_ENTRY PRInt32 PR_Write(PRFileDesc *fd,
                                         const void *buf,
                                         PRInt32 amount) {
  (void)fd;
  (void)buf;
  return amount > 0 ? amount : -1;
}

ACTRAIL_PATCHABLE_ENTRY PRInt32 PR_Send(PRFileDesc *fd,
                                        const void *buf,
                                        PRInt32 amount,
                                        PRIntn flags,
                                        PRIntervalTime timeout) {
  (void)fd;
  (void)buf;
  (void)flags;
  (void)timeout;
  return amount > 0 ? amount : -1;
}

ACTRAIL_PATCHABLE_ENTRY PRInt32 PR_Read(PRFileDesc *fd,
                                        void *buf,
                                        PRInt32 amount) {
  (void)fd;
  const char *response = response_payload();
  size_t response_len = strlen(response);
  if (buf == NULL || amount <= 0 || response_len == 0) {
    return 0;
  }
  size_t limit = (size_t)amount;
  size_t copied = response_len < limit ? response_len : limit;
  memcpy(buf, response, copied);
  return (PRInt32)copied;
}

ACTRAIL_PATCHABLE_ENTRY PRInt32 PR_Recv(PRFileDesc *fd,
                                        void *buf,
                                        PRInt32 amount,
                                        PRIntn flags,
                                        PRIntervalTime timeout) {
  (void)fd;
  (void)timeout;
  (void)flags;
  const char *response = response_payload();
  size_t response_len = strlen(response);
  if (buf == NULL || amount <= 0 || response_len == 0) {
    return 0;
  }
  size_t limit = (size_t)amount;
  size_t copied = response_len < limit ? response_len : limit;
  memcpy(buf, response, copied);
  return (PRInt32)copied;
}
