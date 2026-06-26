#ifndef ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_H
#define ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_H

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

static int actrail_sleep_after_payload(void) {
  const char *raw = getenv("ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_MS");
  if (raw == NULL || raw[0] == '\0') {
    return 0;
  }
  errno = 0;
  char *end = NULL;
  long millis = strtol(raw, &end, 10);
  if (errno != 0 || end == raw || *end != '\0' || millis < 0) {
    fprintf(stderr,
            "invalid ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_MS=%s\n",
            raw);
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

#endif
