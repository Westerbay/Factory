"""Behavior tests for the provider relay. No Docker, network or credentials needed."""
import io
import json
import socket
import unittest

from launcher import PROVIDERS, access_only, connect_target, establish


class ConnectTargetTest(unittest.TestCase):
    def test_allows_exact_provider_connect(self):
        for _, _, hosts in PROVIDERS.values():
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
        hosts = PROVIDERS["codex"][2]
        for request in (
            b"CONNECT ssh.github.com:443 HTTP/1.1\r\n\r\n",
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


if __name__ == "__main__":
    unittest.main()
