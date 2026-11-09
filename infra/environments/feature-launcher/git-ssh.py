"""Transport Git SSH reads from a Feature worker to its own Factory relay."""
import os
import select
import socket
import sys


def main(arguments):
    if len(arguments) != 2 or arguments[0] != "git@github.com":
        return 1
    command = arguments[1].encode()
    if len(command) > 256 or b"\r" in command or b"\n" in command:
        return 1
    if not command.startswith(b"git-upload-pack "):
        print("Factory Feature Git SSH supports repository reads only.", file=sys.stderr)
        return 1
    with socket.socket(socket.AF_UNIX) as relay:
        relay.connect("/run/factory-relay/relay.sock")
        relay.sendall(b"FACTORY-GIT " + command + b"\r\n")
        sources = [relay, sys.stdin]
        while True:
            readable, _, _ = select.select(sources, [], [])
            if relay in readable:
                data = relay.recv(65536)
                if not data:
                    return 0
                sys.stdout.buffer.write(data)
                sys.stdout.buffer.flush()
            if sys.stdin in readable:
                data = os.read(sys.stdin.fileno(), 65536)
                if data:
                    relay.sendall(data)
                else:
                    relay.shutdown(socket.SHUT_WR)
                    sources.remove(sys.stdin)


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except OSError:
        sys.exit(1)
