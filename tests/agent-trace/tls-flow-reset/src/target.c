#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "post_payload_sleep.h"
#include "workload_exit.h"

int SSL_write(void *ssl, const void *buffer, int length);
int SSL_read(void *ssl, void *buffer, int length);

enum {
  READ_BUFFER_BYTES = 1024,
};

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s PAYLOAD\n", argv[0]);
    return DYN_TLS_EXIT_USAGE;
  }
  char ssl_slot = 0;
  void *ssl = &ssl_slot;
  const char *payload = argv[1];
  size_t payload_len = strlen(payload);
  int written = SSL_write(ssl, payload, (int)payload_len);
  if (written != (int)payload_len) {
    fprintf(stderr, "SSL_write returned %d for %zu bytes\n", written, payload_len);
    return DYN_TLS_EXIT_SSL_WRITE;
  }
  char first[READ_BUFFER_BYTES];
  int first_read = SSL_read(ssl, first, (int)sizeof(first));
  if (first_read <= 0) {
    fprintf(stderr, "first SSL_read returned %d\n", first_read);
    return DYN_TLS_EXIT_SSL_READ;
  }
  char second[READ_BUFFER_BYTES];
  int second_read = SSL_read(ssl, second, (int)sizeof(second));
  if (second_read <= 0) {
    fprintf(stderr, "second SSL_read returned %d\n", second_read);
    return DYN_TLS_EXIT_SSL_READ;
  }
  printf("tls-flow-reset-first-read=%d\n", first_read);
  printf("tls-flow-reset-second=%.*s\n", second_read, second);
  if (actrail_sleep_after_payload() != 0) {
    return DYN_TLS_EXIT_POST_PAYLOAD_SLEEP;
  }
  return EXIT_SUCCESS;
}
