"""Behavior tests for the provider relay. No Docker, network or credentials needed."""
import io
import json
import os
from pathlib import Path
import socket
import tempfile
import unittest
import uuid
from unittest.mock import patch

from launcher import (
    PROVIDERS,
    REPOSITORY_HTTPS,
    WORKSPACE_HTTPS,
    access_only,
    checkout_proof,
    connect_target,
    establish,
    github_read,
)


class ConnectTargetTest(unittest.TestCase):
    def test_worker_hosts_are_reachable_through_installation_proxy(self):
        squid_config = Path(__file__).resolve().parents[2] / "egress/squid.conf"
        acl_prefix = "acl providers dstdomain -n "
        proxy_hosts = next(
            line.removeprefix(acl_prefix).split()
            for line in squid_config.read_text().splitlines()
            if line.startswith(acl_prefix)
        )
        for _, _, provider_hosts in PROVIDERS.values():
            self.assertLessEqual(set(provider_hosts + WORKSPACE_HTTPS), set(proxy_hosts))

    def test_repository_downloads_use_the_setup_relay(self):
        squid = Path(__file__).resolve().parents[2] / "egress/squid.conf"
        allowed = set()
        for line in squid.read_text().splitlines():
            if line.startswith(("acl providers dstdomain -n ", "acl tooling dstdomain -n ")):
                allowed.update(line.split()[4:])
        self.assertLessEqual(set(REPOSITORY_HTTPS), allowed)
        for _, _, provider_hosts in PROVIDERS.values():
            self.assertTrue(set(REPOSITORY_HTTPS).isdisjoint(provider_hosts))
            self.assertNotIn("static.rust-lang.org", provider_hosts + WORKSPACE_HTTPS)

    def test_allows_exact_provider_and_workspace_hosts(self):
        for _, _, provider_hosts in PROVIDERS.values():
            hosts = provider_hosts + WORKSPACE_HTTPS
            for host in hosts:
                authority = host + ":443"
                self.assertEqual(
                    connect_target(
                        f"CONNECT {authority} HTTP/1.1\r\n".encode(),
                        io.BytesIO(f"Host: {authority}\r\nProxy-Connection: Keep-Alive\r\n\r\n".encode()),
                        hosts,
                    ),
                    authority,
                )

    def test_rejects_cross_provider_and_smuggled_connect(self):
        hosts = PROVIDERS["codex"][2] + WORKSPACE_HTTPS
        for request in (
            b"CONNECT ssh.github.com:443 HTTP/1.1\r\n\r\n",
            b"CONNECT github.com.example.org:443 HTTP/1.1\r\n\r\n",
            b"CONNECT registry.npmjs.org.example.org:443 HTTP/1.1\r\n\r\n",
            b"CONNECT auth.openai.com:443 HTTP/1.1\r\n\r\n",
            b"CONNECT api.anthropic.com:443 HTTP/1.1\r\n\r\n",
            b"CONNECT API.OPENAI.COM:443 HTTP/1.1\r\n\r\n",
            b"CONNECT api.openai.com:22 HTTP/1.1\r\n\r\n",
            b"CONNECT api.openai.com:443/ HTTP/1.1\r\n\r\n",
            b"CONNECT api.openai.com:443 HTTP/1.1\n\n",
            b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: ssh.github.com:443\r\n\r\n",
            b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nHost: api.openai.com:443\r\n\r\n",
            b"CONNECT api.openai.com:443 HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"CONNECT api.openai.com:443 HTTP/1.1\r\nContent-Length: 0\r\n\r\n",
            b"CONNECT api.openai.com:443 HTTP/1.1\r\nX-Large: " + b"x" * 8192 + b"\r\n\r\n",
        ):
            with self.subTest(request=request[:100]):
                stream = io.BytesIO(request)
                self.assertIsNone(connect_target(stream.readline(), stream, hosts))

    def test_canonical_request_keeps_tunnel_bytes(self):
        source, upstream = socket.socketpair()
        with source, upstream:
            source.settimeout(2)
            upstream.settimeout(2)
            upstream.sendall(b"HTTP/1.1 200 Connection established\r\n\r\ntunnel")
            self.assertTrue(establish(source, "chatgpt.com:443"))
            self.assertEqual(source.recv(6), b"tunnel")
            self.assertEqual(
                upstream.recv(512),
                b"CONNECT chatgpt.com:443 HTTP/1.1\r\nHost: chatgpt.com:443\r\n\r\n",
            )


class WorkerCredentialTest(unittest.TestCase):
    def test_copies_only_the_selected_access_token(self):
        codex = json.loads(access_only("codex", b"""{
            "last_refresh":"2026-11-07T15:00:00Z",
            "tokens":{"access_token":"fixture","id_token":"fixture","refresh_token":"forbidden"},
            "personal_access_token":"forbidden"
        }"""))
        self.assertEqual(codex["tokens"]["access_token"], "fixture")
        self.assertEqual(codex["tokens"]["refresh_token"], "")
        self.assertNotIn("personal_access_token", codex)
        claude = json.loads(access_only("claude", b"""{
            "claudeAiOauth":{"accessToken":"fixture","expiresAt":4102444800000,"refreshToken":"forbidden"},
            "otherCredentials":"forbidden"
        }"""))
        self.assertEqual(claude, {
            "claudeAiOauth":{"accessToken":"fixture","expiresAt":4102444800000}
        })
        self.assertNotIn("platform.claude.com", PROVIDERS["claude"][2])


class GitHubReadTest(unittest.TestCase):
    def test_only_runs_a_fixed_upload_pack_command(self):
        client, server = socket.socketpair()
        with client, server, patch("launcher.subprocess.run") as run:
            for request in (
                b"FACTORY-GIT git-receive-pack 'owner/repo.git'\r\n",
                b"FACTORY-GIT git-upload-pack 'owner/repo.git'; id\r\n",
                b"FACTORY-GIT git-upload-pack '../repo.git'\r\n",
                b"FACTORY-GIT git-upload-pack '-owner/repo.git'\r\n",
                b"FACTORY-GIT git-upload-pack 'owner/sub/repo.git'\r\n",
            ):
                github_read(server, request)
            run.assert_not_called()
            github_read(server, b"FACTORY-GIT git-upload-pack '/owner/repo.git'\r\n")
            run.assert_called_once()
            self.assertEqual(run.call_args.args[0][-2:], ["github.com", "git-upload-pack 'owner/repo.git'"])
            self.assertIs(run.call_args.kwargs["stdin"], server)
            self.assertIs(run.call_args.kwargs["stdout"], server)
            self.assertEqual(run.call_args.kwargs["env"], {"HOME": "/home/factory", "PATH": "/usr/bin:/bin"})


class CheckoutProofTest(unittest.TestCase):
    def test_refuses_replaced_or_symlink_checkout(self):
        with tempfile.TemporaryDirectory() as home:
            relative = f".factory/features/{uuid.uuid4()}/{uuid.uuid4()}/attempts/{uuid.uuid4()}/checkout"
            checkout = Path(home) / relative
            checkout.mkdir(parents=True)
            info = checkout.stat()
            receipt = checkout.parent / "ready"
            receipt.write_text(json.dumps({"device": info.st_dev, "inode": info.st_ino}))
            receipt.chmod(0o600)
            directory = os.open(home, os.O_RDONLY | os.O_DIRECTORY)
            try:
                self.assertEqual(checkout_proof(directory, relative), f"{info.st_dev}:{info.st_ino}")
                moved = checkout.rename(checkout.parent / "moved")
                checkout.mkdir()
                with self.assertRaises(ValueError):
                    checkout_proof(directory, relative)
                checkout.rmdir()
                checkout.symlink_to(moved)
                with self.assertRaises(OSError):
                    checkout_proof(directory, relative)
            finally:
                os.close(directory)


if __name__ == "__main__":
    unittest.main()
