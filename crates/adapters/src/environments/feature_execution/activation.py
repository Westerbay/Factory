"""Activate the image's repository environment before the provider handshake starts."""

import errno
import importlib.util
import json
import os
import sys


def load_runtime():
    # The API embeds this bootstrap, so retained workspace images need no new CLI
    # flag or on-disk script. Only the existing, immutable image helper is loaded.
    spec = importlib.util.spec_from_file_location(
        "factory_repository_environment", "/usr/local/lib/factory/repository-environment.py"
    )
    runtime = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = runtime
    spec.loader.exec_module(runtime)
    return runtime


def run(access, command):
    runtime = load_runtime()
    try:
        if access not in {"read", "write"} or not command:
            raise ValueError("configuration")
        repository = runtime.Repository(runtime.CHECKOUT, runtime.TOOLING)
        env = runtime.activate(repository, access == "write", os.environ)
        print('{"type":"factory_repository_ready"}', flush=True)
        os.execve(command[0], command, env)
    except runtime.PreparationError as error:
        step = str(error)
        if step not in {"environment-not-prepared", "node-offline-install", "configuration"}:
            step = "configuration"
    except OSError as error:
        step = {errno.ENOSPC: "disk-space", errno.EACCES: "filesystem-permission",
                errno.ENOEXEC: "executable-format"}.get(error.errno, "configuration")
    except (KeyError, TypeError, ValueError):
        step = "configuration"
    print(json.dumps({"type": "factory_repository_failed", "step": step}), flush=True)
    return 1


if __name__ == "__main__":
    sys.exit(run(sys.argv[1], sys.argv[2:]))
