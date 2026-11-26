#!/usr/bin/env python3
"""TLS terminator + measuring/caching stand-in in front of a plain-HTTP sui-id (test procedure only; stdlib only).

usage: tlsproxy.py LISTEN_PORT UPSTREAM_PORT CERT KEY CONTROL_DIR

* terminates TLS (the throw-away CA's certificate) and forwards every request to 127.0.0.1:UPSTREAM_PORT, adding X-Forwarded-Proto: https
* writes one JSON line per request to CONTROL_DIR/requests.log: time, method, path, whether the request carried Cache-Control: no-cache, the
  status, the response size, the milliseconds it took, and what was served (upstream | stale | override)
* control files (read on every request; create or remove them while a check runs):
    jwks-stale.json      the JWKS body served, in place of the provider's, to a request for /.well-known/jwks.json that does NOT carry
                         `Cache-Control: no-cache` (a cache in front of the provider holding the old set); a request that does carry it goes
                         through to the provider, unless the file `jwks-ignore-nocache` exists (then the stale set is served to it too)
    token-delay-secs     seconds to wait before forwarding a POST /oauth2/token
    discovery-override   a body served, in place of the provider's, for /.well-known/openid-configuration (any bytes)
    down                 the connection is closed without an answer (the provider is down)
"""
import http.client
import json
import os
import ssl
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LISTEN, UPSTREAM, CERT, KEY, CTL = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]
LOCK = threading.Lock()


def ctl(name):
    p = os.path.join(CTL, name)
    return p if os.path.exists(p) else None


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"  # one request per connection: simple and enough

    def log_message(self, *a):
        pass

    def _handle(self):
        t0 = time.time()
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        path = self.path
        route = path.split("?", 1)[0]
        nocache = "no-cache" in (self.headers.get("Cache-Control") or "").lower()
        served = "upstream"
        status, hdrs, data = None, [], b""
        ua = self.headers.get("User-Agent") or ""

        if ctl("down"):  # the provider is down: the connection is closed without an answer
            with LOCK:
                with open(os.path.join(CTL, "requests.log"), "a") as f:
                    f.write(json.dumps({"t": round(t0, 3), "method": self.command, "path": route, "ua": ua, "served": "down"}) + "\n")
            self.close_connection = True
            try:
                self.connection.shutdown(2)
            except Exception:
                pass
            return

        if route == "/.well-known/openid-configuration" and ctl("discovery-override"):
            served, status, data = "override", 200, open(ctl("discovery-override"), "rb").read()
            hdrs = [("Content-Type", "application/json")]
        elif route == "/.well-known/jwks.json" and ctl("jwks-stale.json") and (not nocache or ctl("jwks-ignore-nocache")):
            served, status, data = "stale", 200, open(ctl("jwks-stale.json"), "rb").read()
            hdrs = [("Content-Type", "application/json"), ("Cache-Control", "public, max-age=300")]
        else:
            if route == "/oauth2/token" and self.command == "POST" and ctl("token-delay-secs"):
                time.sleep(float(open(ctl("token-delay-secs")).read().strip()))
            try:
                c = http.client.HTTPConnection("127.0.0.1", UPSTREAM, timeout=120)
                fwd = {k: v for k, v in self.headers.items() if k.lower() not in ("connection", "keep-alive", "transfer-encoding")}
                fwd["X-Forwarded-Proto"] = "https"
                c.request(self.command, path, body=body or None, headers=fwd)
                r = c.getresponse()
                data = r.read()
                status = r.status
                hdrs = [(k, v) for k, v in r.getheaders() if k.lower() not in ("connection", "transfer-encoding", "content-length", "keep-alive")]
            except Exception as e:  # the provider is down
                served, status, data, hdrs = "error:" + type(e).__name__, 502, b"bad gateway", []

        self.send_response(status)
        for k, v in hdrs:
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except Exception:
            pass
        rec = {
            "t": round(t0, 3), "method": self.command, "path": route, "nocache": nocache, "status": status,
            "bytes": len(data), "ms": int((time.time() - t0) * 1000), "served": served, "ua": ua,
            "cache_control": next((v for k, v in hdrs if k.lower() == "cache-control"), None),
        }
        with LOCK:
            with open(os.path.join(CTL, "requests.log"), "a") as f:
                f.write(json.dumps(rec) + "\n")

    do_GET = do_POST = do_HEAD = do_PUT = do_DELETE = _handle


def main():
    os.makedirs(CTL, exist_ok=True)
    srv = ThreadingHTTPServer(("127.0.0.1", LISTEN), H)
    srv.daemon_threads = True
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(CERT, KEY)
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    with open(os.path.join(CTL, "port"), "w") as f:
        f.write(str(LISTEN))
    srv.serve_forever()


main()
