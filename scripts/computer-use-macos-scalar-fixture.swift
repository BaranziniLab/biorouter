// Synthetic macOS scalar fixture for Biorouter Copilot `set_value` acceptance.
//
// Three sliders, each a different contract:
//   supported-scalar      an app-implemented settable scalar that APPLIES the value
//   no-op-scalar          one that acknowledges the write and ignores it
//   unmodified-baseline   a stock NSSlider, handled entirely by AppKit
//
// The negative control is the point: `set_value` must distinguish a real
// mutation from an acknowledged no-op, and must never replay or retry an
// unconfirmed write. Driven by scripts/test-computer-use-macos-scalar-fixture.py.
//
// This lived only in /tmp until now, which made every scalar finding
// unreproducible from a clean clone.

import AppKit

let runID = UUID().uuidString
// Where the independent observation log goes. Overridable so a run can keep its
// evidence beside the rest of a receipt instead of in a shared temp directory.
let evidence = URL(fileURLWithPath:
    ProcessInfo.processInfo.environment["BIOROUTER_SCALAR_FIXTURE_LOG"]
        ?? FileManager.default.currentDirectoryPath + "/events.jsonl")
func record(_ data: [String: Any]) {
    var entry = data
    entry["timestamp"] = Date().timeIntervalSince1970
    entry["run_id"] = runID
    entry["pid"] = ProcessInfo.processInfo.processIdentifier
    guard let bytes = try? JSONSerialization.data(withJSONObject: entry, options: [.sortedKeys]) else { return }
    if !FileManager.default.fileExists(atPath: evidence.path) {
        try? FileManager.default.createDirectory(
            at: evidence.deletingLastPathComponent(), withIntermediateDirectories: true)
        _ = FileManager.default.createFile(atPath: evidence.path, contents: nil)
    }
    guard let handle = try? FileHandle(forWritingTo: evidence) else { return }
    defer { try? handle.close() }
    _ = try? handle.seekToEnd()
    try? handle.write(contentsOf: bytes + Data([10]))
}
func rectangle(_ rect: NSRect) -> [String: Double] {
    ["x": rect.origin.x, "y": rect.origin.y, "width": rect.width, "height": rect.height]
}
func scalarProbeValues(_ slider: NSSlider) -> [String: Any] {
    let ax = slider.accessibilityValue()
    var result: [String: Any] = [
        "control": slider.accessibilityIdentifier(),
        "class": String(describing: type(of: slider)),
        "model_double_value": slider.doubleValue,
        "ax_value": (ax as? NSNumber) ?? (ax as? NSString) ?? NSNull(),
        "ax_value_type": ax.map { String(describing: type(of: $0)) } ?? "nil",
        "frame": rectangle(slider.frame), "bounds": rectangle(slider.bounds),
        "minimum": slider.minValue, "maximum": slider.maxValue,
        "needs_display": slider.needsDisplay,
    ]
    if let cell = slider.cell as? NSSliderCell {
        result["cell_double_value"] = cell.doubleValue
        result["cell_class"] = String(describing: type(of: cell))
        result["cell_knob_rect"] = rectangle(cell.knobRect(flipped: slider.isFlipped))
        result["cell_track_rect"] = rectangle(cell.trackRect)
    }
    return result
}
func sample(_ slider: NSSlider, checkpoint: String, writeID: String? = nil) {
    var data = scalarProbeValues(slider)
    data["kind"] = "value_probe"
    data["checkpoint"] = checkpoint
    if let writeID { data["write_id"] = writeID }
    record(data)
}
final class ObservableSlider: NSSlider {
    /// An app-implemented settable scalar that ACKNOWLEDGES the write and ignores
    /// it, which is the case `set_value` must refuse rather than report success.
    var ignoresAccessibilityWrite = false
    override func setAccessibilityValue(_ value: Any?) {
        let writeID = UUID().uuidString
        sample(self, checkpoint: "before_setter", writeID: writeID)
        let before = doubleValue
        // NEVER call `super.setAccessibilityValue(_:)` here. On an NSView it writes
        // the per-instance accessibility attribute OVERRIDE store: it changes what
        // the element REPORTS to an AX client and never reaches NSSlider or its
        // cell. A fixture that calls it reports a value its own model contradicts,
        // permanently -- which is exactly how this control came to read ax=65 with
        // its knob still drawn at 20%, and why that reading was mistaken for a
        // runtime defect. A real settable scalar applies the value to its model.
        if !ignoresAccessibilityWrite {
            let requested = (value as? NSNumber)?.doubleValue
                ?? (value as? NSString)?.doubleValue
            if let requested { doubleValue = requested }
        }
        record(["kind": "accessibility_set_value", "control": accessibilityIdentifier(),
                "before": before, "requested": String(describing: value ?? "nil"),
                "requested_type": value.map { String(describing: type(of: $0)) } ?? "nil",
                "after": doubleValue, "ignored": ignoresAccessibilityWrite, "write_id": writeID])
        sample(self, checkpoint: "after_setter_immediate", writeID: writeID)
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            sample(self, checkpoint: "next_main_turn", writeID: writeID)
        }
        for delay in [0.05, 0.25, 1.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                guard let self else { return }
                sample(self, checkpoint: "after_\(delay)s", writeID: writeID)
            }
        }
    }
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        sample(self, checkpoint: "after_actual_slider_draw")
    }
}
final class Delegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var sliders: [NSSlider] = []
    var timer: Timer?
    var ticks = 0
    var lastValues: [String: Data] = [:]
    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 260, y: 240, width: 540, height: 365), styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        window.title = "BioRouter Synthetic Scalar Fixture"
        let root = window.contentView!
        let title = NSTextField(labelWithString: "Synthetic scalar model / accessibility comparison")
        title.frame = NSRect(x: 24, y: 315, width: 490, height: 26)
        root.addSubview(title)
        for (index, name) in ["Supported scalar", "Acknowledged no-op scalar", "Unmodified NSSlider baseline"].enumerated() {
            let y = 260 - index * 85
            let label = NSTextField(labelWithString: name)
            label.frame = NSRect(x: 24, y: y + 25, width: 480, height: 22)
            root.addSubview(label)
            let slider: NSSlider
            if index < 2 {
                let observed = ObservableSlider(value: 20, minValue: 0, maxValue: 100, target: nil, action: nil)
                observed.ignoresAccessibilityWrite = index == 1
                slider = observed
            } else {
                slider = NSSlider(value: 20, minValue: 0, maxValue: 100, target: nil, action: nil)
            }
            slider.frame = NSRect(x: 24, y: y, width: 480, height: 24)
            slider.setAccessibilityLabel(name)
            slider.setAccessibilityIdentifier(["supported-scalar", "no-op-scalar", "unmodified-baseline"][index])
            root.addSubview(slider)
            sliders.append(slider)
        }
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        record(["kind": "ready", "instrumentation": "model-ax-cell-knob-v1", "source": "instrumented.swift"])
        sliders.forEach { sample($0, checkpoint: "ready") }
        timer = Timer(timeInterval: 0.05, repeats: true) { [weak self] _ in
            guard let self else { return }
            self.ticks += 1
            for slider in self.sliders {
                let state = scalarProbeValues(slider)
                let key = slider.accessibilityIdentifier()
                let encoded = try? JSONSerialization.data(withJSONObject: state, options: [.sortedKeys])
                if encoded != self.lastValues[key] || self.ticks % 20 == 0 {
                    sample(slider, checkpoint: encoded != self.lastValues[key] ? "poll_changed" : "poll_1s")
                    self.lastValues[key] = encoded
                }
            }
        }
        RunLoop.main.add(timer!, forMode: .common)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
