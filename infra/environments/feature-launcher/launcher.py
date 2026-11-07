#!/usr/bin/python3
"""Trusted launch helper invoked by Factory with fixed arguments."""
import errno
import json
import os
import re
import select
import socket
import stat
import subprocess
import sys
import threading
import time
import uuid

PROVIDERS = {
    "codex": (".codex", "auth.json", ("chatgpt.com", "api.openai.com")),
    "claude": (".claude", ".credentials.json", ("api.anthropic.com",)),
}
PROXY = ("factory-egress", 3128)
MAX_HEADERS = 8192
MAX_CREDENTIAL = 65536
HEADER = re.compile(rb"([!#$%&'*+\-.^_`|~0-9A-Za-z]+):[ \t]*([\t\x20-\x7e]*)\r\n")
DIRECTORY = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC


def connect_target(line, stream, hosts):
    """Read one bounded CONNECT header and return its canonical allowed authority."""
    target = next(
        (host + ":443" for host in hosts if line in (
            f"CONNECT {host}:443 HTTP/1.0\r\n".encode(),
            f"CONNECT {host}:443 HTTP/1.1\r\n".encode(),
        )),
        None,
    )
    if target is None:
        return None
    consumed = len(line)
    seen_host = False
    while consumed < MAX_HEADERS:
        header = stream.readline(MAX_HEADERS - consumed + 1)
        consumed += len(header)
        if consumed > MAX_HEADERS:
            return None
        if header == b"\r\n":
            return target
        match = HEADER.fullmatch(header)
        if not match:
            return None
        name, value = match.groups()
        name = name.lower()
        if name in (b"transfer-encoding", b"content-length"):
            return None
        if name == b"host":
            if seen_host or value.rstrip() != target.encode():
                return None
            seen_host = True
    return None


def tunnel(left, right):
    """Relay data until both peers close, with a bounded idle timeout."""
    pending = {left: right, right: left}
    while pending:
        readable, _, _ = select.select(list(pending), [], [], 300)
        if not readable:
            return
        for source in readable:
            destination = pending[source]
            data = source.recv(65536)
            if data:
                destination.sendall(data)
            else:
                try:
                    destination.shutdown(socket.SHUT_WR)
                except OSError as error:
                    if error.errno != errno.ENOTCONN:
                        raise
                del pending[source]


def owned_directory(parent, name, create=False, private=True):
    if create:
        try:
            os.mkdir(name, mode=0o700, dir_fd=parent)
        except FileExistsError:
            pass
    fd = os.open(name, DIRECTORY, dir_fd=parent)
    info = os.fstat(fd)
    if info.st_uid != os.geteuid() or info.st_mode & (0o077 if private else 0o022):
        os.close(fd)
        raise PermissionError("not an owned private directory")
    return fd


def access_only(provider, data):
    """Give each worker a credential for one access-token lifetime only.

    Parallel isolated copies must never rotate the authoritative account's
    refresh token. The relay also denies the provider's refresh hosts.
    """
    value = json.loads(data)
    if not isinstance(value, dict):
        raise ValueError("invalid credential")
    if provider == "codex":
        tokens = value.get("tokens")
        if not isinstance(tokens, dict) or not all(
            isinstance(tokens.get(key), str) and tokens[key] for key in ("access_token", "id_token")
        ):
            raise ValueError("connected ChatGPT access token unavailable")
        seed = {
            "auth_mode": "chatgpt",
            "last_refresh": value["last_refresh"],
            "tokens": {
                "access_token": tokens["access_token"],
                "id_token": tokens["id_token"],
                "account_id": tokens.get("account_id"),
                # The pinned Codex parser requires this string. It is unusable.
                "refresh_token": "",
            },
        }
    elif provider == "claude":
        tokens = value.get("claudeAiOauth")
        if not isinstance(tokens, dict) or not isinstance(tokens.get("accessToken"), str):
            raise ValueError("connected Claude access token unavailable")
        seed = {"claudeAiOauth": {
            key: tokens[key] for key in (
                "accessToken", "expiresAt", "scopes", "subscriptionType", "rateLimitTier"
            ) if key in tokens
        }}
    else:
        raise ValueError("unknown provider")
    return json.dumps(seed, separators=(",", ":")).encode()


def credential(home, provider):
    directory, filename, _ = PROVIDERS[provider]
    fd = owned_directory(home, directory, private=False)
    try:
        file = os.open(
            filename, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=fd
        )
        with os.fdopen(file, "rb") as stream:
            info = os.fstat(stream.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                    or info.st_mode & 0o077):
                raise PermissionError("not an owned private credential file")
            data = stream.read(MAX_CREDENTIAL + 1)
            if not 0 < len(data) <= MAX_CREDENTIAL:
                raise ValueError("invalid credential file size")
            return access_only(provider, data)
    finally:
        os.close(fd)


def establish(upstream, authority):
    request = f"CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n"
    upstream.sendall(request.encode())
    with upstream.makefile("rb", buffering=0) as reply:
        status = reply.readline(512)
        if not re.fullmatch(rb"HTTP/1\.[01] 200 [\x20-\x7e]*\r\n", status):
            return False
        consumed = len(status)
        while consumed < MAX_HEADERS:
            line = reply.readline(MAX_HEADERS - consumed + 1)
            consumed += len(line)
            if not line or consumed > MAX_HEADERS:
                return False
            if line == b"\r\n":
                return True
    return False


def serve(listener, data, hosts):
    slots = threading.BoundedSemaphore(16)
    auth_lock = threading.Lock()
    auth_served = threading.Event()

    def relay(connection):
        try:
            with connection, connection.makefile("rb", buffering=0) as request:
                connection.settimeout(10)
                line = request.readline(512)
                if line == b"FACTORY-AUTH\r\n":
                    with auth_lock:
                        value = bytes(data)
                        data.clear()
                        auth_served.set()
                    connection.sendall(len(value).to_bytes(4, "big") + value)
                    return
                authority = connect_target(line, request, hosts)
                if authority is None:
                    connection.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                    return
                with socket.create_connection(PROXY, timeout=10) as upstream:
                    if not establish(upstream, authority):
                        connection.sendall(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                        return
                    connection.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    connection.settimeout(20)
                    upstream.settimeout(20)
                    tunnel(connection, upstream)
        except OSError:
            # No proxy requests, credentials, or provider responses are written to logs.
            pass
        finally:
            slots.release()

    expires = time.monotonic() + 86400
    auth_expires = time.monotonic() + 35
    while time.monotonic() < expires:
        if not auth_served.is_set() and time.monotonic() >= auth_expires:
            return
        readable, _, _ = select.select([listener, sys.stdin], [], [], 1)
        if not readable:
            continue
        if sys.stdin in readable:
            return
        connection, _ = listener.accept()
        if not slots.acquire(blocking=False):
            connection.close()
            continue
        threading.Thread(target=relay, args=(connection,), daemon=True).start()


def relay_main(attempt, provider):
    if str(uuid.UUID(attempt)) != attempt or uuid.UUID(attempt).int == 0:
        return 1
    with socket.socket(socket.AF_UNIX) as listener:
        home = os.open("/home/factory", DIRECTORY)
        try:
            data = bytearray(credential(home, provider))
            factory = owned_directory(home, ".factory", create=True)
            try:
                relays = owned_directory(factory, "feature-relays", create=True)
            finally:
                os.close(factory)
        finally:
            os.close(home)
        try:
            os.mkdir(attempt, mode=0o700, dir_fd=relays)
            private = owned_directory(relays, attempt)
            try:
                # Short and anchored to the opened directory even if its name is swapped.
                listener.bind(f"/proc/self/fd/{private}/relay.sock")
                listener.listen(16)
                info = os.fstat(private)
                print(f"ready {info.st_dev}:{info.st_ino}", flush=True)
                serve(listener, data, PROVIDERS[provider][2])
            finally:
                try:
                    os.unlink("relay.sock", dir_fd=private)
                except FileNotFoundError:
                    pass
                os.close(private)
                os.rmdir(attempt, dir_fd=relays)
        finally:
            os.close(relays)
    return 0


def checkout_proof(home, checkout):
    parts = checkout.split("/")
    if len(parts) != 7 or parts[:2] != [".factory", "features"] or parts[4] != "attempts" or parts[6] != "checkout":
        raise ValueError("invalid checkout")
    for position in (2, 3, 5):
        if str(uuid.UUID(parts[position])) != parts[position]:
            raise ValueError("invalid checkout")

    slot = os.dup(home)
    try:
        for name in parts[:-1]:
            child = owned_directory(slot, name, private=False)
            os.close(slot)
            slot = child
        receipt = os.open(
            "ready", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=slot
        )
        with os.fdopen(receipt, "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
                raise PermissionError("invalid checkout receipt")
            value = json.loads(stream.read(513))
        directory = owned_directory(slot, "checkout", private=False)
        try:
            info = os.fstat(directory)
        finally:
            os.close(directory)
        if value != {"device": info.st_dev, "inode": info.st_ino}:
            raise ValueError("checkout was replaced")
        return f"{info.st_dev}:{info.st_ino}"
    finally:
        os.close(slot)


def proof_main(checkout):
    home = os.open("/home/factory", DIRECTORY)
    try:
        print(checkout_proof(home, checkout), flush=True)
    finally:
        os.close(home)
    return 0


def matches_mount(path, proof):
    info = os.stat(path, follow_symlinks=False)
    return stat.S_ISDIR(info.st_mode) and proof == f"{info.st_dev}:{info.st_ino}"


def guarded_main(proof, relay_proof, *command):
    if not matches_mount("/workspace/checkout", proof):
        return 1
    if relay_proof != "-" and not matches_mount("/run/factory-relay", relay_proof):
        return 1
    os.execv(command[0], command)


RELAY = "/run/factory-relay/relay.sock"


def seed_credentials(provider):
    directory, filename, _ = PROVIDERS[provider]
    with socket.socket(socket.AF_UNIX) as channel:
        channel.settimeout(10)
        channel.connect(RELAY)
        channel.sendall(b"FACTORY-AUTH\r\n")
        with channel.makefile("rb") as response:
            size = int.from_bytes(response.read(4), "big")
            if not 0 < size <= MAX_CREDENTIAL:
                raise ValueError("isolated credentials unavailable")
            data = response.read(size)
            if len(data) != size:
                raise ValueError("isolated credentials unavailable")
    private = os.path.join("/home/factory", directory)
    os.mkdir(private, mode=0o700)
    with open(os.path.join(private, filename), "xb") as output:
        output.write(data)


def serve_loop(listener):
    slots = threading.BoundedSemaphore(16)

    def proxy(connection):
        try:
            with connection, socket.socket(socket.AF_UNIX) as upstream:
                upstream.settimeout(20)
                connection.settimeout(20)
                upstream.connect(RELAY)
                tunnel(connection, upstream)
        except OSError:
            pass
        finally:
            slots.release()

    while True:
        connection, _ = listener.accept()
        if not slots.acquire(blocking=False):
            connection.close()
            continue
        threading.Thread(target=proxy, args=(connection,), daemon=True).start()


def worker_main(provider, *args):
    os.umask(0o077)
    seed_credentials(provider)
    with socket.socket(socket.AF_INET) as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(16)
        thread = threading.Thread(target=serve_loop, args=(listener,), daemon=True)
        thread.start()
        proxy = f"http://127.0.0.1:{listener.getsockname()[1]}"
        os.environ.update(HTTP_PROXY=proxy, HTTPS_PROXY=proxy, http_proxy=proxy, https_proxy=proxy)
        binary = {"codex": "/opt/codex/bin/codex", "claude": "/opt/claude/claude"}[provider]
        try:
            return subprocess.call([binary, *args], timeout=86400)
        except subprocess.TimeoutExpired:
            return 124


if __name__ == "__main__":
    try:
        role, *args = sys.argv[1:]
        sys.exit({
            "relay": relay_main, "worker": worker_main, "proof": proof_main, "guard": guarded_main,
        }[role](*args))
    except (OSError, ValueError, KeyError, TypeError):
        sys.exit(1)
