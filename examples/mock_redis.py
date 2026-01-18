import socket
import time
import sys
import logging

logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(message)s')

port = 6379
try:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(('127.0.0.1', port))
    sock.listen(1)
    logging.info(f"Redis-mock listening on port {port}")
except Exception as e:
    logging.error(f"Failed to bind port {port}: {e}")
    sys.exit(1)

while True:
    try:
        conn, addr = sock.accept()
        logging.info(f"Connection from {addr}")
        conn.close()
    except:
        pass
