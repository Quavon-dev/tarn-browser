#!/usr/bin/env python3
"""Logging test server and proxy for the Android exit checks (debug builds only).

server.py PORT LOG [CERT KEY]: with a certificate, CONNECT to *.test reaches a TLS server on PORT+1.
"""

import base64
import socket
import ssl
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8771
LOG = open(sys.argv[2] if len(sys.argv) > 2 else "server.log", "a", buffering=1)
TLS_CERT = sys.argv[3] if len(sys.argv) > 4 else None
TLS_KEY = sys.argv[4] if len(sys.argv) > 4 else None
TLS_PORT = PORT + 1
PIXEL = base64.b64decode("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7")
LOCAL = f"http://localhost:{PORT}"
HEAD = '<meta name="viewport" content="width=device-width"><style>a,button{display:block;margin:8px 0;font-size:18px}</style>'

PAGES = {
    "/": "<h1>Tarn test</h1><a href='/links'>links</a><a href='/pn'>private network</a><a href='/dialogs'>dialogs</a>",
    "/pn": f"<h1>Private network</h1><img alt=lo src='http://127.0.0.1:{PORT}/pixel?via=127'>"
    f"<img alt=lh src='{LOCAL}/pixel?via=localhost'><img alt=ten src='http://10.0.2.2:{PORT}/pixel?via=10'>",
    "/links": "<h1>Links</h1>"
    "<a href=\"javascript:document.title='JS-LINK-RAN'\">javascript link</a>"
    "<a href='data:text/html,<h1>DATA-LINK-LOADED</h1>'>data link</a>"
    "<a href='mailto:a@example.com'>mailto link</a>"
    "<a href='intent://scan/#Intent;scheme=zxing;end'>intent link</a>"
    "<a href='http://public.test/plain'>plain http link</a>"
    f"<a href='{LOCAL}/local-from-public'>local link</a>"
    "<a href='http://public.test/popup-target' target='_blank'>blank target</a>"
    "<button onclick=\"window.open('http://public.test/opened')\">window.open</button>"
    "<button onclick=\"window.open('http://public.test/redirect-local')\">popup to local</button>"
    "<a href='http://public.test/redirect-local'>redirect to local</a>"
    "<script>setTimeout(()=>window.open('http://public.test/no-gesture'),300)</script>",
    "/dialogs": "<h1>Dialogs</h1>"
    "<button onclick=\"alert('hello')\">alert</button>"
    "<button onclick=\"document.title='confirm '+confirm('sure?')\">confirm</button>"
    "<button onclick=\"document.title='text '+prompt('name?','tarn')\">prompt</button>"
    "<button onclick=\"for(let i=0;i<5;i++)alert('flood '+i);document.title='flood done'\">flood</button>",
    "/filter": "<h1>Filter</h1><script src='http://ads.test/ad.js'></script>"
    "<img alt=ok src='http://ok.test/pixel?via=ok'><img alt=ad src='http://ads.test/pixel?via=ad'>",
    "/title": "<title>first</title><h1>Title</h1><script>setTimeout(()=>document.title='LATE-TITLE',1500)</script>",
    "/find": "<h1>Find</h1><p>alpha beta alpha gamma alpha</p>",
    "/push": "<h1>Push</h1><button onclick=\"history.pushState({}, '', '/pushed')\">push state</button>",
    "/perm": "<h1>Permissions</h1>"
    "<button onclick=\"navigator.geolocation.getCurrentPosition(p=>document.title='geo ok',e=>document.title='geo '+e.code)\">location</button>"
    "<button onclick=\"navigator.mediaDevices.getUserMedia({video:true}).then(()=>document.title='cam ok',e=>document.title='cam '+e.name)\">camera</button>"
    "<input id=field placeholder='page input'>",
    "/downloads": "<h1>Downloads</h1><a href='/download'>download file</a>",
    "/https-site": "<h1>public.test</h1><a href='http://public.test/plain'>plain link</a>",
    "/unload": "<body onpagehide=\"new Image().src='/beacon?ev=pagehide'\" onbeforeunload=\"new Image().src='/beacon?ev=beforeunload'\""
    " onunload=\"new Image().src='/beacon?ev=unload'\"><h1>UNLOAD-PAGE</h1>"
    "<button onclick=\"new Image().src='/beacon?ev=click'\">beacon</button>"
    "<a href='http://open.test/slow'>slow link</a></body>",
    "/ext": "<title>Extension page</title><h1>Extension</h1><iframe src='/ext-frame'></iframe>"
    "<img alt=ad src='http://ads.test/pixel?via=ext'><script src='http://ads.test/ad.js'></script>",
    "/ext-probe": "<title>probe</title><h1>Probe</h1><script>setTimeout(() => { const d = document.documentElement.dataset; "
    "document.title = [typeof window.chrome, typeof window.browser, typeof window.__tarnExtPort, typeof window.__tarnExtPost, "
    "typeof window.__tarnExtReceive, d.tarnFixture, d.tarnFixtureReply].join(','); }, 1500)</script>",
    "/popups": "<title>Popup opener</title><h1>Popups</h1><a href='http://public.test/opened' target='_blank'>open popup</a>"
    "<button onclick=\"for(let i=0;i<5;i++)window.open('/opened?n='+i)\">five popups</button>",
}

STORE_SCRIPT = """<h1>Private %(m)s</h1><input id=field placeholder='private input'>
<script>
const m = %(mj)s;
document.cookie = 'tarnsession=' + m + '; path=/';
document.cookie = 'tarnpersist=' + m + '; max-age=86400; path=/';
localStorage.setItem('tarn', m);
const r = indexedDB.open('tarn', 1);
r.onupgradeneeded = () => r.result.createObjectStore('s');
r.onsuccess = () => { const t = r.result.transaction('s', 'readwrite'); t.objectStore('s').put(m, 'k'); };
if (self.caches) caches.open('tarn').then(c => c.put('/cached-' + m, new Response(m)));
document.title = 'stored ' + m;
</script>"""


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        LOG.write("%s %s\n" % (self.log_date_time_string(), fmt % args))

    def note(self, url):
        LOG.write(f"{self.log_date_time_string()} REQ {self.command} {url} cookie={self.headers.get('Cookie', '')!r}\n")

    def send(self, status, body, ctype="text/html; charset=utf-8", headers=()):
        data = body if isinstance(body, bytes) else (HEAD + body).encode()
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "max-age=3600" if ctype.startswith("text/javascript") else "no-store")
        for k, v in headers:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def do_CONNECT(self):
        self.note("CONNECT " + self.path)
        host, _, port = self.path.rpartition(":")
        target = ("127.0.0.1", TLS_PORT) if host.endswith(".test") and TLS_CERT else (host, int(port or 443))
        try:
            upstream = socket.create_connection(target, timeout=10)
        except OSError:
            self.send_response(502)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.send_response(200, "Connection established")
        self.end_headers()
        self.close_connection = True
        pipe(self.connection, upstream)

    def do_GET(self):
        url = urlsplit(self.path)
        host = url.hostname or self.headers.get("Host", "")
        self.note(f"http://{host}{url.path}{'?' + url.query if url.query else ''}")
        query = parse_qs(url.query)
        path = url.path
        if path == "/pixel":
            return self.send(200, PIXEL, "image/gif")
        if path == "/ad.js":
            return self.send(200, b"document.title='AD-RAN'", "text/javascript")
        if path == "/auth-third":
            via = "".join(c for c in query.get("via", ["other.test"])[0] if c.isalnum() or c in ".:-")
            return self.send(200, f"<h1>Third-party sign-in</h1><img alt=auth src='http://{via}/auth'>"
                             f"<iframe src='http://{via}/auth'></iframe>")
        if path == "/slow":
            time.sleep(6)
            return self.send(200, "<h1>SLOW-LOADED</h1><script>document.title='SLOW-JS-RAN'</script>")
        if path == "/plain":
            return self.send(200, "<h1>PLAIN-HTTP-LOADED</h1>")
        if path == "/redirect-local":
            return self.send(302, "", headers=[("Location", f"{LOCAL}/local-from-public")])
        if path == "/cookie":
            sent = self.headers.get("Cookie", "")
            headers = [("Set-Cookie", f"tarn={query['set'][0]}; max-age=86400; path=/")] if "set" in query else []
            return self.send(200, f"<h1>cookie: {sent or 'none'}</h1>", headers=headers)
        if path == "/auth":
            expected = "Basic " + base64.b64encode(b"tarn:test").decode()
            if self.headers.get("Authorization") == expected:
                return self.send(200, "<h1>SIGNED-IN</h1>")
            return self.send(401, "<h1>sign in</h1>", headers=[("WWW-Authenticate", 'Basic realm="tarn test"')])
        if path == "/download":
            return self.send(200, b"tarn download\n", "application/octet-stream",
                             headers=[("Content-Disposition", 'attachment; filename="../../evil:name.txt"')])
        if path == "/private":
            marker = query.get("m", ["none"])[0]
            safe = "".join(c for c in marker if c.isalnum())
            return self.send(200, STORE_SCRIPT % {"m": safe, "mj": repr(safe)})
        if path in PAGES:
            return self.send(200, PAGES[path])
        return self.send(200, f"<h1>{host}{path}</h1>")

    do_POST = do_GET


def pipe(a, b):
    def copy(src, dst):
        try:
            while data := src.recv(65536):
                dst.sendall(data)
        except OSError:
            pass
        finally:
            for sock in (src, dst):
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    other = threading.Thread(target=copy, args=(b, a), daemon=True)
    other.start()
    copy(a, b)
    other.join()


if TLS_CERT:
    tls = ThreadingHTTPServer(("127.0.0.1", TLS_PORT), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(TLS_CERT, TLS_KEY)
    tls.socket = context.wrap_socket(tls.socket, server_side=True)
    threading.Thread(target=tls.serve_forever, daemon=True).start()
ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
