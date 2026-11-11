/* Capture complete bounded arguments from the kernel's pending exec image. */
#ifndef ACTRAIL_EXEC_CONTEXT_H
#define ACTRAIL_EXEC_CONTEXT_H
#include <asm/syscall.h>
#include <linux/compat.h>
#include <linux/binfmts.h>
#include <linux/highmem.h>
#include <linux/mm.h>

static unsigned int argv_max_count = AC_EXEC_ARG_COUNT;
static unsigned int argv_arg_max_bytes = AC_EXEC_ARG_BYTES;
module_param(argv_max_count, uint, 0444);
module_param(argv_arg_max_bytes, uint, 0444);
MODULE_PARM_DESC(argv_max_count, "Maximum captured exec argument count (1..16)");
MODULE_PARM_DESC(argv_arg_max_bytes, "Maximum bytes per exec argument including NUL (2..256)");

static int grey_capture_argv(struct linux_binprm *bprm,
                             struct actrail_lsm_exec_context *context)
{
    unsigned long position = bprm->p;
    unsigned int index = 0, size = 0;

    if (!bprm->mm || bprm->argc < 0 || bprm->argc > argv_max_count)
        return -E2BIG;
    while (index < bprm->argc) {
        struct page *page;
        const char *mapped;
        unsigned int offset = offset_in_page(position);
        int locked = 1;
        long result;

        /* This mm belongs to the new image, not the caller's shared argv. */
        mmap_read_lock(bprm->mm);
        result = get_user_pages_remote(bprm->mm, position, 1, 0, &page, &locked);
        if (locked)
            mmap_read_unlock(bprm->mm);
        if (result != 1)
            return -EFAULT;
        mapped = kmap_local_page(page);
        while (offset < PAGE_SIZE && index < bprm->argc) {
            char byte = mapped[offset++];
            position++;
            context->argv[index][size++] = byte;
            if (!byte) {
                index++;
                size = 0;
            } else if (size == argv_arg_max_bytes) {
                kunmap_local(mapped);
                put_page(page);
                return -E2BIG;
            }
        }
        kunmap_local(mapped);
        put_page(page);
    }
    context->argc = index;
    return 0;
}

static int grey_capture_exec(struct linux_binprm *bprm,
                            struct actrail_lsm_exec_context *context)
{
    struct pt_regs *regs = task_pt_regs(current);
    unsigned long args[6];
    long size, nr = syscall_get_nr(current, regs);

    /* Native syscall arguments belong to this same blocked thread. */
    if (in_compat_syscall())
        return -EINVAL;
    syscall_get_arguments(current, regs, args);
    if (nr == __NR_execve) {
        context->syscall = 1;
    } else if (nr == __NR_execveat) {
        context->syscall = 2;
        context->dirfd = (int)args[0];
        context->flags = args[4];
    } else {
        return -EINVAL;
    }
    size = strscpy((char *)context->requested_path, bprm->filename,
                   sizeof(context->requested_path));
    if (size < 0)
        return -EFAULT;
    return grey_capture_argv(bprm, context);
}
#endif
