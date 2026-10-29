"""Owned integration fixture only; no production image or credential usage."""
import os
import socket
import struct
import threading
from pathlib import Path


def answer(packet):
    offset = 12
    while packet[offset]:
        offset += packet[offset] + 1
    offset += 1
    query_type = struct.unpack("!H", packet[offset:offset + 2])[0]
    question = packet[12:offset + 4]
    mode = Path("/tmp/dns-mode").read_text().strip()
    records = []
    if query_type == 1:
        addresses = {
            "public": ["11.255.250.2"],
            "private": [os.environ["FIXTURE_PRIVATE_IP"]],
            "mixed": ["11.255.250.2", os.environ["FIXTURE_PRIVATE_IP"]],
        }[mode]
        records = [b"\xc0\x0c" + struct.pack("!HHIH", 1, 1, 0, 4) + socket.inet_aton(ip) for ip in addresses]
    if query_type == 28 and mode == "mixed":
        records = [b"\xc0\x0c" + struct.pack("!HHIH", 28, 1, 0, 16) + socket.inet_pton(socket.AF_INET6, "fd00::1")]
    return packet[:2] + struct.pack("!HHHHH", 0x8180, 1, len(records), 0, 0) + question + b"".join(records)


def dns():
    server = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    server.bind(("0.0.0.0", 53))
    while True:
        packet, peer = server.recvfrom(4096)
        server.sendto(answer(packet), peer)


def target():
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("0.0.0.0", 443))
    server.listen()
    while True:
        connection, _ = server.accept()
        with Path("/tmp/connections").open("a") as log:
            log.write(connection.getsockname()[0] + "\n")
        connection.close()


Path("/tmp/dns-mode").write_text("public")
threading.Thread(target=target, daemon=True).start()
dns()
