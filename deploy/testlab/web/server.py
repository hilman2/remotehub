"""Web target of the test lab: an appliance's sign-in page over HTTPS.

GET  /        the sign-in form (user name, password, submit); it also loads
              an image from port 8443, which remotehub's browser must never
              reach: that port is another device
POST /login   checks tester / Tester-Passw0rd!, then shows who signed in
GET  /last    for the tests, as JSON, never the password:
              {"username": ..., "ok": ..., "count": <sign-ins so far>,
               "escapes": <requests that reached port 8443>}
              also over plain HTTP on port 8080
"""

import json
import ssl
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs

USERNAME = "tester"
PASSWORD = "Tester-Passw0rd!"

FORM = b"""<!doctype html>
<html><head><title>Lab appliance</title></head>
<body style="font-family: sans-serif; background: #1e5b8c; color: white">
<h1>Lab appliance</h1>
<form method="post" action="/login">
  <label>User <input name="user" type="text" autocomplete="username"></label>
  <label>Password <input name="pass" type="password" autocomplete="current-password"></label>
  <button type="submit">Sign in</button>
</form>
<img src="https://web-target:8443/escape.png" alt="">
</body></html>
"""

last = {"username": None, "ok": None, "count": 0, "escapes": 0}
lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/":
            self.reply(200, "text/html", FORM)
        elif self.path == "/last":
            with lock:
                body = json.dumps(last).encode()
            self.reply(200, "application/json", body)
        else:
            self.reply(404, "text/plain", b"not found")

    def do_POST(self):
        if self.path != "/login":
            self.reply(404, "text/plain", b"not found")
            return
        length = int(self.headers.get("Content-Length", "0"))
        form = parse_qs(self.rfile.read(length).decode())
        username = form.get("user", [""])[0]
        ok = username == USERNAME and form.get("pass", [""])[0] == PASSWORD
        with lock:
            last.update(username=username, ok=ok, count=last["count"] + 1)
        # Green once signed in, red if not: tests read the colour from the picture.
        text = f"Signed in as {username}" if ok else "Wrong user name or password"
        colour = "#2e7d32" if ok else "#b3261e"
        page = (
            f"<!doctype html><title>Lab appliance</title>"
            f'<body style="font-family: sans-serif; background: {colour}; color: white">'
            f"<h1>{text}</h1></body>"
        )
        self.reply(200 if ok else 401, "text/html", page.encode())

    def reply(self, status, kind, body):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class Escape(Handler):
    """Port 8443: counts every request."""

    def do_GET(self):
        with lock:
            last["escapes"] += 1
        self.reply(404, "text/plain", b"not found")


def tls(port, handler):
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain("/etc/web/cert.pem", "/etc/web/key.pem")
    server = ThreadingHTTPServer(("0.0.0.0", port), handler)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    return server


plain = ThreadingHTTPServer(("0.0.0.0", 8080), Handler)
threading.Thread(target=plain.serve_forever, daemon=True).start()
threading.Thread(target=tls(8443, Escape).serve_forever, daemon=True).start()
tls(443, Handler).serve_forever()
