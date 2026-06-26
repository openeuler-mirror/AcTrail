#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "post_payload_sleep.h"

int SSL_write(void *ssl, const void *buffer, int length);
int SSL_read(void *ssl, void *buffer, int length);

enum {
  READ_BUFFER_BYTES = 512,
};

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s PAYLOAD\n", argv[0]);
    return 2;
  }
  const char *payload = argv[1];
  size_t payload_len = strlen(payload);
  int written = SSL_write((void *)payload, payload, (int)payload_len);
  if (written != (int)payload_len) {
    fprintf(stderr, "SSL_write returned %d for %zu bytes\n", written, payload_len);
    return 3;
  }
  char reply[READ_BUFFER_BYTES];
  int read = SSL_read((void *)payload, reply, (int)sizeof(reply));
  if (read <= 0) {
    fprintf(stderr, "SSL_read returned %d\n", read);
    return 4;
  }
  printf("dynamic-needed-reply=%.*s\n", read, reply);
  if (actrail_sleep_after_payload() != 0) {
    return 5;
  }
  return 0;
}
