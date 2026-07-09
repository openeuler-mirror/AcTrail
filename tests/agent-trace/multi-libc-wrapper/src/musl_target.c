#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int SSL_write(void *ssl, const void *buffer, int length);
int SSL_write_ex2(void *ssl, const void *buffer, size_t length,
                  unsigned long long flags, size_t *written);
int SSL_read(void *ssl, void *buffer, int length);

enum {
  READ_BUFFER_BYTES = 512,
};

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s PAYLOAD\n", argv[0]);
    return 2;
  }
  const char *preload = getenv("LD_PRELOAD");
  const char *library_path = getenv("LD_LIBRARY_PATH");
  printf("multi_libc_target_preload=%s\n", preload == NULL ? "" : preload);
  printf("multi_libc_target_ld_library_path=%s\n",
         library_path == NULL ? "" : library_path);
  if (preload == NULL ||
      strstr(preload, "libactrail_tls_payload_probe_sync-musl.so") == NULL) {
    fprintf(stderr, "musl target did not receive musl TLS sync runtime\n");
    return 11;
  }
  if (library_path != NULL &&
      strstr(library_path, "actrail-tls-runtime-deps") != NULL) {
    fprintf(stderr, "musl target received glibc dependency guard\n");
    return 12;
  }
  const char *payload = argv[1];
  size_t payload_len = strlen(payload);
  int written = SSL_write((void *)payload, payload, (int)payload_len);
  if (written != (int)payload_len) {
    fprintf(stderr, "SSL_write returned %d for %zu bytes\n", written,
            payload_len);
    return 13;
  }
  size_t written_ex2 = 0;
  int ok = SSL_write_ex2((void *)payload, payload, payload_len, 0, &written_ex2);
  if (ok != 1 || written_ex2 != payload_len) {
    fprintf(stderr, "SSL_write_ex2 returned %d for %zu of %zu bytes\n", ok,
            written_ex2, payload_len);
    return 14;
  }
  char reply[READ_BUFFER_BYTES];
  int read = SSL_read((void *)payload, reply, (int)sizeof(reply));
  if (read <= 0) {
    fprintf(stderr, "SSL_read returned %d\n", read);
    return 15;
  }
  printf("multi_libc_target_runtime=musl\n");
  printf("multi-libc-target-reply=%.*s\n", read, reply);
  return 0;
}
