import AppKit

final class ObservationWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let window = ObservationWindow(contentRect: NSRect(x: 200, y: 200, width: 420, height: 180),
                               styleMask: [.titled, .closable], backing: .buffered, defer: false)
window.title = "OCU Observation Fixture"
window.isReleasedWhenClosed = false
let input = NSTextField(frame: NSRect(x: 20, y: 75, width: 380, height: 30))
input.stringValue = "initial-中文🙂"
input.setAccessibilityLabel("Read-back input")
window.contentView?.addSubview(input)
window.orderFront(nil)
let statePath = CommandLine.arguments[1]
let timer = Timer.scheduledTimer(withTimeInterval: 0.03, repeats: true) { _ in
    let data = try! JSONSerialization.data(withJSONObject: ["text": input.stringValue])
    try? data.write(to: URL(fileURLWithPath: statePath), options: .atomic)
}
Timer.scheduledTimer(withTimeInterval: 120, repeats: false) { _ in app.terminate(nil) }
app.run()
