#!/usr/bin/env python3
"""Real GTK/AT-SPI acceptance. Run under xvfb-run + dbus-run-session, never on a user's desktop."""
import argparse
import base64
import json
import os
from pathlib import Path
import queue
import re
import signal
import subprocess
import tempfile
import threading
import time


def fixture(work):
    import gi
    gi.require_version("Gtk", "3.0")
    from gi.repository import Gtk, GLib, Gdk, Atk
    GLib.set_prgname("biorouter-computer-use-fixture")
    window = Gtk.Window(title="BioRouter Computer Use Fixture")
    window.set_default_size(640, 480)
    window.connect("destroy", Gtk.main_quit)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
    window.add(box)
    entry = Gtk.Entry()
    entry.get_accessible().set_name("FixtureInput")
    box.pack_start(entry, False, False, 0)
    button = Gtk.Button(label="Apply fixture")
    button.connect("clicked", lambda _: (work / "result.txt").write_text(entry.get_text()))
    box.pack_start(button, False, False, 0)
    drag = Gtk.EventBox()
    drag_layout = Gtk.Fixed()
    drag_marker = Gtk.Label(label="Drag me")
    drag_layout.put(drag_marker, 30, 10)
    drag.add(drag_layout)
    drag.set_size_request(320, 60)
    drag.get_accessible().set_name("FixtureDrag")
    drag.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK | Gdk.EventMask.POINTER_MOTION_MASK)
    gesture = {"down": False, "moves": 0, "released": False, "start_x": 0, "end_x": 0}
    def drag_event(widget, event):
        if event.type == Gdk.EventType.BUTTON_PRESS and event.button == 1:
            gesture.update(down=True, moves=0, released=False, start_x=event.x, end_x=event.x)
        elif event.type == Gdk.EventType.MOTION_NOTIFY and gesture["down"] and event.state & Gdk.ModifierType.BUTTON1_MASK:
            gesture["moves"] += 1
            gesture["end_x"] = event.x
        elif event.type == Gdk.EventType.BUTTON_RELEASE and event.button == 1 and gesture["down"]:
            gesture.update(down=False, released=True, end_x=event.x)
        drag_layout.move(drag_marker, int(gesture["end_x"]), 10)
        (work / "drag-result.json").write_text(json.dumps(gesture))
        return True
    drag.connect("button-press-event", drag_event)
    drag.connect("motion-notify-event", drag_event)
    drag.connect("button-release-event", drag_event)
    box.pack_start(drag, False, False, 0)
    scroller = Gtk.ScrolledWindow()
    view = Gtk.TextView()
    view.set_editable(False)
    view.get_accessible().set_name("FixtureScroll")
    view.get_buffer().set_text("\n".join(f"Fixture scroll row {i:04d}" for i in range(400)))
    scroller.add(view)
    box.pack_start(scroller, True, True, 0)
    adjustment = scroller.get_vadjustment()
    adjustment.connect("value-changed", lambda value: (work / "scroll.txt").write_text(str(value.get_value())))
    (work / "scroll.txt").write_text("0")
    window.show_all()
    window.present()
    def ready():
        surface = drag.get_accessible().get_extents(Atk.CoordType.SCREEN)
        frame = window.get_accessible().get_extents(Atk.CoordType.SCREEN)
        (work / "drag-geometry.json").write_text(json.dumps({"from_x": surface.x - frame.x + 30,
            "from_y": surface.y - frame.y + surface.height / 2,
            "to_x": surface.x - frame.x + 150, "to_y": surface.y - frame.y + surface.height / 2}))
        (work / "ready").write_text("ready")
        return False
    GLib.timeout_add(500, ready)
    Gtk.main()


class Client:
    def __init__(self, binary, env):
        self.process = subprocess.Popen([str(binary), "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, env=env, start_new_session=True)
        self.lines = queue.Queue()
        self.request_id = 0
        def read():
            for line in self.process.stdout:
                self.lines.put(line)
        threading.Thread(target=read, daemon=True).start()
        self.request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "BioRouter-fixture", "version": "1"}})

    def request(self, method, params):
        self.request_id += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.request_id, "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            reply = json.loads(self.lines.get(timeout=max(0.1, deadline - time.monotonic())))
            if reply.get("id") == self.request_id:
                if "error" in reply:
                    raise RuntimeError(reply["error"])
                return reply["result"]
        raise TimeoutError(method)

    def call(self, name, args, allow_error=False):
        result = self.request("tools/call", {"name": name, "arguments": args})
        if result.get("isError") and not allow_error:
            raise RuntimeError(f"{name}: {result}")
        return result

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            self.process.wait(timeout=10)


def eventually(predicate, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise TimeoutError("Fixture state did not change before its deadline")


def validate_drag(result):
    if not result.get("released") or result.get("down") or result.get("moves", 0) < 2 or abs(result.get("end_x", 0) - result.get("start_x", 0) - 120) > 3:
        raise AssertionError(f"Independent child drag did not complete the requested 120px gesture: {result}")


def main(directory, report):
    if not os.environ.get("DISPLAY") or not os.environ.get("DBUS_SESSION_BUS_ADDRESS"):
        raise RuntimeError("Run with xvfb-run -a dbus-run-session -- /usr/bin/python3 ...")
    with tempfile.TemporaryDirectory(prefix="biorouter-linux-fixture-") as temp:
        work = Path(temp)
        env = dict(os.environ, NO_AT_BRIDGE="0", GTK_MODULES="atk-bridge", XDG_SESSION_TYPE="x11")
        env.setdefault("XDG_RUNTIME_DIR", str(work / "runtime"))
        Path(env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700, parents=True, exist_ok=True)
        manager = subprocess.Popen(["openbox", "--sm-disable"], env=env, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, start_new_session=True)
        gui = subprocess.Popen(["/usr/bin/python3", __file__, "--fixture", str(work)], env=env, start_new_session=True)
        client = wayland = None
        try:
            eventually(lambda: (work / "ready").exists())
            client = Client(directory / "ocu", env)
            eventually(lambda: "BioRouter Computer Use Fixture" in json.dumps(client.call("list_apps", {}, allow_error=True)))
            app = str(gui.pid)
            def state():
                result = client.call("get_app_state", {"app": app, "max_tree_nodes": 100})
                return "\n".join(c.get("text", "") for c in result["content"])
            def element(label, text):
                match = re.search(r"^\s*(\d+)\s+.*" + re.escape(label), text, re.MULTILINE)
                if not match:
                    raise AssertionError(f"Missing AT-SPI element {label}: {text}")
                return match.group(1)
            tree = state()
            print("PASS: real GTK app discovered through AT-SPI and accessibility tree received", flush=True)
            client.call("set_value", {"app": app, "element_index": element("FixtureInput", tree), "value": "BioRouter AT-SPI verified"})
            tree = state()
            client.call("click", {"app": app, "element_index": element("Apply fixture", tree), "click_method": "accessibility"})
            eventually(lambda: (work / "result.txt").exists())
            if (work / "result.txt").read_text() != "BioRouter AT-SPI verified":
                raise AssertionError("Independent GTK fixture text/click result did not match")
            print("PASS: editable text and accessibility click independently confirmed by GTK fixture", flush=True)
            tree = state()
            scroll_index = element("FixtureScroll", tree)
            client.call("click", {"app": app, "element_index": scroll_index})
            tree = state()
            print("Scroll target snapshot: " + tree, flush=True)
            client.call("scroll", {"app": app, "element_index": element("FixtureScroll", tree), "direction": "down", "pages": 2})
            eventually(lambda: float((work / "scroll.txt").read_text() or "0") > 0)
            print("PASS: GTK scroll adjustment independently changed", flush=True)
            state()
            drag_coordinates = json.loads((work / "drag-geometry.json").read_text())
            client.call("drag", {"app": app, **drag_coordinates})
            def drag_finished():
                path = work / "drag-result.json"
                try:
                    return json.loads(path.read_text()).get("released", False)
                except (FileNotFoundError, json.JSONDecodeError):
                    return False
            try:
                eventually(drag_finished, timeout=10)
            finally:
                path = work / "drag-result.json"
                report.with_name(report.stem + "-independent-drag.json").write_text(
                    path.read_text() if path.exists() else json.dumps({"error": "No pointer event reached FixtureDrag"}))
            drag_result = json.loads((work / "drag-result.json").read_text())
            validate_drag(drag_result)
            print("PASS: GTK child received pressed drag motion and release at displaced endpoint", flush=True)
            capture = client.call("screen_capture", {"window_title": "BioRouter Computer Use Fixture"})
            images = [base64.b64decode(c["data"]) for c in capture["content"] if c.get("type") == "image"]
            if len(images) != 1 or not images[0].startswith(b"\x89PNG"):
                raise AssertionError("Native window capture did not return one PNG")
            report.with_suffix(".png").write_bytes(images[0])
            import gi
            gi.require_version("GdkPixbuf", "2.0")
            from gi.repository import GdkPixbuf
            loader = GdkPixbuf.PixbufLoader.new_with_type("png")
            loader.write(images[0]); loader.close()
            pixels = loader.get_pixbuf()
            if pixels.get_width() < 300 or pixels.get_height() < 100 or len(set(pixels.get_pixels())) < 8:
                raise AssertionError("Native fixture screenshot is too small or blank")
            print("PASS: native window capture decoded as nonblank fixture PNG", flush=True)
            wayland_env = dict(env, XDG_SESSION_TYPE="wayland")
            doctor = subprocess.run([str(directory / "ocu"), "doctor", "--json"], env=wayland_env,
                                    capture_output=True, text=True, timeout=15, check=True)
            diagnostics = json.loads(doctor.stdout)
            if diagnostics.get("state") != "unsupported_environment" or diagnostics.get("capture_available"):
                raise AssertionError(f"Wayland doctor did not disclose unsupported capture: {diagnostics}")
            wayland = Client(directory / "ocu", wayland_env)
            denied = wayland.call("screen_capture", {}, allow_error=True)
            if not denied.get("isError") or "unsupported" not in json.dumps(denied).lower() or any(c.get("type") == "image" for c in denied["content"]):
                raise AssertionError("Unsupported Wayland capture did not fail explicitly without pixels")
            evidence = {"status": "passed", "validated": True, "target": json.loads((directory / "manifest.json").read_text())["target"],
                        "checks": ["AT-SPI discovery and tree", "set_value", "accessibility click", "independent GTK text", "scroll adjustment changed", "independent child drag gesture", "nonblank window PNG", "explicit Wayland unsupported + doctor"],
                        "not_validated": ["native GNOME/KDE Wayland", "mixed DPI", "multiple displays"]}
            report.write_text(json.dumps(evidence, indent=2)); print(json.dumps(evidence))
        finally:
            for connection in (wayland, client):
                if connection:
                    connection.close()
            for process in (gui, manager):
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    process.wait(timeout=10)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, nargs="?")
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--report", type=Path, default=Path("computer-use-linux-fixture.json"))
    args = parser.parse_args()
    if args.fixture:
        fixture(args.fixture)
    else:
        main(args.directory.resolve(), args.report)
