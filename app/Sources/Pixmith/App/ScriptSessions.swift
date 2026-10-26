import AppKit
import Combine
import PixmithCore
import SwiftUI

// The front-end half of docs/LUA_DIALOG.md. A script run or a plugin is a "session" on a core thread; this polls the
// messages the core queues (dialog.open, state, txn, command.add...), shows dialogs, applies results to the document on
// the main thread, keeps File > Scripts in step with plugin commands and tells the sessions what the user does
// (`app.events`, `sprite.events`).

/// A command a plugin registered with `plugin:newCommand`.
struct ScriptCommand: Identifiable, Equatable {
    let session: UInt64
    let commandID: String
    var title: String
    var group: String
    var enabled: Bool
    var checked: Bool
    var id: String { "\(session)/\(commandID)" }
}

final class ScriptSessions: ObservableObject {
    static let shared = ScriptSessions()

    struct Info {
        var name: String
        var isPlugin: Bool
        var state = "running"
        var runningSince: Date? = Date()
    }

    @Published private(set) var commands: [ScriptCommand] = []
    private(set) var sessions: [UInt64: Info] = [:]
    private(set) var dialogs: [String: DialogModel] = [:]
    private var windows: [String: DialogWindow] = [:]
    private var busyWindows: [UInt64: NSWindow] = [:]
    private var timer: Timer?
    private var cancellables = Set<AnyCancellable>()
    private var observing = false

    /// How a result notice or an error is shown; the QA runner replaces it so no alert blocks a scenario.
    var presentNotice: (_ title: String, _ text: String, _ warning: Bool) -> Void = { title, text, warning in
        let alert = NSAlert()
        alert.messageText = title
        alert.alertStyle = warning ? .warning : .informational
        alert.informativeText = text
        alert.runModal()
    }

    /// Set by `SpriteModel.undo/redo` just before they publish the change.
    var fromUndo = false
    /// True while the host itself changes the model (applying a result, `app.set`), which must not echo back as an event.
    private var quiet = false

    private var model: SpriteModel { .shared }

    // MARK: Starting and stopping

    /// Runs a script as a session on the active document. The UI stays responsive; a modal dialog of the script is a
    /// sheet, and a run that takes long shows a progress sheet with Cancel.
    func start(source: String, name: String) {
        guard !MainWindow.hasSheet else { return }
        model.finishTransients()
        observeModel()
        ScriptPermissions.install()
        syncDocPaths()
        pixmith_script_set_colors(model.color, model.opts.secondary)
        let id = source.withCString { src in
            name.withCString { n in
                pixmith_script_start(model.doc, tag(of: model.activeSlotObject), src, n, UInt32(model.layer), UInt32(model.frame))
            }
        }
        guard id != 0 else {
            model.errorMessage = String(cString: pixmith_last_error())
            return
        }
        sessions[id] = Info(name: ScriptPermissions.displayName(name), isPlugin: false)
        startTimer()
    }

    /// Loads a plugin once: its `init` registers commands. Returns the session id (0 on failure).
    @discardableResult
    func loadPlugin(source: String, name: String, dir: String?, prefsPath: String?) -> UInt64 {
        observeModel()
        ScriptPermissions.install()
        syncDocPaths()
        pixmith_script_set_colors(model.color, model.opts.secondary)
        let id = source.withCString { src in
            name.withCString { n in
                withOptionalCString(dir) { d in withOptionalCString(prefsPath) { p in pixmith_plugin_load(src, n, d, p) } }
            }
        }
        guard id != 0 else {
            presentNotice("Could not load \(name)", String(cString: pixmith_last_error()), true)
            return 0
        }
        sessions[id] = Info(name: name, isPlugin: true)
        startTimer()
        return id
    }

    func stop(_ session: UInt64) { pixmith_script_stop(session) }

    /// Stops every session (quit): plugins run their `exit`.
    func stopAll() {
        sessions.keys.forEach { pixmith_script_stop($0) }
        let deadline = Date().addingTimeInterval(0.6)
        while pixmith_script_count() > 0 && Date() < deadline {
            pump()
            RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.02))
        }
        timer?.invalidate()
        timer = nil
    }

    private func startTimer() {
        guard timer == nil else { return }
        let t = Timer(timeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in self?.tick() }
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    private func tick() {
        if pixmith_script_count() == 0 && sessions.isEmpty { return }
        pump()
        refreshPlugins()
    }

    /// Plugins decide whether their commands are enabled from the document of the last event they got, so an idle
    /// plugin hears a no-op event whenever the site or the document changed (at most four times a second).
    private var refreshDirty = false
    private var lastRefresh = Date.distantPast

    func refreshPlugins() {
        guard refreshDirty, Date().timeIntervalSince(lastRefresh) > 0.25 else { return }
        let idle = sessions.filter { $0.value.isPlugin && $0.value.state == "idle" }.map(\.key)
        guard !idle.isEmpty else { return }
        refreshDirty = false
        lastRefresh = Date()
        for id in idle { send(id, ["type": "app.event", "name": "refresh"]) }
    }

    // MARK: Messages

    /// Handles everything the core has queued. Called by the timer; the QA runner calls it directly.
    func pump() {
        guard let p = pixmith_script_poll(0) else { return }
        defer { pixmith_string_free(p) }
        guard let data = String(cString: p).data(using: .utf8),
              let messages = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return }
        for m in messages { handle(m) }
        updateBusy()
    }

    private func handle(_ m: [String: Any]) {
        let session = (m["session"] as? NSNumber)?.uint64Value ?? 0
        switch m["type"] as? String {
        case "dialog.open":
            if let d = m["dialog"] as? [String: Any], let dialog = DialogModel(session: session, json: d) { open(dialog) }
        case "dialog.modify":
            if let id = m["dialog"] as? String { dialogs[id]?.modify(m) }
        case "dialog.close":
            if let id = m["dialog"] as? String { closeFromCore(id) }
        case "state":
            if let state = m["state"] as? String { setState(session, state) }
        case "txn":
            finishTransaction(session, m)
        case "app.set":
            quiet = true
            if let fg = m["fgColor"] as? NSNumber { model.color = fg.uint32Value }
            if let bg = m["bgColor"] as? NSNumber { model.opts.secondary = bg.uint32Value }
            quiet = false
            pixmith_script_set_colors(model.color, model.opts.secondary)
        case "command.add":
            if let c = m["command"] as? [String: Any], let cid = c["id"] as? String {
                commands.removeAll { $0.session == session && $0.commandID == cid }
                commands.append(ScriptCommand(
                    session: session, commandID: cid, title: c["title"] as? String ?? cid, group: c["group"] as? String ?? "file_scripts",
                    enabled: c["enabled"] as? Bool ?? true, checked: c["checked"] as? Bool ?? false))
            }
        case "command.update":
            if let c = m["command"] as? [String: Any], let cid = c["id"] as? String,
               let i = commands.firstIndex(where: { $0.session == session && $0.commandID == cid }) {
                if let v = c["title"] as? String { commands[i].title = v }
                if let v = c["enabled"] as? Bool { commands[i].enabled = v }
                if let v = c["checked"] as? Bool { commands[i].checked = v }
            }
        case "command.remove":
            if let cid = m["id"] as? String { commands.removeAll { $0.session == session && $0.commandID == cid } }
        case "session.end":
            end(session)
        default:
            break
        }
    }

    private func setState(_ session: UInt64, _ state: String) {
        guard var info = sessions[session] else { return }
        if state == "running" {
            if info.state != "running" { info.runningSince = Date() }
        } else {
            info.runningSince = nil
        }
        info.state = state
        sessions[session] = info
        if state == "idle", info.isPlugin { refreshDirty = true }
        if state != "running" { dismissBusy(session) }
    }

    private func end(_ session: UInt64) {
        for (id, d) in dialogs where d.session == session { closeFromCore(id) }
        commands.removeAll { $0.session == session }
        dismissBusy(session)
        sessions[session] = nil
    }

    // MARK: Transactions

    private func finishTransaction(_ session: UInt64, _ m: [String: Any]) {
        let name = sessions[session]?.name ?? "script"
        let tagValue = (m["doc"] as? NSNumber)?.uint64Value ?? 0
        let ok = m["ok"] as? Bool ?? true
        let error = m["error"] as? String
        let txn = (m["txn"] as? NSNumber)?.uint64Value ?? 0
        let messages = (m["messages"] as? [String] ?? []).joined(separator: "\n")
        var failure = ok ? nil : (error ?? "unknown error")
        var applied = false
        if ok, m["commit"] as? Bool == true {
            let slot = slot(forTag: tagValue)
            let doc: OpaquePointer? = slot.map { $0 === model.activeSlotObject ? model.doc : $0.doc }
            let r = pixmith_script_apply(session, txn, doc)
            switch r {
            case 1: applied = true
            case -1: failure = "The document changed while the script was running; its result was discarded."
            default: break
            }
            if applied, let slot, slot === model.activeSlotObject {
                if let sel = m["select"] as? [String: Any] {
                    if let l = (sel["layer"] as? NSNumber)?.intValue { model.layer = max(0, min(l - 1, model.layerCount - 1)) }
                    if let f = (sel["frame"] as? NSNumber)?.intValue { model.frame = max(0, min(f - 1, model.frameCount - 1)) }
                }
                model.clampSelection()
                model.snapDrawColor()
                quiet = true
                model.revision &+= 1
                quiet = false
            }
        }
        if ok, let docs = m["docs"] as? [String: Any] {
            takeDocuments(session: session, txn: txn, docs: docs, original: slot(forTag: tagValue))
        }
        if failure == "script cancelled" { return }
        guard !messages.isEmpty || failure != nil else { return }
        let conflict = failure?.hasPrefix("The document changed") ?? false
        let title = failure == nil || conflict ? name : "Script error in \(name)"
        let text = [messages, failure.map { conflict ? $0 : "\($0)\nThe document was not changed." }]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "\n\n")
        presentNotice(title, text, failure != nil)
    }

    // MARK: Documents

    private var tags: [ObjectIdentifier: UInt64] = [:]
    private var registeredPaths: [UInt64: String] = [:]

    /// Tells the core which file each tab has (`sprite.filename`, `app.open` of the script's own document).
    func syncDocPaths() {
        for slot in model.slots {
            let url = slot === model.activeSlotObject ? model.fileURL : slot.fileURL
            let path = url?.path
            let t = tag(of: slot)
            if registeredPaths[t] == path { continue }
            registeredPaths[t] = path
            withOptionalCString(path) { pixmith_script_set_doc_path(t, $0) }
        }
    }

    /// The `docs` object of a `txn` message (docs/LUA_HOST_API.md, section 2): the documents the script created or
    /// opened become tabs (clean when the core says they equal their file), the one it left active is activated, and
    /// what it did to its own document (saved it under a path, closed it) is followed.
    private func takeDocuments(session: UInt64, txn: UInt64, docs: [String: Any], original: DocumentSlot?) {
        var created: [Int: DocumentSlot] = [:]
        for entry in docs["new"] as? [[String: Any]] ?? [] {
            guard let index = (entry["index"] as? NSNumber)?.intValue,
                  let d = pixmith_script_take_doc(session, txn, UInt32(index)) else { continue }
            let url = (entry["path"] as? String).map { URL(fileURLWithPath: $0) }
            if let url, let open = model.slot(for: url) {
                pixmith_free(d)  // that file is already a tab: show it instead of a second copy
                created[index] = open
                continue
            }
            if entry["modified"] as? Bool ?? true { pixmith_touch(d) } else { pixmith_mark_saved(d) }
            AppRoute.shared.enterEditor()
            created[index] = model.addBackgroundTab(doc: d, url: url)
            if let url { RecentFiles.shared.add(url) }
        }
        // What the script did to the document it ran on.
        let closeOriginal = docs["closeOriginal"] as? Bool ?? false
        if !closeOriginal, let original, let path = docs["originalPath"] as? String {
            adoptSavedPath(URL(fileURLWithPath: path), saved: docs["originalSaved"] as? Bool ?? false, slot: original)
        }
        // The tab to show.
        quiet = true
        defer { quiet = false }
        switch docs["active"] {
        case let n as NSNumber:
            if let slot = created[n.intValue], let i = model.slots.firstIndex(where: { $0 === slot }) { model.activate(i) }
        case let s as String where s == "original":
            if let original, let i = model.slots.firstIndex(where: { $0 === original }) { model.activate(i) }
        default:
            break
        }
        if closeOriginal, let original, let i = model.slots.firstIndex(where: { $0 === original }) {
            model.closeTab(i, discardChanges: true)
        }
        syncDocPaths()
    }

    /// The script saved its document with `saveAs` to a document file: the tab becomes that file, and is clean when
    /// nothing changed after the save.
    private func adoptSavedPath(_ url: URL, saved: Bool, slot: DocumentSlot) {
        let active = slot === model.activeSlotObject
        if active { model.fileURL = url } else { slot.fileURL = url }
        slot.customName = nil
        slot.importSource = nil
        RecentFiles.shared.add(url)
        guard saved else { return }
        if active {
            pixmith_mark_saved(model.doc)
            model.savedRevision = model.revision
        } else {
            pixmith_mark_saved(slot.doc)
            slot.savedRevision = slot.revision
        }
        Autosaver.shared.discard(slot)
    }

    /// A small number that stands for a document in events and results (JSON numbers keep 53 bits, so no pointers).
    func tag(of slot: DocumentSlot?) -> UInt64 {
        guard let slot else { return 0 }
        let key = ObjectIdentifier(slot)
        if let t = tags[key] { return t }
        let t = UInt64(tags.count + 1)
        tags[key] = t
        return t
    }

    private func slot(forTag t: UInt64) -> DocumentSlot? {
        t == 0 ? model.activeSlotObject : model.slots.first { tag(of: $0) == t }
    }

    // MARK: Events to the core

    /// Sends one event to a session on behalf of the active document.
    @discardableResult
    func send(_ session: UInt64, _ event: [String: Any]) -> Bool {
        guard let data = try? JSONSerialization.data(withJSONObject: event), let json = String(data: data, encoding: .utf8) else { return false }
        syncDocPaths()
        return json.withCString {
            pixmith_script_send(session, model.doc, tag(of: model.activeSlotObject), UInt32(model.layer), UInt32(model.frame), $0)
        }
    }

    @discardableResult
    func broadcast(_ event: [String: Any]) -> UInt32 {
        guard pixmith_script_count() > 0,
              let data = try? JSONSerialization.data(withJSONObject: event), let json = String(data: data, encoding: .utf8) else { return 0 }
        syncDocPaths()
        return json.withCString {
            pixmith_script_broadcast(model.doc, tag(of: model.activeSlotObject), UInt32(model.layer), UInt32(model.frame), $0)
        }
    }

    /// Runs a menu command between `beforecommand` and `aftercommand` events (`name` is the menu title).
    func command(_ name: String, _ body: () -> Void) {
        guard pixmith_script_count() > 0 else { return body() }
        broadcast(["type": "app.event", "name": "beforecommand", "command": name])
        body()
        broadcast(["type": "app.event", "name": "aftercommand", "command": name])
    }

    /// A menu click on a plugin command.
    func invoke(_ c: ScriptCommand) {
        guard !MainWindow.hasSheet else { return }
        model.finishTransients()
        send(c.session, ["type": "command", "id": c.commandID])
    }

    private func observeModel() {
        guard !observing else { return }
        observing = true
        let m = model
        Publishers.CombineLatest3(m.$activeSlot, m.$layer, m.$frame)
            .removeDuplicates { $0 == $1 }
            .dropFirst()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                self?.refreshDirty = true
                guard let self, !self.quiet else { return }
                self.broadcast(["type": "app.event", "name": "sitechange", "layer": m.layer + 1, "frame": m.frame + 1])
            }
            .store(in: &cancellables)
        m.$color.removeDuplicates().dropFirst().receive(on: DispatchQueue.main).sink { [weak self] c in
            pixmith_script_set_colors(c, m.opts.secondary)
            guard let self, !self.quiet else { return }
            self.broadcast(["type": "app.event", "name": "fgcolorchange", "color": c])
        }.store(in: &cancellables)
        m.opts.$secondary.removeDuplicates().dropFirst().receive(on: DispatchQueue.main).sink { [weak self] c in
            pixmith_script_set_colors(m.color, c)
            guard let self, !self.quiet else { return }
            self.broadcast(["type": "app.event", "name": "bgcolorchange", "color": c])
        }.store(in: &cancellables)
        m.$revision.dropFirst().sink { [weak self] _ in
            self?.refreshDirty = true
            guard let self, !self.quiet, pixmith_script_count() > 0 else { return }
            let undo = self.fromUndo
            self.fromUndo = false
            DispatchQueue.main.async {
                self.broadcast(["type": "sprite.event", "name": "change", "fromUndo": undo])
            }
        }.store(in: &cancellables)
        m.$fileURL.dropFirst().removeDuplicates().receive(on: DispatchQueue.main).sink { [weak self] _ in
            self?.broadcast(["type": "sprite.event", "name": "filenamechange"])
        }.store(in: &cancellables)
    }

    // MARK: Dialogs

    private func open(_ dialog: DialogModel) {
        dialog.send = { [weak self, weak dialog] widget, event, extra in
            guard let self, let dialog else { return }
            var msg: [String: Any] = ["type": "widget", "dialog": dialog.id, "id": widget, "event": event]
            extra.forEach { msg[$0.key] = $0.value }
            self.send(dialog.session, msg)
        }
        dialog.userClosed = { [weak self, weak dialog] in
            guard let self, let dialog else { return }
            self.send(dialog.session, ["type": "dialog.closed", "dialog": dialog.id])
            self.closeLocally(dialog.id)
        }
        dialogs[dialog.id] = dialog
        let present = { [weak self] in
            guard let self, self.dialogs[dialog.id] != nil else { return }
            let w = DialogWindow(model: dialog, sheet: dialog.modal)
            self.windows[dialog.id] = w
            w.show()
        }
        // A progress sheet of this session gives way to the dialog first.
        if busyWindows[dialog.session] != nil {
            dismissBusy(dialog.session)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15, execute: present)
        } else {
            present()
        }
    }

    private func closeFromCore(_ id: String) { closeLocally(id) }

    private func closeLocally(_ id: String) {
        dialogs[id] = nil
        windows[id]?.dismiss()
        windows[id] = nil
    }

    // MARK: Progress

    /// A run that goes on for more than a moment without a dialog of its own blocks the window with a sheet.
    private func updateBusy() {
        // A script waiting for the user's answer to a file permission alert is not busy.
        guard ScriptPermissions.asking == 0 else { return }
        for (id, info) in sessions where info.state == "running" {
            guard let since = info.runningSince, Date().timeIntervalSince(since) > 0.3, busyWindows[id] == nil,
                  !dialogs.values.contains(where: { $0.session == id && $0.modal }),
                  let host = MainWindow.window, !MainWindow.assumeKey else { continue }
            let w = NSWindow(contentViewController: NSHostingController(rootView: ProgressSheet(
                title: "Running \(info.name)…", onCancel: { [weak self] in self?.stop(id) })))
            w.styleMask = [.titled]
            busyWindows[id] = w
            (host.attachedSheet ?? host).beginSheet(w)
        }
    }

    private func dismissBusy(_ session: UInt64) {
        guard let w = busyWindows.removeValue(forKey: session) else { return }
        w.sheetParent?.endSheet(w)
    }
}

private func withOptionalCString<R>(_ s: String?, _ body: (UnsafePointer<CChar>?) -> R) -> R {
    guard let s else { return body(nil) }
    return s.withCString { body($0) }
}

// MARK: - Windows

/// The sheet or panel that shows one dialog.
private final class DialogWindow: NSObject, NSWindowDelegate {
    let model: DialogModel
    let isSheet: Bool
    let window: NSWindow
    private var closing = false
    private var titleWatch: AnyCancellable?

    init(model: DialogModel, sheet: Bool) {
        self.model = model
        isSheet = sheet
        let host = NSHostingController(rootView: LuaDialogView(model: model, showsTitle: sheet))
        if sheet {
            window = NSWindow(contentViewController: host)
            window.styleMask = [.titled]
        } else {
            let panel = NSPanel(contentViewController: host)
            panel.styleMask = [.titled, .closable, .utilityWindow]
            panel.isFloatingPanel = true
            panel.hidesOnDeactivate = false
            panel.title = model.title
            window = panel
        }
        super.init()
        window.isReleasedWhenClosed = false
        window.delegate = self
        if !sheet { titleWatch = model.$title.sink { [weak panel = window] in panel?.title = $0 } }
    }

    func show() {
        if isSheet, let host = MainWindow.window, !MainWindow.assumeKey {
            var top = host
            while let next = top.attachedSheet { top = next }
            top.beginSheet(window)
        } else {
            window.center()
            window.orderFront(nil)
        }
    }

    func dismiss() {
        closing = true
        if let parent = window.sheetParent { parent.endSheet(window) } else { window.close() }
    }

    func windowWillClose(_ notification: Notification) {
        guard !closing, !isSheet else { return }
        closing = true
        model.userClosed()
    }
}
