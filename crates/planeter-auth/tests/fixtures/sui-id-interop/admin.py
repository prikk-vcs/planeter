#!/usr/bin/env python3
"""Drives sui-id's own admin console with plain HTTP calls (stdlib only): the administrator's way of doing things.

usage: admin.py WORKDIR COMMAND [ARGS...]     (WORKDIR holds run/ports.env, run/setup.out, tls/ca.pem)
commands:
  probe                      sign in as the administrator and print the pages' forms (to learn them)
  create-user NAME           create an ordinary user; print its id and generated password as JSON
  create-client NAME KIND    KIND = public | confidential; redirect URI from env REDIRECT; print client_id/secret as JSON
  rotate-key                 rotate the signing key from the console
  disable-user ID | enable-user ID
"""
import http.client
import http.cookiejar
import json
import os
import re
import ssl
import sys
import urllib.parse
from html.parser import HTMLParser

W = sys.argv[1]
PORTS = dict(kv.split("=") for kv in open(f"{W}/run/ports.env").read().split() if "=" in kv)
PP = int(PORTS["PP"])
CA = f"{W}/tls/ca.pem"
SETUP = open(f"{W}/run/setup.out").read()
ADMIN_PW = re.search(r"password: (\S+)", SETUP).group(1)
CTX = ssl.create_default_context(cafile=CA)


class Conn(http.client.HTTPSConnection):
    def connect(self):
        import socket
        sock = socket.create_connection(("127.0.0.1", PP), 20)
        self.sock = CTX.wrap_socket(sock, server_hostname="idp.test")


class Form(HTMLParser):
    def __init__(self):
        super().__init__()
        self.forms, self.cur = [], None

    def handle_starttag(self, tag, a):
        a = dict(a)
        if tag == "form":
            self.cur = {"action": a.get("action"), "method": a.get("method", "get"), "fields": []}
            self.forms.append(self.cur)
        elif tag in ("input", "select", "textarea", "button") and self.cur is not None:
            self.cur["fields"].append((tag, a.get("name"), a.get("type"), a.get("value")))

    def handle_endtag(self, tag):
        if tag == "form":
            self.cur = None


class Browser:
    def __init__(self):
        self.cookies = {}

    def req(self, method, path, form=None, follow=True, headers=None):
        for _ in range(10):
            c = Conn("idp.test", PP)
            h = {"Host": f"idp.test:{PP}", "Accept-Language": "en", "Cookie": "; ".join(f"{k}={v}" for k, v in self.cookies.items())}
            h.update(headers or {})
            body = None
            if form is not None:
                body = urllib.parse.urlencode(form)
                h["Content-Type"] = "application/x-www-form-urlencoded"
            c.request(method, path, body=body, headers=h)
            r = c.getresponse()
            data = r.read().decode("utf-8", "replace")
            for k, v in r.getheaders():
                if k.lower() == "set-cookie":
                    nv = v.split(";", 1)[0]
                    n, _, val = nv.partition("=")
                    if "max-age=0" in v.lower() or "expires=thu, 01 jan 1970" in v.lower():
                        self.cookies.pop(n, None)
                    else:
                        self.cookies[n] = val
            loc = r.getheader("Location")
            if follow and r.status in (301, 302, 303, 307, 308) and loc:
                path = urllib.parse.urlparse(loc)
                path = path.path + ("?" + path.query if path.query else "")
                method, form = "GET", None
                continue
            return r.status, loc, data, path
        raise RuntimeError("too many redirects")

    def forms(self, path):
        st, loc, html, p = self.req("GET", path)
        f = Form()
        f.feed(html)
        return st, f.forms, html, p

    def csrf(self, html):
        m = re.search(r'name="_csrf"[^>]*value="([^"]+)"', html) or re.search(r'value="([^"]+)"[^>]*name="_csrf"', html)
        return m.group(1) if m else None


def login(b):
    st, forms, html, p = b.forms("/admin/login")
    csrf = b.csrf(html)
    form = {"username": "admin", "password": ADMIN_PW, "next": "/admin"}
    if csrf:
        form["_csrf"] = csrf
    st, loc, html, p = b.req("POST", "/admin/login", form)
    return st, p, html


def text(html):
    html = re.sub(r"(?s)<(script|style).*?</\1>", " ", html)
    return re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", html)).strip()


def post_form(b, page, action, fields):
    """GET `page`, take its CSRF token, POST `fields` to `action`; return (status, final path, page text, raw html)."""
    st, forms, html, p = b.forms(page)
    form = dict(fields)
    form["_csrf"] = b.csrf(html)
    st, loc, html2, p2 = b.req("POST", action, form)
    return st, p2, text(html2), html2


def confirm_action(b, path, extra=None, depth=0):
    """GET a dangerous-action confirm page, post its form as a browser would; handle one step-up (password) if asked."""
    st, loc, html, p = b.req("GET", path)
    f = Form(); f.feed(html)
    forms = [fm for fm in f.forms if fm["action"] != "/admin/logout"]
    if not forms:
        return {"status": st, "path": p, "text": text(html)[:500], "note": "no form"}
    fm = forms[0]
    form = {}
    for tag, name, typ, val in fm["fields"]:
        if name:
            form[name] = val or ""
    form.update(extra or {})
    st, loc, html2, p2 = b.req("POST", fm["action"], form, follow=False)
    out = {"post": fm["action"], "status": st, "location": loc, "fields": sorted(form)}
    if 300 <= st < 400 and loc:
        st2, l2, html3, p3 = b.req("GET", loc)
        out["then"] = {"status": st2, "path": p3, "text": text(html3)[:500]}
        f3 = Form(); f3.feed(html3)
        out["then"]["forms"] = [(x["action"], [y[1] for y in x["fields"] if y[1]]) for x in f3.forms if x["action"] != "/admin/logout"]
    else:
        out["text"] = text(html2)[:500]
    return out


def main():
    cmd = sys.argv[2]
    if cmd == "signal-provider":      # STOP | CONT | KILL: the real sui-id process (pid in run/pid.sui-id)
        import signal
        pid = int(open(f"{W}/run/pid.sui-id").read().strip())
        os.kill(pid, {"STOP": signal.SIGSTOP, "CONT": signal.SIGCONT}[sys.argv[3]])
        print(json.dumps({"signalled": pid, "signal": sys.argv[3]})); return
    if cmd == "unlock-user":          # the CLI, offline: sui-id admin unlock-user --username NAME
        import subprocess
        r = subprocess.run([f"{os.path.abspath(W)}/target/release/sui-id", "admin", "unlock-user", "--username", sys.argv[3], "--config", "./sui-id.toml"], cwd=f"{W}/run", capture_output=True, text=True)
        print(json.dumps({"status": r.returncode, "out": r.stdout.strip()[:100]})); return
    b = Browser()
    st, p, html = login(b)
    if cmd == "client-scopes":        # client-scopes CLIENT_ID "openid profile email offline_access"
        cid, scopes = sys.argv[3], sys.argv[4]
        st, loc, html, p0 = b.req("GET", f"/admin/clients/{cid}/edit")
        f = Form(); f.feed(html)
        fm = [x for x in f.forms if x["action"] == f"/admin/clients/{cid}/edit"][0]
        form = {}
        for t, n, ty, v in fm["fields"]:
            if n:
                form[n] = v or ""
        form["redirect_uris"] = os.environ.get("REDIRECT", "https://forge.example/login/oidc/callback")
        form["allowed_scopes"] = scopes
        form["consent_policy"] = "none"
        st, loc, h2, p2 = b.req("POST", fm["action"], form, follow=False)
        print(json.dumps({"status": st, "location": loc, "text": text(h2)[:300] if st >= 400 else ""})); return
    if cmd == "probe":
        print("login ->", st, p, "cookies:", list(b.cookies))
        for page in ["/admin", "/admin/users", "/admin/users/new", "/admin/clients/new", "/admin/signing-keys"]:
            st, forms, html, pth = b.forms(page)
            print("\n##", page, "->", st, pth)
            for f in forms:
                print("  form", f["method"], f["action"], [x[1] for x in f["fields"] if x[1]])
        return
    if cmd == "create-user":
        name = sys.argv[3]
        st, p, t, raw = post_form(b, "/admin/users/new", "/admin/users", {"username": name, "display_name": name.title(), "email": os.environ.get("EMAIL", "")})
        m = re.search(r"/admin/users/([0-9a-f-]{36})", p)
        print(json.dumps({"status": st, "user_id": m.group(1) if m else None}))
        return
    if cmd == "create-client":
        name, kind = sys.argv[3], sys.argv[4]
        fields = {"name": name, "redirect_uris": os.environ.get("REDIRECT", "https://forge.example/login/oidc/callback"),
                  "allowed_scopes": "openid profile email", "post_logout_redirect_uris": "", "consent_policy": "none"}
        if kind == "confidential":
            fields["confidential"] = "true"
        elif kind == "public":
            fields["confidential"] = "false"      # an unchecked box sends nothing, and the handler reads nothing as confidential
        elif kind == "public-unchecked-box":
            pass                                  # what a browser sends for the unchecked box: no field at all
        st, p, t, raw = post_form(b, "/admin/clients/new", "/admin/clients", fields)
        # The list page names every client: find this one by name (the newest of that name is the last in the list).
        st2, forms2, html2, p2 = b.forms("/admin/clients")
        lt = text(html2)
        ids = re.findall(re.escape(name) + r" \S+ ([0-9a-f]{8}-[0-9a-f-]{27}) \S+ \S+ (confidential|public)", lt)
        cid, listed = ids[-1] if ids else (None, None)
        secret = re.search(r"Client ID: " + re.escape(cid or "-") + r" \S+ \S+ Client Secret: (\S+)", t)
        print(json.dumps({"status": st, "client_id": cid, "listed_kind": listed, "client_secret": secret.group(1) if secret else None}))
        return
    if cmd == "set-password":
        token, pw = sys.argv[3], sys.argv[4]
        b2 = Browser()
        st, p2, t, raw = post_form(b2, "/reset-password", "/reset-password", {"token": token, "password": pw, "confirm_password": pw})
        print(json.dumps({"status": st, "path": p2, "text": t[:400]}))
        return
    if cmd == "try":
        # try POST path with fields from argv: try /page /action k=v ...
        page, action = sys.argv[3], sys.argv[4]
        fields = dict(kv.split("=", 1) for kv in sys.argv[5:])
        st, loc, html0, p0 = b.req("GET", page)
        fields["_csrf"] = b.csrf(html0)
        st, loc, html, p2 = b.req("POST", action, fields, follow=False)
        print("->", st, loc)
        if st < 300 or st >= 400:
            print(text(html)[:800])
        else:
            st, l2, html, p3 = b.req("GET", loc)
            f = Form(); f.feed(html)
            print("GET", loc, "->", st, p3); print(text(html)[:600])
            for fm in f.forms:
                print("  form", fm["method"], fm["action"], fm["fields"])
        return
    if cmd == "rotate-key":
        print(json.dumps(confirm_action(b, "/admin/signing-keys/rotate-confirm"))[:400]); return
    if cmd == "disable-user":
        print(json.dumps(confirm_action(b, f"/admin/users/{sys.argv[3]}/disable-confirm", {"reason": "interop test: offboarding"}))[:600]); return
    if cmd == "enable-user":
        print(json.dumps(confirm_action(b, f"/admin/users/{sys.argv[3]}/disable-confirm", {"reason": "interop test: re-enabled"}))[:300]); return
    if cmd == "confirm-action":
        path = sys.argv[3]
        extra = dict(kv.split("=", 1) for kv in sys.argv[4:])
        r = confirm_action(b, path, extra)
        print(json.dumps(r)); return
    if cmd == "dump":
        st, forms, html, p = b.forms(sys.argv[3])
        print(st, p)
        print(text(html)[:3000])
        for f in forms:
            print("  form", f["method"], f["action"], f["fields"])
        return


main()
