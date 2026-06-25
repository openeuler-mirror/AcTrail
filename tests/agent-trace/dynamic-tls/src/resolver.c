#include <dlfcn.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "post_payload_sleep.h"

typedef int (*ssl_write_fn)(void *ssl, const void *buffer, int length);
typedef int (*ssl_read_fn)(void *ssl, void *buffer, int length);

enum {
  READ_BUFFER_BYTES = 512,
};

int main(int argc, char **argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: %s LIBSSL PAYLOAD\n", argv[0]);
    return 2;
  }
  void *handle = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
  if (handle == NULL) {
    fprintf(stderr, "dlopen failed: %s\n", dlerror());
    return 3;
  }
  ssl_write_fn ssl_write = (ssl_write_fn)dlsym(handle, "SSL_write");
  ssl_read_fn ssl_read = (ssl_read_fn)dlsym(handle, "SSL_read");
  if (ssl_write == NULL || ssl_read == NULL) {
    fprintf(stderr, "dlsym failed: %s\n", dlerror());
    return 4;
  }
  const char *payload = argv[2];
  size_t payload_len = strlen(payload);
  int written = ssl_write((void *)handle, payload, (int)payload_len);
  if (written != (int)payload_len) {
    fprintf(stderr, "SSL_write returned %d for %zu bytes\n", written, payload_len);
    return 5;
  }
  char reply[READ_BUFFER_BYTES];
  int read = ssl_read((void *)handle, reply, (int)sizeof(reply));
  if (read <= 0) {
    fprintf(stderr, "SSL_read returned %d\n", read);
    return 6;
  }
  printf("dynamic-dlsym-reply=%.*s\n", read, reply);
  if (actrail_sleep_after_payload() != 0) {
    return 7;
  }
  dlclose(handle);
  return 0;
}
