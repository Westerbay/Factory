"""Remove only ignored, untracked build directories in one stopped worker checkout."""

import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys


def rust_cache(root):
    try:
        target = os.open("target", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
    except FileNotFoundError:
        return False
    try:
        try:
            tag = os.open("CACHEDIR.TAG", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=target)
        except FileNotFoundError:
            return False
        try:
            return stat.S_ISREG(os.fstat(tag).st_mode) and os.read(tag, 128).startswith(
                b"Signature: 8a477f597d28d172789f06886806bc55"
            )
        finally:
            os.close(tag)
    finally:
        os.close(target)


def git(checkout, *args):
    # Checkout configuration is untrusted; in particular, do not execute fsmonitor.
    env = {"PATH": "/usr/bin:/bin", "HOME": "/tmp",
           "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
           "GIT_OPTIONAL_LOCKS": "0"}
    return subprocess.run(
        ["/usr/bin/git", "-c", "core.fsmonitor=false", "-C", str(checkout), *args],
        env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL, timeout=30, check=False,
    )


def cleanup(checkout):
    if not shutil.rmtree.avoids_symlink_attacks:
        raise RuntimeError("unsupported-platform")
    root = os.open(checkout, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        metadata = os.open(".git", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
        os.close(metadata)
        for name in ("target", "node_modules"):
            try:
                info = os.stat(name, dir_fd=root, follow_symlinks=False)
            except FileNotFoundError:
                continue
            if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid():
                continue
            if name == "target" and not rust_cache(root):
                continue
            if name == "node_modules":
                try:
                    package = os.stat("package.json", dir_fd=root, follow_symlinks=False)
                except FileNotFoundError:
                    continue
                if not stat.S_ISREG(package.st_mode):
                    continue
            tracked = git(checkout, "ls-files", "--error-unmatch", "--", name)
            if tracked.returncode == 0:
                continue
            if tracked.returncode != 1:
                raise RuntimeError("git-index")
            ignored = git(checkout, "check-ignore", "-q", "--", name)
            if ignored.returncode == 1:
                continue
            if ignored.returncode:
                raise RuntimeError("git-ignore")
            # rmtree uses directory descriptors and never follows links inside it.
            shutil.rmtree(name, dir_fd=root)
    finally:
        os.close(root)


if __name__ == "__main__":
    try:
        cleanup(Path("/workspace/checkout"))
    except (OSError, RuntimeError, subprocess.SubprocessError):
        print("Feature build cleanup failed", file=sys.stderr)
        sys.exit(1)
