import AppKit
import SwiftUI
@testable import PixmithKit
import PixmithCore

// The Lua Dialog renderer (docs/LUA_DIALOG.md): sessions started from the model, dialogs as models the views render,
// widget events sent back, results applied as one undo step, plugin commands from an extension.

private var notices: [(title: String, text: String, warning: Bool)] = []

/// Pumps the session host until `done` holds (or two seconds pass).
@discardableResult
private func waitFor(_ what: String, seconds: Double = 3, _ done: () -> Bool) -> Bool {
    let end = Date().addingTimeInterval(seconds)
    while Date() < end {
        ScriptSessions.shared.pump()
        if done() { return true }
        pumpRunLoop(0.02)
    }
    expect(false, "timed out waiting for \(what)")
    return false
}

private func startScript(_ source: String, name: String = "test.lua") {
    notices = []
    ScriptSessions.shared.presentNotice = { notices.append(($0, $1, $2)) }
    ScriptSessions.shared.start(source: source, name: name)
}

private func openDialog() -> DialogModel? { ScriptSessions.shared.dialogs.values.first }

private func shot<V: View>(_ view: V, size: NSSize, dark: Bool, name: String) {
    let host = NSHostingView(rootView: view)
    let container = Backdrop()
    let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    container.frame = NSRect(origin: .zero, size: size)
    host.frame = container.bounds
    host.autoresizingMask = [.width, .height]
    container.addSubview(host)
    window.contentView = container
    pumpRunLoop(0.2)
    window.layoutIfNeeded()
    host.layoutSubtreeIfNeeded()
    pumpRunLoop(0.1)
    let rep = container.bitmapImageRepForCachingDisplay(in: container.bounds)!
    container.cacheDisplay(in: container.bounds, to: rep)
    try? FileManager.default.createDirectory(atPath: QA.outDir, withIntermediateDirectories: true)
    try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: QA.outDir).appendingPathComponent("\(name).png"))
}

private let rampScript = """
local dlg = Dialog("Dots")
dlg:label  { id = "info", text = "Pick a color" }
   :newrow()
   :color  { id = "c", label = "Color", color = 0x1D2B53FF }
   :newrow()
   :slider { id = "n", label = "Dots", min = 1, max = 10, value = 4 }
   :newrow()
   :entry  { id = "who", label = "Name", text = "pix" }
   :newrow()
   :check  { id = "x", text = "Extra", selected = true }
   :newrow()
   :combobox { id = "mode", label = "Mode", option = "b", options = { "a", "b", "c" } }
   :separator{ text = "Go" }
   :button { id = "ok", text = "&OK", focus = true }
   :button { id = "cancel", text = "Cancel" }
   :show()
local d = dlg.data
if d.ok then
  for i = 0, d.n - 1 do sprite:setPixel(i, 0, d.c) end
  print("who=" .. d.who .. " x=" .. tostring(d.x) .. " mode=" .. d.mode)
end
"""

func registerLuaDialog() {
    scenario("luadialog/modal-dialog-result-is-one-undo-step") {
        let m = fresh()
        let undo0 = m.historyCounts.undo
        startScript(rampScript)
        waitFor("the dialog") { openDialog() != nil }
        guard let d = openDialog() else { return }
        expectEq(d.title, "Dots")
        expect(d.modal)
        expectEq(d.rows.count, 8, "one row per newrow, a separator has its own")
        expectEq(d.widgets["n"]?.value, 4)
        expectEq(d.widgets["mode"]?.option, "b")
        expectEq(d.widgets["c"]?.color, 0x1D2B53FF)
        expectEq(d.widgets["ok"]?.focus, true)
        for dark in [true, false] {
            shot(LuaDialogView(model: d), size: NSSize(width: 420, height: 360), dark: dark, name: "luadialog-\(dark ? "dark" : "light")")
        }
        // The user edits widgets and presses OK.
        d.edit("n") { $0.value = 6 }
        d.change("n", value: 6)
        d.release("n")
        d.edit("who") { $0.text = "ada" }
        d.change("who", value: "ada")
        d.edit("x") { $0.selected = false }
        d.click("x", value: false)
        d.edit("mode") { $0.option = "c" }
        d.change("mode", value: "c")
        d.edit("c") { $0.color = 0xFF0000FF }
        d.change("c", value: UInt32(0xFF0000FF))
        d.click("ok")
        waitFor("the script to finish") { ScriptSessions.shared.sessions.isEmpty }
        expect(ScriptSessions.shared.dialogs.isEmpty, "the dialog is gone")
        expectEq(m.historyCounts.undo, undo0 + 1, "dialog and edits are one undo step")
        for i in 0..<6 { expectPixel(i, 0, 0xFF0000FF) }
        expectPixel(6, 0, CLEAR)
        expectEq(notices.map(\.text), ["who=ada x=false mode=c"])
        m.undo()
        expectPixel(0, 0, CLEAR)
    }

    scenario("luadialog/cancel-close-and-errors") {
        let m = fresh()
        // Closing the window leaves `data.ok` false and changes nothing.
        startScript(rampScript)
        waitFor("the dialog") { openDialog() != nil }
        openDialog()?.userClosed()
        waitFor("the script to finish") { ScriptSessions.shared.sessions.isEmpty }
        expectPixel(0, 0, CLEAR)
        expectEq(notices.count, 0)
        // A script error rolls everything back and says so.
        startScript("sprite:setPixel(0, 0, 0xFF0000FF)\nerror('boom')", name: "bad.lua")
        waitFor("the error notice") { !notices.isEmpty }
        expectEq(notices.first?.title, "Script error in bad.lua")
        expect(notices.first?.text.contains("boom") == true && notices.first?.text.contains("not changed") == true, "\(notices)")
        expectEq(notices.first?.warning, true)
        expectPixel(0, 0, CLEAR)
        // Cancelling a running script.
        startScript("while true do end", name: "loop.lua")
        waitFor("the session") { !ScriptSessions.shared.sessions.isEmpty }
        pumpRunLoop(0.1)
        for id in ScriptSessions.shared.sessions.keys { ScriptSessions.shared.stop(id) }
        waitFor("the run to stop") { ScriptSessions.shared.sessions.isEmpty }
        expectEq(notices.count, 1, "a cancelled run shows nothing")
        // Alerts with buttons ride on a modal dialog.
        startScript("local n = app.alert{ title = 'Sure?', text = 'Really?', buttons = { '&Yes', '&No' } }\nprint('n=' .. n)")
        waitFor("the alert dialog") { openDialog() != nil }
        expectEq(openDialog()?.title, "Sure?")
        let yes = openDialog()?.widgets.values.first { $0.kind == "button" && dialogText($0.text) == "Yes" }
        expect(yes != nil, "a Yes button")
        if let yes { openDialog()?.click(yes.id) }
        waitFor("the script to finish") { ScriptSessions.shared.sessions.isEmpty }
        expectEq(notices.last?.text, "n=1")
        _ = m
    }

    scenario("luadialog/non-modal-dialog-modify-and-events") {
        let m = fresh()
        startScript("""
        local dlg = Dialog{ title = "Live" }
        dlg:label{ id = "info", text = "waiting" }
           :button{ id = "close", text = "Close", onclick = function() dlg:close() end }
        app.events:on("fgcolorchange", function(ev) dlg:modify{ id = "info", text = string.format("fg %08X", ev.color) } end)
        dlg:show{ wait = false }
        """, name: "live.lua")
        waitFor("the dialog") { openDialog() != nil }
        guard let d = openDialog() else { return }
        expect(!d.modal)
        waitFor("the main chunk") { ScriptSessions.shared.sessions.values.first?.state == "idle" }
        expectEq(d.widgets["info"]?.text, "waiting")
        // The foreground color changes: the script hears it and patches the label.
        m.color = 0x336699FF
        waitFor("the label patch") { d.widgets["info"]?.text == "fg 336699FF" }
        // Its button runs a callback that closes the dialog; the session ends with the last listener... keep it
        // simple: stop it.
        d.click("close")
        waitFor("the dialog to close") { ScriptSessions.shared.dialogs.isEmpty }
        for id in ScriptSessions.shared.sessions.keys { ScriptSessions.shared.stop(id) }
        waitFor("the session to end") { ScriptSessions.shared.sessions.isEmpty }
    }

    scenario("luadialog/extension-plugin-command") {
        let m = fresh()
        let lib = ExtensionLibrary.shared
        lib.reload()
        lib.items.forEach { lib.uninstall($0.name) }
        notices = []
        ScriptSessions.shared.presentNotice = { notices.append(($0, $1, $2)) }
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let out = tmp("plugin.pixmith-extension")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        p.arguments = [root.appendingPathComponent("scripts/extension-template/pack.sh").path,
                       root.appendingPathComponent("scripts/extension-template").path, out.path]
        p.standardOutput = FileHandle.nullDevice
        try? p.run()
        p.waitUntilExit()
        expect(lib.install(out), "installs: \(coreError)")
        // The enabled extension's script loaded as a plugin and registered its command.
        waitFor("the plugin command") { ScriptSessions.shared.commands.contains { $0.commandID == "sample_dots" } }
        guard var cmd = ScriptSessions.shared.commands.first(where: { $0.commandID == "sample_dots" }) else { return }
        expectEq(cmd.title, "Sample: Diagonal Dots...")
        // `onenabled` needs a document: the host's refresh event tells the plugin about the active one.
        waitFor("the command to become enabled") {
            ScriptSessions.shared.commands.first { $0.commandID == "sample_dots" }?.enabled == true
        }
        cmd = ScriptSessions.shared.commands.first { $0.commandID == "sample_dots" } ?? cmd
        let undo0 = m.historyCounts.undo
        ScriptSessions.shared.invoke(cmd)
        waitFor("the command's dialog") { openDialog() != nil }
        guard let d = openDialog() else { return }
        expectEq(d.title, "Diagonal dots")
        d.edit("count") { $0.value = 5 }
        d.change("count", value: 5)
        d.click("ok")
        waitFor("the command to finish") { notices.contains { $0.text.hasPrefix("Drew 5 dots") } }
        for i in 0..<5 { expectPixel(i, i, 0xFF6AC1FF) }
        expectEq(m.historyCounts.undo, undo0 + 1, "one undo step")
        expect(notices.last?.text.contains("run 1") == true, "\(notices)")
        // Disabling the extension unloads the plugin and removes its command.
        lib.setEnabled("sample-extension", false)
        waitFor("the command to go") { !ScriptSessions.shared.commands.contains { $0.commandID == "sample_dots" } }
        lib.setEnabled("sample-extension", true)
        waitFor("the command to return") { ScriptSessions.shared.commands.contains { $0.commandID == "sample_dots" } }
        lib.uninstall("sample-extension")
        waitFor("the command to go again") { !ScriptSessions.shared.commands.contains { $0.commandID == "sample_dots" } }
        ScriptSessions.shared.stopAll()
    }
}
