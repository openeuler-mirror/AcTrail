#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "post_payload_sleep.h"
#include "workload_exit.h"

int SSL_write(void *ssl, const void *buffer, int length);
int SSL_write_ex2(void *ssl, const void *buffer, size_t length, uint64_t flags,
                  size_t *written);
int SSL_read(void *ssl, void *buffer, int length);

enum {
  READ_BUFFER_BYTES = 512,
};

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
  size_t written_ex2 = 0;
  int ok = SSL_write_ex2((void *)payload, payload, payload_len, 0, &written_ex2);
  if (ok != 1 || written_ex2 != payload_len) {
    fprintf(stderr, "SSL_write_ex2 returned %d for %zu of %zu bytes\n", ok, written_ex2,
            payload_len);
    return DYN_TLS_EXIT_SSL_WRITE_EX2;
  }
  char reply[READ_BUFFER_BYTES];
  int read = SSL_read((void *)payload, reply, (int)sizeof(reply));
  if (read <= 0) {
    fprintf(stderr, "SSL_read returned %d\n", read);
    return DYN_TLS_EXIT_SSL_READ;
  }
  printf("dynamic-needed-reply=%.*s\n", read, reply);
  if (actrail_sleep_after_payload() != 0) {
    return DYN_TLS_EXIT_POST_PAYLOAD_SLEEP;
  }
  return EXIT_SUCCESS;
}
