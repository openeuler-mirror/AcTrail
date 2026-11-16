/* Bounded synchronous userspace decisions for sleepable BPF LSM hooks. */
#include <linux/bpf.h>
#include <linux/btf.h>
#include <linux/btf_ids.h>
#include <linux/fs.h>
#include <linux/ktime.h>
#include <linux/list.h>
#include <linux/miscdevice.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/poll.h>
#include <linux/slab.h>
#include <linux/uaccess.h>
#include <linux/wait.h>
#include "protocol.h"
#include "exec_context.h"

static unsigned int max_pending = 64;
static unsigned int max_wait_ms = 30000;
module_param(max_pending, uint, 0444);
module_param(max_wait_ms, uint, 0444);
MODULE_PARM_DESC(max_pending, "Maximum simultaneous grey requests");
MODULE_PARM_DESC(max_wait_ms, "Maximum time a grey request can wait in milliseconds");

struct grey_pending {
    struct list_head link;
    struct actrail_lsm_grey_message message;
    wait_queue_head_t wait;
    bool delivered;
    bool done;
    int decision;
};

static LIST_HEAD(requests);
static DEFINE_MUTEX(requests_lock);
static DECLARE_WAIT_QUEUE_HEAD(reader_wait);
static unsigned int pending_count;
static atomic_t unread_count = ATOMIC_INIT(0);
static bool connected;
static u64 serial;

static int grey_decide(struct linux_binprm *bprm, void *data, u32 data__sz)
{
    const struct actrail_lsm_grey_request *input = data;
    struct grey_pending *entry, *other;
    unsigned int same_rule = 0;
    long waited;
    int result;

    if (data__sz != sizeof(*input) || !input->timeout_ms || !input->concurrency_limit)
        return -EINVAL;
    /* Only grey allocates. NOFS avoids reclaim re-entering a held filesystem lock. */
    entry = kzalloc(sizeof(*entry), GFP_NOFS);
    if (!entry)
        return -ENOMEM;
    /* The BPF caller provides task-local storage, stable across allocation. */
    memcpy(&entry->message.request, input, sizeof(*input));
    if (entry->message.request.event.domain == 4) {
        result = bprm ? grey_capture_exec(bprm, &entry->message.command) : -EINVAL;
        if (result) {
            kfree(entry);
            return AC_GREY_CONTEXT_INVALID;
        }
    }
    init_waitqueue_head(&entry->wait);
    mutex_lock(&requests_lock);
    if (!connected) {
        result = -ENOTCONN;
        goto reject;
    }
    if (pending_count >= max_pending) {
        result = -EBUSY;
        goto reject;
    }
    list_for_each_entry(other, &requests, link) {
        if (!memcmp(other->message.request.target, entry->message.request.target, sizeof(entry->message.request.target)) &&
            !memcmp(other->message.request.event.owner, entry->message.request.event.owner, sizeof(entry->message.request.event.owner)) &&
            !memcmp(other->message.request.event.rule_id, entry->message.request.event.rule_id, sizeof(entry->message.request.event.rule_id)))
            same_rule++;
    }
    if (same_rule >= entry->message.request.concurrency_limit) {
        result = -EBUSY;
        goto reject;
    }
    entry->message.request.request_id = ++serial;
    entry->message.request.timeout_ms = min(entry->message.request.timeout_ms, max_wait_ms);
    entry->message.request.deadline_boottime_ns = ktime_get_boottime_ns() +
        (u64)entry->message.request.timeout_ms * NSEC_PER_MSEC;
    list_add_tail(&entry->link, &requests);
    pending_count++;
    atomic_inc(&unread_count);
    mutex_unlock(&requests_lock);
    wake_up_interruptible(&reader_wait);

    /* Preserve the original syscall stack and release all broker locks while waiting. */
    waited = wait_event_interruptible_timeout(entry->wait, READ_ONCE(entry->done),
        msecs_to_jiffies(entry->message.request.timeout_ms));
    mutex_lock(&requests_lock);
    result = waited > 0 ? entry->decision : (waited == 0 ? -ETIMEDOUT : -EINTR);
    if (!entry->delivered)
        atomic_dec(&unread_count);
    list_del(&entry->link);
    pending_count--;
    mutex_unlock(&requests_lock);
    kfree(entry);
    return result;
reject:
    mutex_unlock(&requests_lock);
    kfree(entry);
    return result;
}
__bpf_kfunc_start_defs();
__bpf_kfunc int actrail_lsm_decide(void *data, u32 data__sz)
{
    return grey_decide(NULL, data, data__sz);
}

__bpf_kfunc int actrail_lsm_exec_decide(struct linux_binprm *bprm,
                                      void *data, u32 data__sz)
{
    return grey_decide(bprm, data, data__sz);
}
__bpf_kfunc_end_defs();

static int grey_open(struct inode *inode, struct file *file)
{
    int result = 0;
    mutex_lock(&requests_lock);
    if (connected)
        result = -EBUSY;
    else
        connected = true;
    mutex_unlock(&requests_lock);
    return result;
}

static int grey_release(struct inode *inode, struct file *file)
{
    struct grey_pending *entry;
    mutex_lock(&requests_lock);
    connected = false;
    list_for_each_entry(entry, &requests, link) {
        if (entry->done)
            continue;
        if (!entry->delivered) {
            entry->delivered = true;
            atomic_dec(&unread_count);
        }
        entry->decision = -ENOTCONN;
        WRITE_ONCE(entry->done, true);
        wake_up_interruptible(&entry->wait);
    }
    mutex_unlock(&requests_lock);
    return 0;
}

static ssize_t grey_read(struct file *file, char __user *buffer, size_t length, loff_t *offset)
{
    struct actrail_lsm_grey_message *message;
    struct grey_pending *entry;
    bool found = false;
    int result;
    if (length != sizeof(*message))
        return -EINVAL;
    if (!(file->f_flags & O_NONBLOCK)) {
        result = wait_event_interruptible(reader_wait, atomic_read(&unread_count) > 0);
        if (result)
            return result;
    }
    /* Keep command payloads off the kernel stack and copy to userspace unlocked. */
    message = kmalloc(sizeof(*message), GFP_KERNEL);
    if (!message)
        return -ENOMEM;
    mutex_lock(&requests_lock);
    list_for_each_entry(entry, &requests, link) {
        if (entry->delivered || entry->done)
            continue;
        *message = entry->message;
        entry->delivered = true;
        atomic_dec(&unread_count);
        found = true;
        break;
    }
    mutex_unlock(&requests_lock);
    result = !found ? -EAGAIN :
        (copy_to_user(buffer, message, sizeof(*message)) ? -EFAULT : sizeof(*message));
    kfree(message);
    return result;
}

static ssize_t grey_write(struct file *file, const char __user *buffer,
                          size_t length, loff_t *offset)
{
    struct actrail_lsm_grey_reply reply;
    struct grey_pending *entry;
    int result = -ENOENT;
    if (length != sizeof(reply))
        return -EINVAL;
    if (copy_from_user(&reply, buffer, sizeof(reply)))
        return -EFAULT;
    if (reply.decision > 1)
        return -EINVAL;
    mutex_lock(&requests_lock);
    list_for_each_entry(entry, &requests, link) {
        if (entry->message.request.request_id != reply.request_id || entry->done || !entry->delivered)
            continue;
        if (ktime_get_boottime_ns() >= entry->message.request.deadline_boottime_ns)
            break;
        entry->decision = reply.decision;
        WRITE_ONCE(entry->done, true);
        wake_up_interruptible(&entry->wait);
        result = sizeof(reply);
        break;
    }
    mutex_unlock(&requests_lock);
    return result;
}

static __poll_t grey_poll(struct file *file, poll_table *wait)
{
    poll_wait(file, &reader_wait, wait);
    return atomic_read(&unread_count) > 0 ? EPOLLIN | EPOLLRDNORM : 0;
}

static const struct file_operations grey_ops = {
    .owner = THIS_MODULE,
    .open = grey_open,
    .release = grey_release,
    .read = grey_read,
    .write = grey_write,
    .poll = grey_poll,
    .llseek = no_llseek,
};
static struct miscdevice grey_device = {
    .minor = MISC_DYNAMIC_MINOR,
    .name = "actrail_lsm_grey",
    .fops = &grey_ops,
    .mode = 0600,
};
BTF_SET8_START(grey_kfunc_ids)
BTF_ID_FLAGS(func, actrail_lsm_decide, KF_SLEEPABLE)
BTF_ID_FLAGS(func, actrail_lsm_exec_decide, KF_SLEEPABLE)
BTF_SET8_END(grey_kfunc_ids)
static const struct btf_kfunc_id_set grey_kfunc_set = {
    .owner = THIS_MODULE,
    .set = &grey_kfunc_ids,
};

static int __init grey_init(void)
{
    int result;
    if (!max_pending || !max_wait_ms || !argv_max_count ||
        argv_max_count > AC_EXEC_ARG_COUNT || argv_arg_max_bytes < 2 ||
        argv_arg_max_bytes > AC_EXEC_ARG_BYTES)
        return -EINVAL;
    result = misc_register(&grey_device);
    if (result)
        return result;
    result = register_btf_kfunc_id_set(BPF_PROG_TYPE_LSM, &grey_kfunc_set);
    if (result)
        misc_deregister(&grey_device);
    return result;
}

static void __exit grey_exit(void)
{
    misc_deregister(&grey_device);
}
module_init(grey_init);
module_exit(grey_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("AcTrail sleepable LSM userspace decision channel");
