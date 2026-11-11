"""Delete actual files through absolute paths and retained directory FDs."""

import os


class RemovalOperations:
    def __init__(self, root):
        self.root = root
        self.directories = {}
        try:
            for role in ("protected", "control"):
                self.directories[role] = os.open(root / f"remove-{role}", os.O_RDONLY | os.O_DIRECTORY)
        except BaseException:
            self.close()
            raise

    def run(self, results):
        for role, directory in self.directories.items():
            for mode in ("absolute", "relative"):
                try:
                    if mode == "absolute":
                        os.unlink(self.root / f"remove-{role}" / mode)
                    else:
                        os.unlink(mode, dir_fd=directory)
                    results[f"{role}-unlink-{mode}"] = {"allowed": True}
                except OSError as error:
                    results[f"{role}-unlink-{mode}"] = {"allowed": False, "errno": error.errno}

    def close(self):
        for directory in self.directories.values():
            os.close(directory)
        self.directories.clear()
