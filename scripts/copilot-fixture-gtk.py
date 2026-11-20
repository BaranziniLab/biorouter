#!/usr/bin/env python3
"""Biorouter Copilot's GTK fixture app: the app the Linux acceptance drives (AD-21).

It is the app under test, not the test. crates/biorouter-mcp/tests/computer_use_fixture.rs
starts it, drives it through Copilot's own tools and reads every outcome from the state file
this app writes, never from a tool's own report. The Windows twin is
scripts/copilot-fixture-winforms.cs; both keep the same names, titles and state keys.

Run it only inside a throwaway X11 session with the accessibility bridge on
(xvfb-run, dbus-run-session, at-spi2-core and a window manager), never on a person's desktop:

    copilot-fixture-gtk.py WORK_DIR

What it offers, and the state key each one writes to WORK_DIR/state.json (beside pid, and app:
the process name list_apps shows):
  - "FixtureInput", a text field                      input
  - "Apply fixture", a button copying the field       applied
  - "Fixture option", a checkbox                       checked
  - "FixtureSecret", a password field                  secret_length (never its text)
  - "FixtureList", a list reordered by drag            order
  - "Drag me", a button whose drag carries text        (dropped, in the other window)
  - "FixtureScroll", a long scrolling text             scroll
  - the pointer's left button, read every 100 ms       pointer_pressed
A second window, "Biorouter Copilot Fixture Drop", holds "FixtureDrop", a text field that
accepts the dragged text (dropped). WORK_DIR/ready appears once both windows are shown.
"""
import json
import os
import sys
from pathlib import Path

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

TITLE = "Biorouter Copilot Fixture"
DROP_TITLE = "Biorouter Copilot Fixture Drop"
DRAG_TEXT = "Dropped by Copilot"
ITEMS = ["Alpha", "Beta", "Gamma"]
ROW_TARGET = "BIOROUTER_FIXTURE_ROW"


def main(work: Path) -> None:
    GLib.set_prgname("biorouter-copilot-fixture")
    GLib.set_application_name(TITLE)
    state = {
        "pid": os.getpid(),
        # The process name the desktop's accessibility service reports for this app.
        "app": os.path.basename(os.readlink("/proc/self/exe")),
        "input": "",
        "applied": "",
        "checked": False,
        "secret_length": 0,
        "order": list(ITEMS),
        "dropped": "",
        "scroll": 0.0,
        "pointer_pressed": False,
    }

    def save() -> None:
        temporary = work / "state.json.tmp"
        temporary.write_text(json.dumps(state))
        os.replace(temporary, work / "state.json")

    def named(widget: Gtk.Widget, name: str) -> Gtk.Widget:
        widget.get_accessible().set_name(name)
        return widget

    window = Gtk.Window(title=TITLE)
    window.set_default_size(520, 600)
    window.move(40, 40)
    window.connect("destroy", Gtk.main_quit)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, margin=12)
    window.add(box)

    entry = named(Gtk.Entry(), "FixtureInput")
    entry.connect("changed", lambda field: (state.update(input=field.get_text()), save()))
    box.pack_start(entry, False, False, 0)

    apply = Gtk.Button(label="Apply fixture")
    apply.connect("clicked", lambda _: (state.update(applied=entry.get_text()), save()))
    box.pack_start(apply, False, False, 0)

    option = Gtk.CheckButton(label="Fixture option")
    option.connect("toggled", lambda button: (state.update(checked=button.get_active()), save()))
    box.pack_start(option, False, False, 0)

    secret = named(Gtk.Entry(), "FixtureSecret")
    secret.set_visibility(False)
    secret.set_input_purpose(Gtk.InputPurpose.PASSWORD)
    secret.connect("changed", lambda field: (state.update(secret_length=len(field.get_text())), save()))
    box.pack_start(secret, False, False, 0)

    # A list reordered by dragging one row onto another. The drop is handled here, not by
    # GtkListStore's own row drag, so dropping X anywhere on Y always puts X where Y was: an
    # outcome the test can assert exactly, whichever part of the row the pointer lands on.
    store = Gtk.ListStore(str)
    for item in ITEMS:
        store.append([item])
    view = named(Gtk.TreeView(model=store), "FixtureList")
    view.append_column(Gtk.TreeViewColumn("Item", Gtk.CellRendererText(), text=0))
    row_target = Gtk.TargetEntry.new(ROW_TARGET, Gtk.TargetFlags.SAME_WIDGET, 0)
    view.enable_model_drag_source(Gdk.ModifierType.BUTTON1_MASK, [row_target], Gdk.DragAction.COPY)
    view.enable_model_drag_dest([row_target], Gdk.DragAction.COPY)

    def row_drag_get(widget, context, data, info, time):
        model, selected = widget.get_selection().get_selected()
        if selected is not None:
            data.set(data.get_target(), 8, model[selected][0].encode())
        widget.stop_emission_by_name("drag-data-get")

    def row_drag_received(widget, context, x, y, data, info, time):
        widget.stop_emission_by_name("drag-data-received")
        moved = bytes(data.get_data() or b"").decode()
        names = [row[0] for row in store]
        destination = widget.get_dest_row_at_pos(x, y)
        if moved not in names or destination is None:
            context.finish(False, False, time)
            return
        path, _position = destination
        target = store[path.get_indices()[0]][0]
        if target != moved:
            names.remove(moved)
            names.insert(names.index(target), moved)
            store.clear()
            for name in names:
                store.append([name])
        state["order"] = names
        save()
        context.finish(True, False, time)

    view.connect("drag-data-get", row_drag_get)
    view.connect("drag-data-received", row_drag_received)
    box.pack_start(view, False, False, 0)

    # A button whose drag carries text to the other window's field.
    source = Gtk.Button(label="Drag me")
    source.drag_source_set(Gdk.ModifierType.BUTTON1_MASK, [], Gdk.DragAction.COPY)
    source.drag_source_add_text_targets()
    source.connect("drag-data-get", lambda _w, _c, data, _i, _t: data.set_text(DRAG_TEXT, -1))
    box.pack_start(source, False, False, 0)

    scroller = Gtk.ScrolledWindow()
    scroller.set_min_content_height(200)
    text = named(Gtk.TextView(), "FixtureScroll")
    text.set_editable(False)
    text.get_buffer().set_text("\n".join(f"Fixture scroll row {i:04d}" for i in range(400)))
    scroller.add(text)
    scroller.get_vadjustment().connect(
        "value-changed", lambda adjustment: (state.update(scroll=adjustment.get_value()), save())
    )
    box.pack_start(scroller, True, True, 0)

    drop_window = Gtk.Window(title=DROP_TITLE)
    drop_window.set_default_size(320, 120)
    drop_window.move(620, 40)
    drop_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, margin=12)
    drop_window.add(drop_box)
    drop = named(Gtk.Entry(), "FixtureDrop")
    drop.connect("changed", lambda field: (state.update(dropped=field.get_text()), save()))
    drop_box.pack_start(drop, False, False, 0)

    # A press the release guard failed to lift would show here as a button still down.
    def read_pointer() -> bool:
        gdk_window = window.get_window()
        seat = Gdk.Display.get_default().get_default_seat()
        if gdk_window is not None and seat is not None:
            _, _, _, mask = gdk_window.get_device_position(seat.get_pointer())
            pressed = bool(mask & Gdk.ModifierType.BUTTON1_MASK)
            if pressed != state["pointer_pressed"]:
                state["pointer_pressed"] = pressed
                save()
        return True

    def ready() -> bool:
        save()
        (work / "ready").write_text("ready")
        return False

    window.show_all()
    drop_window.show_all()
    window.present()
    save()
    GLib.timeout_add(100, read_pointer)
    GLib.timeout_add(600, ready)
    Gtk.main()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(Path(sys.argv[1]))
