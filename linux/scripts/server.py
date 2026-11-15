import base64, http.server, os, ssl, subprocess, sys, threading, time

class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, fmt, *args):
        headers = getattr(self, "headers", None)
        sys.stderr.write("%s %s %s %s\n" % (headers.get("Host", "") if headers else "-", getattr(self, "command", "-"), getattr(self, "path", "-"), fmt % args))
        sys.stderr.flush()

    def do_GET(self):
        if self.path.startswith("/auth"):
            expected = "Basic " + base64.b64encode(b"tarn:secret").decode()
            if self.headers.get("Authorization") != expected:
                self.send_response(401)
                self.send_header("WWW-Authenticate", 'Basic realm="tarn-test"')
                self.end_headers()
                self.wfile.write(b"no")
                return
            return self.reply(b"<title>signed in</title><h1>signed in</h1>", "text/html")
        if self.path.startswith("/slow"):
            time.sleep(6)
            return self.reply(b"<title>slow</title><h1>SLOW-LOADED</h1><script>document.title='SLOW-JS-RAN'</script>", "text/html")
        if self.path.startswith("/bigdownload"):
            chunk, count = b"x" * 65536, 160
            self.send_response(200)
            self.send_header("Content-Type", "application/zip")
            self.send_header("Content-Length", str(len(chunk) * count))
            self.send_header("Content-Disposition", 'attachment; filename="photos.zip"')
            self.end_headers()
            for _ in range(count):
                self.wfile.write(chunk)
                self.wfile.flush()
                time.sleep(0.1)
            return
        if self.path.startswith("/download"):
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", 'attachment; filename="../evil‮gpj.sh"'.encode("latin-1", "replace").decode("latin-1"))
            self.end_headers()
            self.wfile.write(b"#!/bin/sh\necho downloaded\n")
            return
        if self.path.startswith("/pixel?"):
            return self.reply(base64.b64decode("R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw=="), "image/gif")
        if self.path.startswith("/pixel") or self.path.startswith("/insecure"):
            return self.reply(b"<title>insecure loaded</title>reached", "text/html")
        return super().do_GET()

    def reply(self, body, kind):
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

def serve(port, tls=False):
    server = http.server.ThreadingHTTPServer(("0.0.0.0", port), Handler)
    if tls:
        if not os.path.exists("/tmp/cert.pem"):
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost,DNS:trusted.test",
                            "-keyout", "/tmp/key.pem", "-out", "/tmp/cert.pem", "-days", "2"], check=True, capture_output=True)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain("/tmp/cert.pem", "/tmp/key.pem")
        server.socket = context.wrap_socket(server.socket, server_side=True)
    server.serve_forever()

port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
threading.Thread(target=serve, args=(8443, True), daemon=True).start()
serve(port)
