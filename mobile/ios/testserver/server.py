import base64
import datetime
import socket
import ssl
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PAGES = Path(__file__).parent / "pages"
LOG = Path(sys.argv[1])
CERT, KEY, PUBLIC_V6 = sys.argv[2], sys.argv[3], sys.argv[4]
AUTH = "Basic " + base64.b64encode(b"tarn:test").decode()
lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def record(self):
        with lock, LOG.open("a") as log:
            stamp = datetime.datetime.now().strftime("%H:%M:%S")
            log.write(f"{stamp} {self.server.name} {self.command} {self.path} cookie={self.headers.get('Cookie', '')}\n")

    def send(self, status, body, kind="text/html; charset=utf-8", headers=()):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.record()
        path = self.path.split("?")[0]
        if path == "/pixel":
            return self.send(200, b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;", "image/gif")
        if path == "/auth":
            if self.headers.get("Authorization") == AUTH:
                return self.send(200, b"<meta name=viewport content='width=device-width'><h1>Signed in</h1>")
            return self.send(401, b"<h1>Sign-in needed</h1>", headers=[("WWW-Authenticate", 'Basic realm="Tarn test"')])
        if path == "/redirect":
            target = urllib.parse.unquote(self.path.partition("?to=")[2])
            return self.send(302, b"", headers=[("Location", target)])
        if path == "/download.txt":
            return self.send(200, b"Tarn download test\n", "text/plain", [("Content-Disposition", 'attachment; filename="../tarn report.txt"')])
        page = PAGES / (path.strip("/") or "index.html")
        if page.parent == PAGES and page.suffix == ".html" and page.is_file():
            return self.send(200, page.read_bytes().replace(b"{V6}", PUBLIC_V6.encode()))
        self.send(404, b"not found")


class V6Server(ThreadingHTTPServer):
    address_family = socket.AF_INET6


def serve(server, name, tls):
    server.name = name
    if tls:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(CERT, KEY)
        server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()


serve(ThreadingHTTPServer(("127.0.0.1", 8765), Handler), "local-http", False)
serve(ThreadingHTTPServer(("127.0.0.1", 8444), Handler), "local-https", True)
serve(V6Server((PUBLIC_V6, 8443), Handler), "public-https", True)
threading.Event().wait()
