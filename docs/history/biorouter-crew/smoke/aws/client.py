#!/usr/bin/env python3
import socket
import sys

connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
connection.connect("/run/crew-smoke/broker.sock")
connection.sendall(sys.argv[1].encode() + b"\n")
print(connection.makefile("r").readline().rstrip())
