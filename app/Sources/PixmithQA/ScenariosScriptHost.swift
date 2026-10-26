import AppKit
@testable import PixmithKit
import PixmithCore

// The Lua host contract (docs/LUA_HOST_API.md): the permission alert a script waits on, documents a script creates,
// saves or closes, and sprite.filename. Scripts run as sessions like in the app; files live in the QA temp directory.

private var notices: [(title: String, text: String, warning: Bool)] = []

/// Answers the permission alert from a list (running out denies) and records what it asked.
private final class Asker {
    var answers: [NSApplication.ModalResponse]
    private(set) var titles: [String] = []
    private(set) var buttons: [[String]] = []
    static let deny = NSApplication.ModalResponse.alertFirstButtonReturn
    static let once = NSApplication.ModalResponse.alertSecondButtonReturn
    static let always = NSApplication.ModalResponse.alertThirdButtonReturn

    init(_ answers: [NSApplication.ModalResponse]) { self.answers = answers }

    func install() {
        SpriteModel.alertRunner = { [self] alert in
            titles.append(alert.messageText)
            buttons.append(alert.buttons.map(\.title))
            return answers.isEmpty ? Self.deny : answers.removeFirst()
        }
    }
}

/// Starts a script as a session on the active tab and waits until it has finished and its result is applied.
private func run(_ source: String, name: String) {
    notices = []
    ScriptSessions.shared.presentNotice = { notices.append(($0, $1, $2)) }
    ScriptSessions.shared.start(source: source, name: name)
    let end = Date().addingTimeInterval(10)
    while ScriptSessions.shared.sessions.values.contains(where: { !$0.isPlugin }), Date() < end {
        ScriptSessions.shared.pump()
        pumpRunLoop(0.02)
    }
    expect(Date() < end, "the script finished")
    pumpRunLoop(0.05)
}

/// A Lua long string holding a path.
private func lua(_ url: URL) -> String { "[==[\(url.path)]==]" }

func registerScriptHost() {
    scenario("scripthost/read-asks-first") {
        _ = resetTabs()
        defer { resetTabs() }
        let secret = tmp("secret.txt")
        try? "hello".write(to: secret, atomically: true, encoding: .utf8)
        let source = "local f = io.open(\(lua(secret)), 'r')\nprint(f:read('a'))\nf:close()"

        // Deny (also what a closed alert means): a script error, and nothing was read.
        let deny = Asker([Asker.deny])
        deny.install()
        run(source, name: "/qa/scripts/qa-read.lua")
        expectEq(deny.titles.count, 1, "one question")
        expectEq(deny.titles.first, "“qa-read.lua” wants to read \(secret.path)")
        expectEq(deny.buttons.first, ["Deny", "Allow Once", "Always Allow for This Script"])
        expectEq(notices.first?.title, "Script error in qa-read.lua")
        expect(notices.first?.text.contains("permission denied") == true, "\(notices)")
        expect(notices.first?.text.contains("hello") == false, "nothing was read: \(notices)")
        expectEq(notices.first?.warning, true)

        // Allow Once works for this run only: the next run asks again.
        let once = Asker([Asker.once, Asker.deny])
        once.install()
        run(source, name: "/qa/scripts/qa-read.lua")
        expectEq(once.titles.count, 1)
        expectEq(notices.map(\.text), ["hello"])
        run(source, name: "/qa/scripts/qa-read.lua")
        expectEq(once.titles.count, 2, "Allow Once is not remembered")
        expectEq(notices.first?.title, "Script error in qa-read.lua")

        // Always Allow for This Script is remembered for that script only (a script with the same file name in another
        // folder is another script).
        let always = Asker([Asker.always])
        always.install()
        run(source, name: "/qa/scripts/qa-read.lua")
        expectEq(always.titles.count, 1)
        expectEq(notices.map(\.text), ["hello"])
        run(source, name: "/qa/scripts/qa-read.lua")
        expectEq(always.titles.count, 1, "no second question for the same script")
        expectEq(notices.map(\.text), ["hello"])
        run(source, name: "/qa/other/qa-read.lua")
        expectEq(always.titles.count, 2, "another script asks again")
        expectEq(notices.first?.title, "Script error in qa-read.lua")
    }

    scenario("scripthost/real-alert-allow-once") {
        _ = resetTabs()
        defer { resetTabs() }
        let secret = tmp("real.txt")
        try? "real alert".write(to: secret, atomically: true, encoding: .utf8)
        SpriteModel.alertRunner = { $0.runModal() }
        // The script thread waits while the alert, shown on the main thread, is answered by clicking its button.
        answerModal("Allow Once") {
            run("local f = io.open(\(lua(secret)), 'r') print(f:read('a')) f:close()", name: "qa-real.lua")
        }
        expectEq(notices.map(\.text), ["real alert"])
        // Writing asks for "write", and Deny is the button that Return presses.
        var title = ""
        var defaultButton = ""
        SpriteModel.alertRunner = { alert in
            title = alert.messageText
            defaultButton = alert.buttons.first { $0.keyEquivalent == "\r" }?.title ?? ""
            return Asker.deny
        }
        let target = tmp("never.txt")
        run("io.open(\(lua(target)), 'w'):close()", name: "qa-real.lua")
        expectEq(title, "“qa-real.lua” wants to write \(target.path)")
        expectEq(defaultButton, "Deny")
        expect(!FileManager.default.fileExists(atPath: target.path), "a denied write creates nothing")
    }

    scenario("scripthost/new-sprite-opens-a-tab") {
        let m = resetTabs()
        defer { resetTabs() }
        SpriteModel.alertRunner = { _ in Asker.deny }
        let undo0 = m.historyCounts.undo
        let script = """
        local s = Sprite(16, 16)
        local img = Image(16, 16)
        img:clear(Color(255, 0, 0))
        s:newCel(s.layers[1], 1, img)
        """
        run(script, name: "qa-new.lua")
        expectEq(notices.count, 0, "\(notices)")
        expectEq(m.slots.count, 2, "the new document is a tab")
        expectEq(m.activeSlot, 1, "and the active one")
        expectEq(m.documentName, "Untitled 2")
        expect(m.isDirty, "an unsaved script document is modified")
        expectPixel(0, 0, RED)
        expectEq(m.slots[0].fileURL, nil)
        m.activate(0)
        expectEq(m.historyCounts.undo, undo0, "the document the script ran on is untouched")

        // A script that goes back to its own document leaves that tab active.
        run("Sprite(4, 4)\napp.activeSprite = sprite", name: "qa-new.lua")
        expectEq(m.slots.count, 3)
        expectEq(m.activeSlot, 0, "the original stays active")
        expectEq(m.slots.last.map { Int(pixmith_width($0.doc)) }, 4)

        // A failing script opens nothing.
        run("Sprite(5, 5)\nerror('boom')", name: "qa-new.lua")
        expectEq(m.slots.count, 3)
    }

    scenario("scripthost/save-as-names-the-tab") {
        let m = resetTabs()
        defer { resetTabs() }
        let own = tmp("own.aseprite"), made = tmp("made.aseprite"), png = tmp("export.png")
        try? FileManager.default.removeItem(at: own)
        // The script saves the document it runs on: the tab takes the file's name and is clean.
        Asker([Asker.once]).install()
        run("sprite:setPixel(1, 1, 0xFF0000FF)\nsprite:saveAs(\(lua(own)))", name: "qa-save.lua")
        expectEq(notices.count, 0, "\(notices)")
        expect(FileManager.default.fileExists(atPath: own.path), "the file was written")
        expectEq(m.fileURL, own)
        expectEq(m.documentName, "own.aseprite")
        expect(!m.isDirty, "saved by the script, so clean")
        expectPixel(1, 1, RED)
        expectEq(m.slots.count, 1)

        // sprite.filename is the tab's file now.
        run("print(sprite.filename)", name: "qa-save.lua")
        expectEq(notices.map(\.text), [own.path])

        // A document the script made and saved arrives as a clean tab with that name; an export (png) changes nothing.
        let asker = Asker([Asker.once, Asker.once])
        asker.install()
        run("""
        local spr = Sprite(8, 8)
        spr:saveAs(\(lua(made)))
        spr:saveAs(\(lua(png)))
        """, name: "qa-save2.lua")
        expectEq(notices.count, 0, "\(notices)")
        expect(FileManager.default.fileExists(atPath: made.path) && FileManager.default.fileExists(atPath: png.path))
        expectEq(m.slots.count, 2)
        expectEq(m.activeSlot, 1)
        expectEq(m.documentName, "made.aseprite")
        expectEq(m.fileURL, made)
        expect(!m.isDirty, "equal to its file, so clean")
        expectEq(asker.titles.count, 2, "one question per file")
    }

    scenario("scripthost/close-original-without-asking") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 1, 1, RED)
        let first = m.activeSlotObject!
        let second = addTab()
        m.activate(0)
        let prompts = AlertScript([])
        prompts.install()
        run("sprite:setPixel(3, 3, 0x0000FFFF)\nsprite:close()", name: "qa-close.lua")
        expectEq(notices.count, 0, "\(notices)")
        expectEq(prompts.prompts.count, 0, "no save prompt: \(prompts.prompts)")
        expectEq(m.slots.count, 1, "the tab is gone")
        expect(m.activeSlotObject === second, "the other tab is active")
        expect(!m.slots.contains { $0 === first })
    }
}
