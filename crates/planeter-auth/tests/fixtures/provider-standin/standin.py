#!/usr/bin/env python3
"""A stand-in OpenID provider over HTTPS for planeter's tests (RFC 014 D-11). TEST ONLY.

usage: standin.py BIND_ADDRESS DIR CERT KEY

Serves, from DIR, `discovery.json` at /.well-known/openid-configuration, `jwks.json` at /jwks and `token.json` at POST /token. Every request
is appended to DIR/requests.log as one JSON line (method, path, the Host, Cache-Control, Pragma and the POST body), so a test can count what
reached it. If DIR/redirect exists, every GET is answered with a 302 to the URL in that file. The chosen port is written to DIR/port once the
socket is listening. The certificate is the self-made one next to this file; the test makes `curl` trust it with `--cacert` (the product has
no such mechanism: RFC 014 open question 4)."""
import http.server, json, os, ssl, sys

bind, d, cert, key = sys.argv[1:5]


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def record(self, body=""):
        with open(os.path.join(d, "requests.log"), "a") as f:
            f.write(json.dumps({
                "method": self.command, "path": self.path, "host": self.headers.get("Host"),
                "cache_control": self.headers.get("Cache-Control"), "pragma": self.headers.get("Pragma"), "body": body,
            }) + "\n")

    def reply(self, status, body=b"", location=None):
        self.send_response(status)
        if location:
            self.send_header("Location", location)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def serve(self, name):
        try:
            with open(os.path.join(d, name), "rb") as f:
                self.reply(200, f.read())
        except OSError:
            self.reply(404, b"{}")

    def do_GET(self):
        self.record()
        redirect = os.path.join(d, "redirect")
        if os.path.exists(redirect):
            self.reply(302, b"", open(redirect).read().strip())
        elif self.path == "/.well-known/openid-configuration":
            self.serve("discovery.json")
        elif self.path == "/jwks":
            self.serve("jwks.json")
        else:
            self.reply(404, b"{}")

    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        self.record(self.rfile.read(n).decode())
        if self.path == "/token":
            self.serve("token.json")
        else:
            self.reply(404, b"{}")


server = http.server.ThreadingHTTPServer((bind, 0), Handler)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(cert, key)
server.socket = ctx.wrap_socket(server.socket, server_side=True)
with open(os.path.join(d, "port.tmp"), "w") as f:
    f.write(str(server.server_address[1]))
os.rename(os.path.join(d, "port.tmp"), os.path.join(d, "port"))
server.serve_forever()
