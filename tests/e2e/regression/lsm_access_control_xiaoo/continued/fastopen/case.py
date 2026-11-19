"""Execute Fast Open acceptance in a child network namespace."""
import ctypes
import errno
import multiprocessing
import os
import signal
import shutil
import subprocess
from pathlib import Path

from tests.e2e.common.core import TestRunDirectories, CommonTestConfig, TestCase, TestResult, TestStatus


class LsmFastOpenCase(TestCase):
    case_id = "lsm_access_control_xiaoo_fastopen"
    description = "Verify real-agent Fast Open enforcement in a temporary network namespace"
    default_enabled = False
    CLONE_NEWNET = 0x40000000

    def __init__(self, directories: TestRunDirectories):
        self.directories = directories
        self.config = CommonTestConfig.from_environment(directories, "LSM_FASTOPEN")

    def run(self, context):
        if os.geteuid() != 0 or not all(shutil.which(name) for name in ('ip', 'sysctl')):
            return TestResult(TestStatus.SKIPPED, 'Root, ip and sysctl are required')
        if not (os.environ.get('LSM_ACCESS_CONTROL_XIAOO_E2E_BINARY') or shutil.which('xiaoo')):
            return TestResult(TestStatus.SKIPPED, 'Real Xiaoo is required')
        # Fork keeps the runner context while isolating namespace and environment changes.
        fork = multiprocessing.get_context("fork")
        receiver, sender = fork.Pipe(duplex=False)
        process = fork.Process(target=self._execute, args=(context, sender))
        process.start()
        sender.close()
        try:
            if not receiver.poll(self.config.launch_timeout_seconds):
                os.killpg(process.pid, signal.SIGKILL)
                return TestResult(TestStatus.FAILED, "Fast Open acceptance timed out")
            try:
                return receiver.recv()
            except EOFError:
                return TestResult(TestStatus.FAILED, "Fast Open worker exited without a result")
        finally:
            process.join()
            receiver.close()

    def _execute(self, context, sender):
        from ...case import LsmAccessControlXiaooCase

        os.setsid()
        case = None
        try:
            namespace = Path("/proc/self/ns/net")
            original = namespace.stat()
            libc = ctypes.CDLL(None, use_errno=True)
            libc.unshare.argtypes = [ctypes.c_int]
            libc.unshare.restype = ctypes.c_int
            if libc.unshare(self.CLONE_NEWNET) != 0:
                code = ctypes.get_errno()
                if code in (errno.EPERM, errno.EACCES, errno.ENOSYS):
                    sender.send(TestResult(TestStatus.SKIPPED, f"Network namespace unavailable: {os.strerror(code)}"))
                    sender.close()
                    return
                raise OSError(code, os.strerror(code))
            current = namespace.stat()
            if (current.st_dev, current.st_ino) == (original.st_dev, original.st_ino):
                raise RuntimeError("Fast Open requires an isolated network namespace")
            subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
            subprocess.run(["sysctl", "-w", "net.ipv4.tcp_fastopen=3"], check=True)
            os.environ.update(LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS="lsm",
                              LSM_ACCESS_CONTROL_XIAOO_E2E_CONTINUED="1",
                              LSM_ACCESS_CONTROL_XIAOO_E2E_FASTOPEN="1")
            case = LsmAccessControlXiaooCase(self.directories)
            result = case.run(context)
        except Exception as error:
            result = TestResult(TestStatus.FAILED, str(error))
        finally:
            if case is not None:
                cleanup = case.cleanup(context)
                if cleanup is not None and cleanup.has_failure():
                    result = cleanup
        sender.send(result)
        sender.close()
