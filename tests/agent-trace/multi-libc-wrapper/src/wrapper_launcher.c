#include <errno.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

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
    fprintf(stderr, "helper command failed: %s status=%d\n", command, status);
    return 1;
  }
  return 0;
}

static const char *required_env(const char *name) {
  const char *value = getenv(name);
  if (value == NULL || value[0] == '\0') {
    fprintf(stderr, "missing %s\n", name);
    exit(2);
  }
  return value;
}

static char *join3(const char *first, const char *separator,
                   const char *second) {
  size_t len = strlen(first) + strlen(separator) + strlen(second) + 1;
  char *value = malloc(len);
  if (value == NULL) {
    perror("malloc");
    exit(2);
  }
  snprintf(value, len, "%s%s%s", first, separator, second);
  return value;
}

static char *shell_quote(const char *value) {
  size_t len = 3;
  for (const char *cursor = value; *cursor != '\0'; cursor++) {
    len += (*cursor == '\'') ? 4 : 1;
  }
  char *quoted = malloc(len);
  if (quoted == NULL) {
    perror("malloc");
    exit(2);
  }
  char *out = quoted;
  *out++ = '\'';
  for (const char *cursor = value; *cursor != '\0'; cursor++) {
    if (*cursor == '\'') {
      memcpy(out, "'\\''", 4);
      out += 4;
    } else {
      *out++ = *cursor;
    }
  }
  *out++ = '\'';
  *out = '\0';
  return quoted;
}

static int run_unary_helper(const char *prefix, const char *path) {
  char *quoted = shell_quote(path);
  char *command = join3(prefix, "", quoted);
  int result = run_helper(command);
  free(quoted);
  free(command);
  return result;
}

static int run_copy_helper(const char *source_dir, const char *runtime_dir,
                           const char *file_name) {
  char *source_path = join3(source_dir, "/", file_name);
  char *target_path = join3(runtime_dir, "/", file_name);
  char *quoted_source = shell_quote(source_path);
  char *quoted_target = shell_quote(target_path);
  char *command = join3("cp -n ", quoted_source, " ");
  char *command_with_target = join3(command, "", quoted_target);
  int result = run_helper(command_with_target);
  free(source_path);
  free(target_path);
  free(quoted_source);
  free(quoted_target);
  free(command);
  free(command_with_target);
  return result;
}

int main(int argc, char **argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: %s TARGET PAYLOAD\n", argv[0]);
    return 2;
  }
  const char *source_dir = required_env("ACTRAIL_MULTI_LIBC_MUSL_SOURCE_DIR");
  const char *runtime_dir = required_env("ACTRAIL_MULTI_LIBC_MUSL_RUNTIME_DIR");
  if (setenv("LD_LIBRARY_PATH", source_dir, 1) != 0) {
    perror("setenv LD_LIBRARY_PATH");
    return 2;
  }
  if (run_helper("env >/dev/null") != 0 ||
      run_unary_helper("mkdir -p ", runtime_dir) != 0 ||
      run_unary_helper("chmod 777 ", runtime_dir) != 0 ||
      run_copy_helper(source_dir, runtime_dir, "ld-musl-x86_64.so.1") != 0 ||
      run_copy_helper(source_dir, runtime_dir, "libc.musl-x86_64.so.1") != 0 ||
      run_copy_helper(source_dir, runtime_dir, "libc.so") != 0 ||
      run_copy_helper(source_dir, runtime_dir, "libssl.so") != 0) {
    return 3;
  }
  if (setenv("LD_LIBRARY_PATH", runtime_dir, 1) != 0) {
    perror("setenv target LD_LIBRARY_PATH");
    return 2;
  }
  printf("multi_libc_wrapper_helpers=7\n");
  fflush(stdout);
  char *const target_argv[] = {argv[1], argv[2], NULL};
  execv(argv[1], target_argv);
  perror("execv target");
  return 4;
}
