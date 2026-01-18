import os
import socket
import sys
from datetime import datetime


def log(level: str, msg: str) -> None:
    ts = datetime.utcnow().strftime("%d %b %Y %H:%M:%S.%f")[:-3]
    print(f"[{os.getpid()}] {ts} {level} {msg}", flush=True)


port = 6379
try:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", port))
    sock.listen(1)
    log("*", f"Ready to accept connections at 127.0.0.1:{port}")
except Exception as exc:
    log("#", f"Failed to bind port {port}: {exc}")
    sys.exit(1)

while True:
    try:
        conn, addr = sock.accept()
        log("-", f"Accepted connection from {addr[0]}:{addr[1]}")
        conn.close()
    except Exception:
        pass
