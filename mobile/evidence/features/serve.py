import http.server, sys, time, os
class H(http.server.SimpleHTTPRequestHandler):
    def parse_request(self):
        ok = super().parse_request()
        if ok and self.path.startswith('http://'):
            self.path = '/' + self.path.split('/', 3)[3] if self.path.count('/') >= 3 else '/'
        return ok
    def do_CONNECT(self):
        self.send_error(405)
    def copyfile(self, src, dst):
        if self.path.startswith('/big.bin'):
            while True:
                b = src.read(65536)
                if not b: break
                dst.write(b); time.sleep(0.25)
        else:
            super().copyfile(src, dst)
    def guess_type(self, path):
        return 'application/octet-stream' if path.endswith(('.bin','.zip')) else super().guess_type(path)
os.chdir(sys.argv[1])
http.server.ThreadingHTTPServer(('0.0.0.0', 8790), H).serve_forever()
