"""Loopback static server for the blind A/B page, with HTTP Range support.

Python's http.server answers every request with the whole file, so a browser cannot
seek in a video it serves. This one honours single "Range: bytes=a-b" requests, serves
ab.html from this directory, and accepts the page's verdict log as POST /log/<name>.json
(written into the served directory).

Usage: python3 serve.py CLIP_DIR [--port 8770]
Then open http://127.0.0.1:8770/ab.html?a=one.mp4&b=two.mp4
"""

import argparse
import functools
import os
import re
import shutil
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
LOG_NAME = re.compile(r"^/log/([A-Za-z0-9._-]{1,80}\.json)$")
MAX_LOG_BYTES = 1 << 20


class Handler(SimpleHTTPRequestHandler):
    def translate_path(self, path):
        if path.split("?", 1)[0] == "/ab.html":
            return os.path.join(HERE, "ab.html")
        return super().translate_path(path)

    def do_GET(self):
        rng = self.headers.get("Range")
        m = re.fullmatch(r"bytes=(\d*)-(\d*)", rng.strip()) if rng else None
        path = self.translate_path(self.path)
        if not m or not os.path.isfile(path) or m.group(1) == m.group(2) == "":
            super().do_GET()
            return
        size = os.path.getsize(path)
        if m.group(1):
            start = int(m.group(1))
            end = min(int(m.group(2)), size - 1) if m.group(2) else size - 1
        else:
            start, end = max(0, size - int(m.group(2))), size - 1
        if start > end or start >= size:
            self.send_response(HTTPStatus.REQUESTED_RANGE_NOT_SATISFIABLE)
            self.send_header("Content-Range", f"bytes */{size}")
            self.end_headers()
            return
        self.send_response(HTTPStatus.PARTIAL_CONTENT)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        with open(path, "rb") as f:
            f.seek(start)
            remaining = end - start + 1
            while remaining > 0:
                chunk = f.read(min(1 << 16, remaining))
                if not chunk:
                    break
                self.wfile.write(chunk)
                remaining -= len(chunk)

    def end_headers(self):
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_POST(self):
        m = LOG_NAME.match(self.path)
        length = int(self.headers.get("Content-Length", "0") or 0)
        if not m or not 0 < length <= MAX_LOG_BYTES:
            self.send_error(HTTPStatus.BAD_REQUEST, "expected POST /log/<name>.json with a body under 1 MiB")
            return
        body = self.rfile.read(length)
        tmp = os.path.join(self.directory, m.group(1) + ".part")
        with open(tmp, "wb") as f:
            f.write(body)
        shutil.move(tmp, os.path.join(self.directory, m.group(1)))
        self.send_response(HTTPStatus.NO_CONTENT)
        self.end_headers()


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("root")
    p.add_argument("--port", type=int, default=8770)
    a = p.parse_args()
    handler = functools.partial(Handler, directory=os.path.abspath(a.root))
    ThreadingHTTPServer(("127.0.0.1", a.port), handler).serve_forever()


if __name__ == "__main__":
    main()
