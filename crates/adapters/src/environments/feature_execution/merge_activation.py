"""Start the same guarded provider without parsing possibly conflicted manifests."""

import os
import sys


def run(access, command):
    if access == "write" and command:
        try:
            print('{"type":"factory_repository_ready"}', flush=True)
            os.execve(command[0], command, os.environ)
        except OSError:
            pass
    print('{"type":"factory_repository_failed","step":"configuration"}', flush=True)
    return 1


if __name__ == "__main__":
    sys.exit(run(sys.argv[1], sys.argv[2:]))
