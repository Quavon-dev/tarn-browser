import datetime
import ssl
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG, CERT, KEY = sys.argv[1], sys.argv[2], sys.argv[3]
PUBLIC = "https://127.0.0.1.nip.io:8446"
LOCAL_HTTP = "http://127.0.0.1:8775"
LOCAL_HTTPS = "https://127.0.0.1:8447"
PIXEL = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;"
STYLE = "<meta name=viewport content='width=device-width'><style>body{font:18px -apple-system;margin:12px}a,button{display:block;margin:12px 0;font-size:18px}</style>"
PUBLIC_PAGE = f"""{STYLE}<title>Public page</title><h2>Public page</h2><p id=f>fetch: pending</p>
<img src="{LOCAL_HTTPS}/pixel?from=public-img-https" width=20 height=20>
<img src="https://localhost:8447/pixel?from=public-img-localhost" width=20 height=20>
<img src="{LOCAL_HTTP}/pixel?from=public-img-http" width=20 height=20>
<a href="{LOCAL_HTTP}/pixel?from=public-link">local link</a>
<button onclick="location='{LOCAL_HTTPS}/pixel?from=public-script'">script to local</button>
<a target=_blank href="{LOCAL_HTTPS}/pixel?from=public-popup">local popup</a>
<a target=_blank href="/redirect?to={LOCAL_HTTPS}/pixel?from=public-popup-redirect">popup redirecting to local</a>
<script>fetch("{LOCAL_HTTPS}/pixel?from=public-fetch").then(r=>f.textContent="fetch: loaded "+r.status,e=>f.textContent="fetch: refused")</script>"""
LOCAL_PAGE = f"""{STYLE}<title>Local page</title><h2>Local page</h2>
<img src="{LOCAL_HTTPS}/pixel?from=local-img-https" width=20 height=20>"""
lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, status, body, kind="text/html; charset=utf-8", headers=()):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        with lock, open(LOG, "a") as log:
            stamp = datetime.datetime.now().strftime("%H:%M:%S")
            log.write(f"{stamp} {self.server.name} Host={self.headers.get('Host')} {self.path}\n")
        path = self.path.split("?")[0]
        if path == "/pixel":
            return self.send(200, PIXEL, "image/gif")
        if path == "/redirect":
            return self.send(302, b"", headers=[("Location", self.path.partition("?to=")[2])])
        if path == "/public.html":
            return self.send(200, PUBLIC_PAGE.encode())
        if path == "/local.html":
            return self.send(200, LOCAL_PAGE.encode())
        self.send(404, b"not found")


def serve(port, name, tls):
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    server.name = name
    if tls:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(CERT, KEY)
        server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()


serve(8446, "public-https", True)
serve(8447, "local-https", True)
serve(8775, "local-http", False)
threading.Event().wait()
