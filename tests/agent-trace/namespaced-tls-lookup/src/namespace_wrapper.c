#define _GNU_SOURCE

#include <errno.h>
#include <sched.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

int main(int argc, char **argv) {
  if (argc != 4) {
    fprintf(stderr, "usage: %s MOUNT_DIR SOURCE_TARGET PAYLOAD\n", argv[0]);
    return 2;
  }
  const char *mount_dir = argv[1];
  const char *source_target = argv[2];
  const char *payload = argv[3];
  if (mkdir(mount_dir, 0755) != 0 && errno != EEXIST) {
    fprintf(stderr, "mkdir %s: %s\n", mount_dir, strerror(errno));
    return 3;
  }
  char target[4096];
  int written = snprintf(target, sizeof(target), "%s/agent", mount_dir);
  if (written < 0 || (size_t)written >= sizeof(target)) {
    fprintf(stderr, "target path overflow\n");
    return 4;
  }
  FILE *placeholder = fopen(target, "w");
  if (placeholder == NULL) {
    fprintf(stderr, "create placeholder %s: %s\n", target, strerror(errno));
    return 5;
  }
  fputs("not an elf\n", placeholder);
  fclose(placeholder);
  if (unshare(CLONE_NEWNS) != 0) {
    fprintf(stderr, "unshare mount namespace: %s\n", strerror(errno));
    return 6;
  }
  if (mount(NULL, "/", NULL, MS_REC | MS_PRIVATE, NULL) != 0) {
    fprintf(stderr, "mark mounts private: %s\n", strerror(errno));
    return 7;
  }
  if (mount(source_target, target, NULL, MS_BIND, NULL) != 0) {
    fprintf(stderr, "bind mount %s to %s: %s\n", source_target, target,
            strerror(errno));
    return 8;
  }
  char *const child_argv[] = {target, (char *)payload, NULL};
  execv(target, child_argv);
  fprintf(stderr, "exec %s: %s\n", target, strerror(errno));
  return 9;
}
