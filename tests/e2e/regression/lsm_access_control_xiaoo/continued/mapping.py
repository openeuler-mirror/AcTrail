"""Issue real file-backed mmap/mprotect calls using retained descriptors."""

import ctypes
import mmap
import os


class MappingOperations:
    def __init__(self, descriptors):
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int,
                                  ctypes.c_int, ctypes.c_int, ctypes.c_long]
        self.libc.mmap.restype = ctypes.c_void_p
        self.libc.mprotect.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int]
        self.libc.mprotect.restype = ctypes.c_int
        self.libc.munmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        self.libc.munmap.restype = ctypes.c_int
        self.size = mmap.PAGESIZE
        self.descriptors = descriptors
        self.retained = {}
        try:
            for name, descriptor in descriptors.items():
                self.retained[name] = self.map(descriptor, mmap.PROT_READ)
        except BaseException:
            self.close()
            raise

    def map(self, descriptor, protection):
        address = self.libc.mmap(None, self.size, protection, mmap.MAP_SHARED, descriptor, 0)
        if address == ctypes.c_void_p(-1).value:
            self.raise_errno()
        return address

    def protect(self, address, protection):
        if self.libc.mprotect(address, self.size, protection) != 0:
            self.raise_errno()

    def run(self, operations):
        for name, descriptor in self.descriptors.items():
            for label, protection in (("read", mmap.PROT_READ),
                                      ("write", mmap.PROT_READ | mmap.PROT_WRITE),
                                      ("execute", mmap.PROT_EXEC)):
                for operation in ("mmap", "mprotect"):
                    key = f"{name}-{operation}-{label}"
                    if operation == "mprotect":
                        # Removing access is required independently of the later result.
                        self.protect(self.retained[name], 0)
                    try:
                        if operation == "mmap":
                            address = self.map(descriptor, protection)
                            self.libc.munmap(address, self.size)
                        else:
                            address = self.retained[name]
                            self.protect(address, protection)
                        operations[key] = {"allowed": True}
                    except OSError as error:
                        operations[key] = {"allowed": False, "errno": error.errno}
        # Anonymous mappings must stay independent of path policy.
        with mmap.mmap(-1, self.size) as anonymous:
            anonymous[0:1] = b"a"
            assert anonymous[0:1] == b"a"
        address = self.libc.mmap(None, self.size, mmap.PROT_READ,
                                 mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS, -1, 0)
        if address == ctypes.c_void_p(-1).value:
            self.raise_errno()
        try:
            self.protect(address, mmap.PROT_READ | mmap.PROT_WRITE)
        finally:
            self.libc.munmap(address, self.size)

    def close(self):
        for address in self.retained.values():
            self.libc.munmap(address, self.size)
        self.retained.clear()

    @staticmethod
    def raise_errno():
        code = ctypes.get_errno()
        raise OSError(code, os.strerror(code))
