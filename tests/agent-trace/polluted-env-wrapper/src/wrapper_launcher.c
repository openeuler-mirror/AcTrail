#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <spawn.h>
#include <sys/wait.h>
#include <unistd.h>

#include "post_payload_sleep.h"
#include "workload_exit.h"

int SSL_write(void *ssl, const void *buffer, int length);
int SSL_write_ex2(void *ssl, const void *buffer, size_t length, uint64_t flags,
                  size_t *written);
int SSL_read(void *ssl, void *buffer, int length);
int actrail_private_libstdcxx_marker(void);
extern char **environ;

enum {
  READ_BUFFER_BYTES = 512,
};

static int run_helper(const char *command) {
  pid_t pid = 0;
  char *const argv[] = {"sh", "-c", (char *)command, NULL};
  int spawn_error = posix_spawn(&pid, "/bin/sh", NULL, NULL, argv, environ);
  if (spawn_error != 0) {
    errno = spawn_error;
    perror("posix_spawn helper");
    return 1;
  }
  int status = 0;
  if (waitpid(pid, &status, 0) < 0) {
    perror("waitpid helper");
    return 1;
  }
  if (status != 0) {
    fprintf(stderr, "helper failed: %s status=%d\n", command, status);
    return 1;
  }
  return 0;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s PAYLOAD\n", argv[0]);
    return DYN_TLS_EXIT_USAGE;
  }
  const char *payload = argv[1];
  const char *poison_path = getenv("ACTRAIL_POLLUTED_LIBRARY_PATH");
  if (poison_path == NULL || poison_path[0] == '\0') {
    fprintf(stderr, "ACTRAIL_POLLUTED_LIBRARY_PATH is not set\n");
    return DYN_TLS_EXIT_USAGE;
  }
  if (setenv("LD_LIBRARY_PATH", poison_path, 1) != 0) {
    perror("setenv LD_LIBRARY_PATH");
    return DYN_TLS_EXIT_USAGE;
  }
  if (actrail_private_libstdcxx_marker() != 42) {
    fprintf(stderr, "private libstdc++ marker mismatch\n");
    return DYN_TLS_EXIT_USAGE;
  }
  if (run_helper("env >/dev/null") != 0 ||
      run_helper("rm -rf target/agent-trace/polluted-env-wrapper/helper-work") != 0 ||
      run_helper("mkdir -p target/agent-trace/polluted-env-wrapper/helper-work") != 0 ||
      run_helper("chmod 700 target/agent-trace/polluted-env-wrapper/helper-work") != 0 ||
      run_helper("printf 'alpha\\n' > target/agent-trace/polluted-env-wrapper/helper-work/source.txt") != 0 ||
      run_helper("cp target/agent-trace/polluted-env-wrapper/helper-work/source.txt target/agent-trace/polluted-env-wrapper/helper-work/copy.txt") != 0 ||
      run_helper("sed -n '1p' target/agent-trace/polluted-env-wrapper/helper-work/copy.txt >/dev/null") != 0 ||
      run_helper("rm target/agent-trace/polluted-env-wrapper/helper-work/source.txt") != 0) {
    return DYN_TLS_EXIT_USAGE;
  }
  printf("polluted_env_wrapper_helpers=8\n");
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
  printf("polluted-env-wrapper-reply=%.*s\n", read, reply);
  if (actrail_sleep_after_payload() != 0) {
    return DYN_TLS_EXIT_POST_PAYLOAD_SLEEP;
  }
  return EXIT_SUCCESS;
}
