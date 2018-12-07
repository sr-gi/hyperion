import socket
from lib.conntools import read_logs


def hand_logs(sock_path):
    # Open socket connection
    log_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM, 0)
    log_sock.connect(sock_path)
    for log in read_logs(log_sock):
        yield log

