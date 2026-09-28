#!/usr/bin/env python3
"""Synthetic feasibility fixture, not a production authorization implementation."""
import grp
import json
import os
import pwd
import socket
import struct

STATE = "/var/lib/crew-smoke/events.jsonl"
SOCKET = "/run/crew-smoke/broker.sock"
agents = {}
events = []
if os.path.exists(STATE):
    with open(STATE, encoding="utf-8") as source:
        for line in source:
            event = json.loads(line)
            events.append(event)
            if event["action"] == "create":
                agents[event["agent"]] = event["uid"]


def append(action, uid, agent):
    event = {"seq": len(events) + 1, "action": action, "uid": uid, "agent": agent}
    with open(STATE, "a", encoding="utf-8") as destination:
        destination.write(json.dumps(event, separators=(",", ":")) + "\n")
        destination.flush()
        os.fsync(destination.fileno())
    events.append(event)
    return event


os.makedirs(os.path.dirname(SOCKET), exist_ok=True)
os.chmod(os.path.dirname(SOCKET), 0o755)
if os.path.exists(SOCKET):
    os.unlink(SOCKET)
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(SOCKET)
os.chown(SOCKET, 0, grp.getgrnam("crew-smoke").gr_gid)
os.chmod(SOCKET, 0o660)
server.listen(10)
while True:
    connection, _ = server.accept()
    try:
        connection.settimeout(5)
        pid, uid, gid = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        username = pwd.getpwuid(uid).pw_name
        reader = connection.makefile("rb")
        raw = reader.readline(65537)
        if len(raw) > 65536 or not raw.endswith(b"\n"):
            raise ValueError("invalid_frame")
        request = json.loads(raw)
        result = {"ok": True, "uid": uid, "username": username}
        if "username" in request and request["username"] != username:
            result.update(ok=False, error="claimed_identity_mismatch")
        elif request["action"] == "identity":
            pass
        elif request["action"] == "history":
            result["events"] = [event for event in events if event["seq"] > request.get("after", 0)]
        elif request["action"] == "create":
            agent = request["agent"]
            if agent in agents:
                result.update(ok=False, error="already_exists")
            else:
                result["event"] = append("create", uid, agent)
                agents[agent] = uid
        elif request["action"] == "invoke":
            agent = request["agent"]
            if agents.get(agent) != uid:
                result.update(ok=False, error="not_agent_owner")
            else:
                result["event"] = append("invoke", uid, agent)
        else:
            result.update(ok=False, error="unknown_action")
        connection.sendall((json.dumps(result) + "\n").encode())
    except Exception as error:
        connection.sendall((json.dumps({"ok": False, "error": str(error)}) + "\n").encode())
    finally:
        connection.close()
