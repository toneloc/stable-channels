#!/usr/bin/env python3
"""Minimal read-only git smart-HTTP server for the local Umbrel test store.

Serves (dual-stack IPv4+IPv6 — host.docker.internal may resolve to either):
  /stable-channels-app-store/.git/...  git smart-HTTP (upload-pack only)
  /icon.png                            the app icon referenced by the manifest

Build the store repo first with make-store.sh, then run:
  python3 git-smart-http.py <store-repo-path> [port]
"""
import socket, subprocess, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

REPO = sys.argv[1] if len(sys.argv) > 1 else str(Path(__file__).parent / "store-repo")
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8929
ICON = Path(__file__).parent / "icon.png"


class DualStackServer(ThreadingHTTPServer):
    address_family = socket.AF_INET6

    def server_bind(self):
        self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        super().server_bind()


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _pkt(self, s):
        return (f"{len(s) + 4:04x}" + s).encode()

    def _send(self, code, ctype, body):
        self.send_response(code)
        if ctype:
            self.send_header("Content-Type", ctype)
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if "info/refs" in self.path and "service=git-upload-pack" in self.path:
            out = subprocess.run(
                ["git", "upload-pack", "--stateless-rpc", "--advertise-refs", REPO],
                capture_output=True,
            ).stdout
            body = self._pkt("# service=git-upload-pack\n") + b"0000" + out
            self._send(200, "application/x-git-upload-pack-advertisement", body)
        elif self.path == "/icon.png" and ICON.exists():
            self._send(200, "image/png", ICON.read_bytes())
        else:
            self._send(404, None, b"")

    def do_POST(self):
        if self.path.endswith("git-upload-pack"):
            data = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            out = subprocess.run(
                ["git", "upload-pack", "--stateless-rpc", REPO],
                input=data,
                capture_output=True,
            ).stdout
            self._send(200, "application/x-git-upload-pack-result", out)
        else:
            self._send(404, None, b"")

    def log_message(self, *a):
        print(*a, file=sys.stderr)


print(f"serving {REPO} + icon on :{PORT} (dual-stack)", file=sys.stderr)
DualStackServer(("::", PORT), H).serve_forever()
