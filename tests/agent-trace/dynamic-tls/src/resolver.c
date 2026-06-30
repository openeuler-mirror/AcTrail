#include <dlfcn.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "post_payload_sleep.h"
#include "workload_exit.h"

typedef int (*ssl_write_fn)(void *ssl, const void *buffer, int length);
typedef int (*ssl_write_ex2_fn)(void *ssl, const void *buffer, size_t length, uint64_t flags,
                                size_t *written);
typedef int (*ssl_read_fn)(void *ssl, void *buffer, int length);

enum {
  READ_BUFFER_BYTES = 512,
};

int main(int argc, char **argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: %s LIBSSL PAYLOAD\n", argv[0]);
    return DYN_TLS_EXIT_USAGE;
  }
  void *handle = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
  if (handle == NULL) {
    fprintf(stderr, "dlopen failed: %s\n", dlerror());
    return DYN_TLS_EXIT_DLOPEN;
  }
  ssl_write_fn ssl_write = (ssl_write_fn)dlsym(handle, "SSL_write");
  ssl_write_ex2_fn ssl_write_ex2 = (ssl_write_ex2_fn)dlsym(handle, "SSL_write_ex2");
  ssl_read_fn ssl_read = (ssl_read_fn)dlsym(handle, "SSL_read");
  if (ssl_write == NULL || ssl_write_ex2 == NULL || ssl_read == NULL) {
    fprintf(stderr, "dlsym failed: %s\n", dlerror());
    return DYN_TLS_EXIT_DLSYM;
  }
  const char *payload = argv[2];
  size_t payload_len = strlen(payload);
  int written = ssl_write((void *)handle, payload, (int)payload_len);
  if (written != (int)payload_len) {
    fprintf(stderr, "SSL_write returned %d for %zu bytes\n", written, payload_len);
    return DYN_TLS_EXIT_SSL_WRITE;
  }
  size_t written_ex2 = 0;
  int ok = ssl_write_ex2((void *)handle, payload, payload_len, 0, &written_ex2);
  if (ok != 1 || written_ex2 != payload_len) {
    fprintf(stderr, "SSL_write_ex2 returned %d for %zu of %zu bytes\n", ok, written_ex2,
            payload_len);
    return DYN_TLS_EXIT_SSL_WRITE_EX2;
  }
  char reply[READ_BUFFER_BYTES];
  int read = ssl_read((void *)handle, reply, (int)sizeof(reply));
  if (read <= 0) {
    fprintf(stderr, "SSL_read returned %d\n", read);
    return DYN_TLS_EXIT_SSL_READ;
  }
  printf("dynamic-dlsym-reply=%.*s\n", read, reply);
  if (actrail_sleep_after_payload() != 0) {
    return DYN_TLS_EXIT_POST_PAYLOAD_SLEEP;
  }
  dlclose(handle);
  return EXIT_SUCCESS;
}
