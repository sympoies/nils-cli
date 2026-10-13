// Synthetic resident-only AppKit fixture. Compile and run only in a scheduled GUI slot.
// swiftc coordinate_overlay.swift -o coordinate-overlay -framework AppKit
// Run with --overlap for a second same-application window covering the primary.
import AppKit

final class Fixture: NSObject, NSApplicationDelegate {
    private var windows: [NSWindow] = []
    private var overlay: NSView?
    private var count = 0
    private let counter = NSTextField(labelWithString: "Counter: 0")

    func applicationDidFinishLaunching(_ notification: Notification) {
        let menu = NSMenu()
        let appItem = NSMenuItem()
        menu.addItem(appItem)
        appItem.submenu = NSMenu()
        appItem.submenu?.addItem(withTitle: "Quit fixture", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        NSApp.mainMenu = menu

        let primary = window(title: "Coordinate Fixture", x: 200, y: 200)
        let content = primary.contentView!
        counter.frame = NSRect(x: 20, y: 150, width: 280, height: 24)
        content.addSubview(counter)
        let increment = NSButton(title: "Increment counter", target: self, action: #selector(incrementCounter))
        increment.frame = NSRect(x: 20, y: 20, width: 180, height: 32)
        content.addSubview(increment)
        for index in 1...2 {
            let control = NSButton(title: "Underlying synthetic control \(index)", target: nil, action: nil)
            control.frame = NSRect(x: 20, y: CGFloat(50 + index * 28), width: 280, height: 24)
            content.addSubview(control)
        }

        // Keep underlying views in the AX tree while covering their pixels.
        let cover = NSView(frame: content.bounds)
        cover.wantsLayer = true
        cover.layer?.backgroundColor = NSColor.gray.cgColor
        let label = NSTextField(labelWithString: "Opaque synthetic overlay")
        label.frame = NSRect(x: 20, y: 180, width: 300, height: 24)
        cover.addSubview(label)
        let dismiss = NSButton(title: "Dismiss overlay", target: self, action: #selector(dismissOverlay))
        dismiss.frame = NSRect(x: 20, y: 140, width: 180, height: 32)
        cover.addSubview(dismiss)
        content.addSubview(cover)
        overlay = cover
        primary.makeKeyAndOrderFront(nil)

        let overlapping = CommandLine.arguments.contains("--overlap")
        let sibling = window(title: "Sibling Fixture", x: overlapping ? 220 : 620, y: 200)
        let siblingLabel = NSTextField(labelWithString: "Synthetic sibling window")
        siblingLabel.frame = NSRect(x: 20, y: 100, width: 300, height: 24)
        sibling.contentView?.addSubview(siblingLabel)
        sibling.orderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        print("primary_window_id=\(primary.windowNumber) sibling_window_id=\(sibling.windowNumber)")
        fflush(stdout)
    }

    private func window(title: String, x: CGFloat, y: CGFloat) -> NSWindow {
        let result = NSWindow(contentRect: NSRect(x: x, y: y, width: 360, height: 220),
                              styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        result.title = title
        result.isReleasedWhenClosed = false
        windows.append(result)
        return result
    }

    @objc private func dismissOverlay() {
        overlay?.removeFromSuperview()
        overlay = nil
    }

    @objc private func incrementCounter() {
        count += 1
        counter.stringValue = "Counter: \(count)"
    }
}

let application = NSApplication.shared
let fixture = Fixture()
application.delegate = fixture
application.setActivationPolicy(.regular)
application.run()
