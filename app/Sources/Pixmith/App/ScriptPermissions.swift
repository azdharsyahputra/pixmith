import AppKit
import PixmithCore

// The front-end half of docs/LUA_HOST_API.md, section 1: a script that reads or writes a file, or runs a command, is
// held until the user answers an alert (Allow Once, Always Allow for This Script, Deny). The core calls the handler
// on the script's own thread, so the alert is presented on the main thread while that thread waits; the core leaves
// the wait out of the script's time limit.

enum ScriptPermissions {
    enum Answer: UInt32 {
        case deny = 0, once = 1, always = 2
    }

    /// Core access bits.
    static let read: UInt32 = 1, write: UInt32 = 2, execute: UInt32 = 4

    /// How a request is put to the user; the QA runner replaces it. Runs on the main thread.
    static var ask: (_ script: String, _ path: String, _ access: UInt32) -> Answer = presentAlert

    /// Number of alerts on screen (main thread). Progress sheets stay away while one is up.
    private(set) static var asking = 0

    private static var installed = false

    /// Registers the handler with the core (once). Call before any script runs.
    static func install() {
        guard !installed else { return }
        installed = true
        pixmith_script_set_permission_handler({ _, script, path, access in
            let s = script.map { String(cString: $0) } ?? "script"
            let p = path.map { String(cString: $0) } ?? ""
            return ScriptPermissions.answer(script: s, path: p, access: access).rawValue
        }, nil)
    }

    private final class Box {
        var value = Answer.deny
    }

    /// Called on the script's thread, or on the main thread when a script runs there (then the alert is shown at once,
    /// since waiting for the main queue would never end).
    fileprivate static func answer(script: String, path: String, access: UInt32) -> Answer {
        if Thread.isMainThread { return askOnMain(script, path, access) }
        let box = Box()
        let done = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            box.value = askOnMain(script, path, access)
            done.signal()
        }
        done.wait()
        return box.value
    }

    private static func askOnMain(_ script: String, _ path: String, _ access: UInt32) -> Answer {
        asking += 1
        defer { asking -= 1 }
        return ask(script, path, access)
    }

    // MARK: The alert

    /// What the alert calls a script: its file name when it is a path.
    static func displayName(_ script: String) -> String {
        script.hasPrefix("/") ? (script as NSString).lastPathComponent : script
    }

    static func verb(_ access: UInt32) -> String {
        access & execute != 0 ? "run" : access & write != 0 ? "write" : "read"
    }

    private static func shortened(_ text: String, _ limit: Int = 240) -> String {
        text.count <= limit ? text : String(text.prefix(limit / 2)) + "…" + String(text.suffix(limit / 2 - 1))
    }

    /// The question, as the alert shows it.
    static func title(script: String, path: String, access: UInt32) -> String {
        "“\(displayName(script))” wants to \(verb(access)) \(shortened(path))"
    }

    private static func detail(script: String, access: UInt32) -> String {
        let scope = access & execute != 0 ? "run any command" : access & write != 0 ? "read and write any file" : "read any file"
        var text = "Always Allow for This Script lets it \(scope) without asking again until you quit Pixmith."
        if script.hasPrefix("/") { text += "\n\nScript: \(script)" }
        return text
    }

    private static func presentAlert(script: String, path: String, access: UInt32) -> Answer {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = title(script: script, path: path, access: access)
        alert.informativeText = detail(script: script, access: access)
        // The first button is the default one (Return): Deny, the safe answer.
        alert.addButton(withTitle: "Deny")
        alert.addButton(withTitle: "Allow Once")
        alert.addButton(withTitle: "Always Allow for This Script")
        // Escape denies too (NSAlert only maps it to a button titled "Cancel").
        let escape = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { e in
            guard e.keyCode == 53 else { return e }
            NSApp.stopModal(withCode: .alertFirstButtonReturn)
            return nil
        }
        defer { escape.map(NSEvent.removeMonitor) }
        if !NSApp.isActive { NSApp.activate(ignoringOtherApps: true) }
        switch SpriteModel.alertRunner(alert) {
        case .alertSecondButtonReturn: return .once
        case .alertThirdButtonReturn: return .always
        default: return .deny
        }
    }
}
