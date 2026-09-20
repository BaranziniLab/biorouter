import AppKit
import SwiftUI

@main
enum StandaloneCursorMain {
    static func main() {
        let application = NSApplication.shared
        application.setActivationPolicy(.regular)

        let delegate = StandaloneCursorAppDelegate()
        application.delegate = delegate
        application.run()
    }
}

@MainActor
final class StandaloneCursorAppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        installMenu()

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1360, height: 860),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Standalone Cursor"
        window.center()
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 1180, height: 760)
        window.contentView = NSHostingView(rootView: StandaloneCursorRootView())

        self.window = window
        window.orderFrontRegardless()
        window.makeKeyAndOrderFront(nil)
        NSRunningApplication.current.activate(options: [.activateAllWindows])
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    private func installMenu() {
        let mainMenu = NSMenu()
        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)

        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Quit Standalone Cursor", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appMenuItem.submenu = appMenu

        NSApp.mainMenu = mainMenu
    }
}
