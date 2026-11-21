// Biorouter Copilot's AppKit fixture app: the app the macOS acceptance drives (AD-21, AD-22).
//
// It is the app under test, not the test. crates/biorouter-mcp/tests/computer_use_fixture.rs
// starts it, drives it through Copilot's own tools and reads every outcome from the state file
// this app writes, never from a tool's own report. Its twins are scripts/copilot-fixture-gtk.py
// (Linux) and scripts/copilot-fixture-winforms.cs (Windows); all three keep the same names,
// titles and state keys.
//
// macOS hosted runners cannot grant Accessibility to the Biorouter Copilot bundle, so CI only
// builds this app; the drive runs on a Mac whose owner allowed Biorouter Copilot (AD-22):
//
//   scripts/build-copilot-fixture-macos.sh
//   BIOROUTER_COMPUTER_USE_DIR=target/computer-use/darwin-arm64 \
//   BIOROUTER_COPILOT_FIXTURE='["target/copilot-fixture/Biorouter Copilot Fixture.app/Contents/MacOS/BiorouterCopilotFixture"]' \
//   cargo test -p biorouter-mcp --test computer_use_fixture -- --ignored --nocapture
//
// Usage: BiorouterCopilotFixture WORK_DIR
//
// State keys in WORK_DIR/state.json: pid, app (the name list_apps shows), input, applied,
// checked, secret_length (never the text), order, dropped, scroll, pointer_pressed.
// WORK_DIR/ready appears once both windows are shown.
import AppKit

let title = "Biorouter Copilot Fixture"
let dropTitle = "Biorouter Copilot Fixture Drop"
let dragText = "Dropped by Copilot"
let rowType = NSPasteboard.PasteboardType("org.biorouter.copilot-fixture.row")

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write("usage: BiorouterCopilotFixture WORK_DIR\n".data(using: .utf8)!)
    exit(64)
}
let work = URL(fileURLWithPath: CommandLine.arguments[1])

final class State {
    var input = "", applied = "", dropped = ""
    var checked = false, pointerPressed = false
    var secretLength = 0
    var order = ["Alpha", "Beta", "Gamma"]
    var scroll = 0.0

    func save() {
        let body: [String: Any] = [
            "pid": Int(ProcessInfo.processInfo.processIdentifier),
            "app": NSRunningApplication.current.localizedName ?? title,
            "input": input, "applied": applied, "checked": checked,
            "secret_length": secretLength, "order": order, "dropped": dropped,
            "scroll": scroll, "pointer_pressed": pointerPressed,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: body) else { return }
        let temporary = work.appendingPathComponent("state.json.tmp")
        try? data.write(to: temporary)
        _ = try? FileManager.default.replaceItemAt(
            work.appendingPathComponent("state.json"), withItemAt: temporary)
        if !FileManager.default.fileExists(atPath: work.appendingPathComponent("state.json").path) {
            try? FileManager.default.moveItem(
                at: temporary, to: work.appendingPathComponent("state.json"))
        }
    }
}
let state = State()

/// A list reordered by dragging one row onto another: dropping X anywhere on Y puts X where Y
/// was, so the test can assert the order exactly.
final class ListSource: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    weak var table: NSTableView?
    func numberOfRows(in tableView: NSTableView) -> Int { state.order.count }
    func tableView(_ tableView: NSTableView, objectValueFor column: NSTableColumn?, row: Int) -> Any? {
        state.order[row]
    }
    func tableView(_ tableView: NSTableView, pasteboardWriterForRow row: Int) -> NSPasteboardWriting? {
        let item = NSPasteboardItem()
        item.setString(state.order[row], forType: rowType)
        return item
    }
    func tableView(_ tableView: NSTableView, validateDrop info: NSDraggingInfo, proposedRow row: Int,
                   proposedDropOperation operation: NSTableView.DropOperation) -> NSDragOperation {
        tableView.setDropRow(min(row, state.order.count - 1), dropOperation: .on)
        return .move
    }
    func tableView(_ tableView: NSTableView, acceptDrop info: NSDraggingInfo, row: Int,
                   dropOperation: NSTableView.DropOperation) -> Bool {
        guard let moved = info.draggingPasteboard.string(forType: rowType),
              state.order.contains(moved), row >= 0, row < state.order.count else { return false }
        let onto = state.order[row]
        if onto != moved {
            state.order.removeAll { $0 == moved }
            state.order.insert(moved, at: state.order.firstIndex(of: onto)!)
            tableView.reloadData()
        }
        state.save()
        return true
    }
}

/// A button whose drag carries text to the other window's field.
final class DragSource: NSView, NSDraggingSource {
    override init(frame: NSRect) {
        super.init(frame: frame)
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel("Drag me")
    }
    required init?(coder: NSCoder) { nil }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.controlAccentColor.withAlphaComponent(0.25).setFill()
        bounds.fill()
        ("Drag me" as NSString).draw(at: NSPoint(x: 10, y: 8), withAttributes: nil)
    }
    override func mouseDragged(with event: NSEvent) {
        let item = NSDraggingItem(pasteboardWriter: dragText as NSString)
        item.setDraggingFrame(bounds, contents: nil)
        beginDraggingSession(with: [item], event: event, source: self)
    }
    func draggingSession(_ session: NSDraggingSession,
                         sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        .copy
    }
}

/// A text field that takes dropped text.
final class DropField: NSTextField {
    override init(frame: NSRect) {
        super.init(frame: frame)
        registerForDraggedTypes([.string])
    }
    required init?(coder: NSCoder) { nil }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        sender.draggingPasteboard.string(forType: .string) == nil ? [] : .copy
    }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard let text = sender.draggingPasteboard.string(forType: .string) else { return false }
        stringValue = text
        state.dropped = text
        state.save()
        return true
    }
}

final class Fixture: NSObject, NSApplicationDelegate, NSTextFieldDelegate {
    let window = NSWindow(contentRect: NSRect(x: 60, y: 200, width: 520, height: 600),
                          styleMask: [.titled, .closable], backing: .buffered, defer: false)
    let dropWindow = NSWindow(contentRect: NSRect(x: 620, y: 660, width: 340, height: 120),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
    let input = NSTextField(frame: NSRect(x: 20, y: 550, width: 480, height: 24))
    let secret = NSSecureTextField(frame: NSRect(x: 20, y: 470, width: 480, height: 24))
    let drop = DropField(frame: NSRect(x: 20, y: 60, width: 300, height: 24))
    let list = ListSource()
    var scroller: NSScrollView?

    func applicationDidFinishLaunching(_ notification: Notification) {
        window.title = title
        let content = window.contentView!
        input.setAccessibilityLabel("FixtureInput")
        input.delegate = self
        content.addSubview(input)

        let apply = NSButton(title: "Apply fixture", target: self, action: #selector(applyField))
        apply.frame = NSRect(x: 20, y: 510, width: 160, height: 30)
        content.addSubview(apply)

        let option = NSButton(checkboxWithTitle: "Fixture option", target: self,
                              action: #selector(toggleOption(_:)))
        option.frame = NSRect(x: 200, y: 515, width: 200, height: 24)
        content.addSubview(option)

        secret.setAccessibilityLabel("FixtureSecret")
        secret.delegate = self
        content.addSubview(secret)

        let table = NSTableView(frame: NSRect(x: 0, y: 0, width: 480, height: 90))
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("Item"))
        column.title = "Item"
        column.width = 460
        table.addTableColumn(column)
        table.dataSource = list
        table.delegate = list
        table.registerForDraggedTypes([rowType])
        table.setDraggingSourceOperationMask(.move, forLocal: true)
        table.setAccessibilityLabel("FixtureList")
        list.table = table
        let tableScroll = NSScrollView(frame: NSRect(x: 20, y: 360, width: 480, height: 100))
        tableScroll.documentView = table
        content.addSubview(tableScroll)

        content.addSubview(DragSource(frame: NSRect(x: 20, y: 320, width: 160, height: 30)))

        let scroll = NSTextView.scrollableTextView()
        scroll.frame = NSRect(x: 20, y: 20, width: 480, height: 280)
        let text = scroll.documentView as! NSTextView
        text.isEditable = false
        text.string = (0..<400).map { String(format: "Fixture scroll row %04d", $0) }
            .joined(separator: "\n")
        text.setAccessibilityLabel("FixtureScroll")
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            forName: NSView.boundsDidChangeNotification, object: scroll.contentView, queue: .main
        ) { _ in
            state.scroll = Double(scroll.contentView.bounds.origin.y)
            state.save()
        }
        scroller = scroll
        content.addSubview(scroll)

        dropWindow.title = dropTitle
        drop.setAccessibilityLabel("FixtureDrop")
        drop.delegate = self
        dropWindow.contentView!.addSubview(drop)

        // Read every 100 ms: a value set through accessibility sends no text-change delegate
        // call, and a press the release guard failed to lift shows as a button still down.
        Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [self] _ in
            let pressed = NSEvent.pressedMouseButtons & 1 != 0
            let now = (input.stringValue, secret.stringValue.count, drop.stringValue)
            if pressed != state.pointerPressed || now.0 != state.input
                || now.1 != state.secretLength || now.2 != state.dropped {
                state.pointerPressed = pressed
                (state.input, state.secretLength, state.dropped) = now
                state.save()
            }
        }
        dropWindow.makeKeyAndOrderFront(nil)
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        state.save()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
            try? "ready".write(to: work.appendingPathComponent("ready"), atomically: true,
                               encoding: .utf8)
        }
    }

    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField else { return }
        if field === input { state.input = field.stringValue }
        if field === secret { state.secretLength = field.stringValue.count }
        if field === drop { state.dropped = field.stringValue }
        state.save()
    }

    @objc func applyField() {
        state.input = input.stringValue
        state.applied = input.stringValue
        state.save()
    }

    @objc func toggleOption(_ sender: NSButton) {
        state.checked = sender.state == .on
        state.save()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
}

let app = NSApplication.shared
let delegate = Fixture()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
