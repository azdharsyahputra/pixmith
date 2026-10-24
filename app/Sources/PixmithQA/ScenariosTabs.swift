import AppKit
@testable import PixmithKit
import PixmithCore

// pixmith-qa scenarios for multi-document tabs (SpriteModel.slots / activeSlot / newTab / activate / closeTab,
// SpriteModel.alertRunner, Autosaver.shared, WindowCloseGuard.approved). Each scenario resets to a single pristine tab
// first (`resetTabs`), so it also works with --all-in-process. Where the app's behavior is a judgment call (open curve
// committed vs dropped on a switch), the scenario only asserts what must hold either way and says so.

// MARK: - Helpers

/// Answers the unsaved-changes alert from a list, recording what was asked. Running out of answers cancels,
/// and the extra prompt shows up in `prompts` so the scenario can fail on it.
final class AlertScript {
    var answers: [NSApplication.ModalResponse]
    private(set) var prompts: [String] = []
    /// `SpriteModel.shared.activeSlot` at the moment each alert appeared (the tab the question is about).
    private(set) var activeAtPrompt: [Int] = []

    static let save = NSApplication.ModalResponse.alertFirstButtonReturn
    static let dontSave = NSApplication.ModalResponse.alertSecondButtonReturn
    static let cancel = NSApplication.ModalResponse.alertThirdButtonReturn

    init(_ answers: [NSApplication.ModalResponse]) { self.answers = answers }

    func install() {
        SpriteModel.alertRunner = { [self] alert in
            prompts.append(alert.messageText)
            activeAtPrompt.append(SpriteModel.shared.activeSlot)
            return answers.isEmpty ? Self.cancel : answers.removeFirst()
        }
    }
}

/// Back to exactly one pristine untitled tab with default tool state.
@discardableResult
func resetTabs() -> SpriteModel {
    let m = SpriteModel.shared
    SpriteModel.alertRunner = { _ in AlertScript.dontSave }
    m.finishTransients()
    while m.slots.count > 1 { m.closeTab(m.slots.count - 1) }
    m.closeTab(0)
    WindowCloseGuard.approved = false
    return fresh()
}

/// Opens a new blank tab and returns its slot. The current tab must not be pristine, or it would be replaced.
@discardableResult
func addTab(_ w: Int = 16, _ h: Int = 16) -> DocumentSlot {
    let m = SpriteModel.shared
    let before = m.slots.count
    m.newSprite(width: w, height: h)
    expectEq(m.slots.count, before + 1, "newSprite opened a new tab")
    return m.activeSlotObject!
}

/// Draws one marker pixel into the active tab, which also makes it dirty and not pristine.
func mark(_ d: CanvasDriver, _ x: Int, _ y: Int, _ color: UInt32) {
    put(d, x, y, color)
}

func slotPixel(_ s: DocumentSlot, _ x: Int, _ y: Int, frame: Int = 0) -> UInt32 {
    var buf = [UInt8](repeating: 0, count: Int(pixmith_width(s.doc) * pixmith_height(s.doc)) * 4)
    guard pixmith_render(s.doc, UInt32(frame), &buf, buf.count) else { return 0xDEADBEEF }
    let i = (y * Int(pixmith_width(s.doc)) + x) * 4
    return UInt32(buf[i]) << 24 | UInt32(buf[i + 1]) << 16 | UInt32(buf[i + 2]) << 8 | UInt32(buf[i + 3])
}

func slotHistory(_ s: DocumentSlot) -> (undo: Int, redo: Int) {
    var u: UInt32 = 0, r: UInt32 = 0
    pixmith_history_counts(s.doc, &u, &r)
    return (Int(u), Int(r))
}

func slotVisibleCount(_ s: DocumentSlot) -> Int {
    let n = Int(pixmith_width(s.doc) * pixmith_height(s.doc))
    return (0..<n).filter { slotPixel(s, $0 % Int(pixmith_width(s.doc)), $0 / Int(pixmith_width(s.doc))) & 0xFF != 0 }.count
}

/// Where the app writes recovery copies; `AppSupport.directory` honors PIXMITH_SUPPORT_DIR, which qa points at a temp HOME.
let recoveryDirectory = AppSupport.directory.appendingPathComponent("Pixmith/Recovery", isDirectory: true)

/// Pumps the run loop until `cond` holds (up to ~3 s). Autosave writes on a background queue and reports back on main,
/// and background jobs (scripts) finish on main, so scenarios have to let the loop run.
@discardableResult
func waitUntil(_ cond: () -> Bool) -> Bool {
    for _ in 0..<100 where !cond() { pumpRunLoop(0.03) }
    return cond()
}

/// Runs a Lua script and waits for its background job to finish and be applied.
func runScriptAndWait(_ m: SpriteModel, _ url: URL) {
    m.runScript(at: url)
    waitUntil { !Background.busy }
    pumpRunLoop(0.05)
}

func recoveryFile(_ s: DocumentSlot) -> URL {
    recoveryDirectory.appendingPathComponent(s.recoveryID.uuidString).appendingPathExtension("aseprite")
}

// MARK: - Scenarios

func registerTabs() {
    scenario("tabs/two-tabs-draw-undo-independent") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 1, 1, RED)
        let a = m.activeSlotObject!
        let b = addTab()
        mark(d, 2, 2, BLUE)
        mark(d, 3, 3, BLUE)
        expectEq(m.slots.count, 2)
        expect(m.activeSlot == 1 && m.isDirty(b) && m.isDirty(a))
        expectEq(slotHistory(b).undo, 2)
        expectEq(slotHistory(a).undo, 1)

        // The background tab keeps its own pixels, history and dirty flag.
        expectEq(slotPixel(a, 1, 1), RED)
        expectEq(slotPixel(a, 2, 2), CLEAR)
        m.undo()
        expectPixel(3, 3, CLEAR)
        expectPixel(2, 2, BLUE)
        expectEq(slotHistory(a).undo, 1, "undo in one tab leaves the other's history alone")
        expectEq(slotPixel(a, 1, 1), RED)

        m.activate(0)
        expectPixel(1, 1, RED)
        expectPixel(2, 2, CLEAR, "tab 2's pixels are not shown in tab 1")
        m.undo()
        expectPixel(1, 1, CLEAR)
        expect(!m.isDirty, "undoing back to the opened state is clean")
        expect(m.isDirty(b), "the other tab stays dirty")
        m.redo()
        expectPixel(1, 1, RED)

        // Redo stacks survive a switch: b still has one undone step to redo.
        m.activate(1)
        expectEq(slotHistory(b).redo, 1)
        m.redo()
        expectPixel(3, 3, BLUE)
        // Drawing in b after a switch is still undoable in b only.
        mark(d, 5, 5, GREEN)
        m.undo()
        expectPixel(5, 5, CLEAR)
        expectPixel(3, 3, BLUE)
        expectEq(slotPixel(a, 5, 5), CLEAR)
        d.snapshot("tabs-two-tabs")
    }

    scenario("tabs/per-tab-view-state") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 0, 0, RED)
        m.zoom = 4
        m.addLayer()
        m.frame = 0
        let a = m.activeSlotObject!
        let layersA = m.layerCount
        addTab(32, 24)
        expectEq(m.width, 32)
        m.zoom = 12
        m.activate(0)
        expectEq(m.width, 16, "tab 1 keeps its size")
        expectEq(m.zoom, 4, "and its zoom")
        expectEq(m.layerCount, layersA)
        expect(m.activeSlotObject === a)
        m.activate(1)
        expectEq(m.zoom, 12)
        expectEq(m.height, 24)
    }

    scenario("tabs/switch-commits-floating-paste") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        paintRect(d, 4, 4, 5, 5, RED)
        m.tool = .marquee
        d.stroke([(4, 4), (5, 5)])
        m.copySelection(cut: false)
        m.deselect()
        m.pasteFromPasteboard()
        expect(pixmith_float_active(m.doc), "paste floats")
        d.stroke([(4, 4), (9, 4)]) // drag the floating pixels to x = 9
        let a = m.activeSlotObject!
        let before = slotHistory(a).undo

        // Switching away commits the float into tab 1 and starts tab 2 clean.
        addTab()
        expect(!pixmith_float_active(a.doc), "the float was committed when leaving the tab")
        expect(!pixmith_float_active(m.doc))
        expectEq(slotPixel(a, 9, 4), RED, "committed at the dragged position")
        expectEq(m.visibleCount(), 0, "nothing leaked into the new tab")
        expectEq(slotHistory(m.activeSlotObject!).undo, 0)
        expect(!m.hasSelection || m.selectedCount == 0, "selection state is per tab")

        // Back in tab 1: the paste is a normal undo step and Enter/Escape have nothing floating to act on.
        m.activate(0)
        expectPixel(9, 4, RED)
        expectEq(m.historyCounts.undo, before, "committing added no extra undo step")
        d.escape()
        expectPixel(9, 4, RED, "Escape must not cancel a paste that was already committed")
        // Escape does deselect the (still selected) pasted area, and that is its own undo step; the paste is the one below.
        expectEq(m.historyCounts.undo, before + 1, "Escape added only the deselect step")
        m.undo()
        expectPixel(9, 4, RED, "undoing the deselect keeps the paste")
        m.undo()
        expectPixel(9, 4, CLEAR)
        expectPixel(4, 4, RED)

        // Same via the tab keys: paste again, then next/previous tab.
        m.deselect()
        m.pasteFromPasteboard()
        expect(pixmith_float_active(m.doc))
        m.nextTab()
        expect(!pixmith_float_active(a.doc))
        m.previousTab()
        expectPixel(4, 4, RED)
    }

    scenario("tabs/switch-commits-free-transform") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        paintRect(d, 4, 4, 7, 7, RED)
        m.startTransform()
        expect(m.transformActive, "transform session started")
        let a = m.activeSlotObject!
        addTab()
        expect(!pixmith_xf_active(a.doc), "the transform was committed when leaving")
        expect(!m.transformActive, "the new tab has no transform")
        expectEq(slotVisibleCount(a), 16, "the transformed pixels stayed in tab 1")
        expectEq(m.visibleCount(), 0)
        m.activate(0)
        expect(!m.transformActive)
        expectEq(m.visibleCount(), 16)
        m.undo()
        expectEq(m.visibleCount(), 16, "undo goes back to before the transform, i.e. the rectangle")
        m.undo()
        expectEq(m.visibleCount(), 0)
    }

    scenario("tabs/switch-with-open-curve") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        m.tool = .curve
        m.color = RED
        d.down(1, 8)
        d.drag(14, 8)
        d.up(14, 8)
        expect(m.drawingSessionOpen, "curve waits for control points")
        let a = m.activeSlotObject!
        // The tabs branch finishes open shapes when leaving a tab. Whether that commits or drops the curve,
        // the session must be closed everywhere and the other tab untouched.
        addTab()
        expect(!m.drawingSessionOpen, "no curve session in the new tab")
        expectEq(m.visibleCount(), 0)
        m.tool = .pencil
        mark(d, 3, 3, BLUE)
        m.activate(0)
        expect(!m.drawingSessionOpen, "the old session did not come back")
        expectEq(slotPixel(m.slots[1], 3, 3), BLUE)
        expectEq(slotPixel(a, 3, 3), CLEAR, "drawing in tab 2 did not continue the curve in tab 1")
        expect(slotHistory(a).undo <= 1, "the curve is at most one undo step")
        // Clicking on tab 1's canvas now starts a fresh pencil stroke rather than a curve control point.
        d.click(6, 6)
        expectPixel(6, 6, m.color)
    }

    scenario("tabs/switch-with-open-polygon-and-text") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        m.tool = .shapePolygon  // the drawing polygon; `.polygon` is the polygonal selection lasso
        m.color = GREEN
        d.click(2, 2)
        d.click(12, 2)
        d.click(12, 12)
        expect(m.drawingSessionOpen, "polygon waits for more points")
        addTab()
        expect(!m.drawingSessionOpen)
        m.tool = .pencil
        mark(d, 1, 1, RED)
        m.activate(0)
        expect(!m.drawingSessionOpen)
        // The polygon is committed or dropped, but never half-present in tab 2.
        expectEq(slotPixel(m.slots[1], 12, 2), CLEAR)

        // Same for text still being placed: leaving the tab places it there, and nothing reaches the other tab.
        m.tool = .text
        m.opts.textScale = 1
        m.color = BLUE
        d.click(2, 8)
        expect(m.opts.textRequest != nil, "text tool asks for text")
        m.opts.textString = "A"
        m.renderText()
        let first = m.slots[0]
        let withText = slotVisibleCount(first)
        let other = slotVisibleCount(m.slots[1])
        m.activate(1)
        expect(m.opts.textRequest == nil, "no text request in the other tab")
        expectEq(slotVisibleCount(first), withText, "the text stayed in tab 1")
        expectEq(slotVisibleCount(m.slots[1]), other, "no text pixels in tab 2")
        m.activate(0)
        expect(m.opts.textRequest == nil, "the old text request did not come back")
        expectEq(m.visibleCount(), withText)
    }

    scenario("tabs/switch-with-open-filter-sheet") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 3, 3, RED)
        let filters = FilterState.shared
        filters.active = .invert
        filters.setPreview(d.bitmap().cgImage) // a preview image is showing on the canvas
        expect(filters.preview != nil)
        addTab()
        expect(filters.active == nil, "the filter sheet belongs to the tab it was opened in")
        expect(filters.preview == nil, "its preview image is dropped")
        expectEq(m.visibleCount(), 0)
        // The preview must not paint tab 1's pixels over tab 2.
        let rep = d.bitmap()
        expect(d.shown(3, 3, in: rep) != 0xFF0000, "tab 1's red pixel is not drawn in tab 2")
        m.activate(0)
        expectPixel(3, 3, RED, "tab 1 itself was not filtered")
        expectEq(m.historyCounts.undo, 1)
    }

    scenario("tabs/switch-with-timeline-focus") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 3, 3, RED)
        m.addFrame(duplicate: true)
        let a = m.activeSlotObject!
        let t = TimelineSelection.shared
        t.range = TimelineSelection.Rect(l0: 0, l1: 0, f0: 0, f1: 1)
        t.focused = true
        addTab()
        expect(t.range == nil && !t.focused, "timeline selection does not follow the switch")
        mark(d, 4, 4, BLUE)
        m.selectAll()
        // Delete acts on pixels (selection), not on cels of the other tab's timeline range.
        d.delete()
        expectPixel(4, 4, CLEAR)
        expectEq(Int(pixmith_frame_count(a.doc)), 2, "tab 1 still has both frames")
        expectEq(slotPixel(a, 3, 3), RED)
        expectEq(slotPixel(a, 3, 3, frame: 1), RED)
        m.activate(0)
        expectEq(m.frameCount, 2)
        expect(t.range == nil, "coming back does not restore a stale timeline range")
    }

    scenario("tabs/stroke-interrupted-by-switch") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 0, 0, RED)
        let a = m.activeSlotObject!
        let b = addTab()
        m.activate(0)
        d.down(2, 8)
        d.drag(5, 8)
        m.activate(1) // e.g. a tab shortcut while the button is down
        d.drag(8, 8)
        d.up(9, 8)
        expectEq(slotPixel(b, 8, 8), CLEAR, "the rest of the drag must not draw into the other tab")
        expectEq(slotPixel(b, 5, 8), CLEAR)
        expect(slotPixel(a, 2, 8) == m.color || slotPixel(a, 2, 8) == BLACK, "the part drawn before the switch is kept")
        expectEq(m.visibleCount(), 0)
        expectEq(slotHistory(b).undo, 0)
        m.activate(0)
        m.undo()
        expectPixel(2, 8, CLEAR)
        expectPixel(0, 0, RED, "undo removed only the interrupted stroke")
    }

    scenario("tabs/close-dirty-tab-prompts") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 1, 1, RED)
        addTab()
        mark(d, 2, 2, BLUE)
        addTab()
        mark(d, 3, 3, GREEN)
        expectEq(m.slots.count, 3)
        let middle = m.slots[1]

        // Cancel: nothing changes, and the tab being asked about was brought to the front.
        let cancel = AlertScript([AlertScript.cancel])
        cancel.install()
        expect(!m.closeTab(1), "cancelled close returns false")
        expectEq(m.slots.count, 3)
        expectEq(cancel.prompts.count, 1)
        expectEq(cancel.activeAtPrompt, [1], "the question is about the visible tab")
        expectEq(slotPixel(middle, 2, 2), BLUE)

        // Don't Save: the tab goes, the neighbor of the one we came from stays active.
        let discard = AlertScript([AlertScript.dontSave])
        discard.install()
        expect(m.closeTab(1))
        expectEq(m.slots.count, 2)
        expect(!m.slots.contains { $0 === middle })
        expectPixel(3, 3, GREEN, "back on the tab that was active before the question")

        // Closing a clean tab never asks.
        let none = AlertScript([])
        none.install()
        m.activate(0)
        m.undo() // tab 1 becomes clean again
        expect(!m.isDirty)
        expect(m.closeTab(0))
        expectEq(none.prompts.count, 0)
        expectEq(m.slots.count, 1)
        expectPixel(3, 3, GREEN)
    }

    scenario("tabs/close-dirty-tab-save") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        let url = tmp("close-save.aseprite")
        mark(d, 1, 1, RED)
        m.fileURL = url
        addTab()
        mark(d, 2, 2, BLUE)
        let script = AlertScript([AlertScript.save])
        script.install()
        expect(m.closeTab(0), "Save then close")
        expectEq(script.prompts.count, 1)
        expect(script.prompts[0].contains("close-save"), "the prompt names the document: \(script.prompts[0])")
        expectEq(m.slots.count, 1)
        expect(FileManager.default.fileExists(atPath: url.path), "the file was written")
        // Reopen it: the saved pixel is there and the tab is clean.
        m.open(url)
        expectEq(m.slots.count, 2)
        expectPixel(1, 1, RED)
        expect(!m.isDirty)
    }

    scenario("tabs/close-last-tab") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        // A pristine single tab: closing does nothing and never asks.
        let none = AlertScript([])
        none.install()
        expect(m.closeActiveTab())
        expectEq(none.prompts.count, 0)
        expectEq(m.slots.count, 1)

        // A dirty single tab: Cancel keeps it, Don't Save replaces it with a fresh untitled document.
        mark(d, 1, 1, RED)
        // It has a recovery copy by now (written asynchronously).
        Autosaver.shared.start(m)
        Autosaver.shared.tick()
        let oldCopy = recoveryFile(m.activeSlotObject!)
        waitUntil { FileManager.default.fileExists(atPath: oldCopy.path) }
        expect(FileManager.default.fileExists(atPath: oldCopy.path), "the dirty tab has a recovery copy before closing")
        let script = AlertScript([AlertScript.cancel, AlertScript.dontSave])
        script.install()
        expect(!m.closeActiveTab())
        expectPixel(1, 1, RED)
        expect(m.closeActiveTab())
        expectEq(script.prompts.count, 2)
        expectEq(m.slots.count, 1, "the window keeps one tab")
        expect(m.activeIsPristine, "the replacement is a fresh untitled document")
        expectEq(m.visibleCount(), 0)
        expectEq(m.historyCounts.undo, 0)
        expect(!m.isDirty)
        expectEq(m.documentName, "Untitled")
        // And the old tab's recovery copy is gone.
        expect(!FileManager.default.fileExists(atPath: oldCopy.path), "the old document's recovery copy is gone")
        let leftovers = ((try? FileManager.default.contentsOfDirectory(atPath: recoveryDirectory.path)) ?? [])
            .filter { $0.hasSuffix(".aseprite") && $0.contains(m.activeSlotObject!.recoveryID.uuidString) }
        expectEq(leftovers.count, 0, "and the replacement has none")
    }

    scenario("tabs/rename-document") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        // An unsaved document keeps the name for its tab (and the Save panel).
        let first = m.activeSlotObject!
        m.renameDocument(first, to: "  hero  ")
        expectEq(m.name(of: first), "hero")
        m.renameDocument(first, to: "   ")
        expectEq(m.name(of: first), "hero", "an empty name is ignored")
        // A saved document is renamed on disk, keeping its extension; the old file is gone.
        let url = tmp("rename-me.aseprite")
        mark(d, 1, 1, RED)
        m.fileURL = url
        m.save()
        m.renameDocument(first, to: "walk-cycle.aseprite")
        let renamed = url.deletingLastPathComponent().appendingPathComponent("walk-cycle.aseprite")
        expectEq(m.fileURL, renamed)
        expectEq(m.name(of: first), "walk-cycle.aseprite")
        expect(FileManager.default.fileExists(atPath: renamed.path), "renamed file exists")
        expect(!FileManager.default.fileExists(atPath: url.path), "old file is gone")
        // Renaming onto an existing file is refused.
        let taken = tmp("taken.aseprite")
        FileManager.default.createFile(atPath: taken.path, contents: Data())
        m.renameDocument(first, to: "taken")
        expectEq(m.fileURL, renamed, "an existing file is not overwritten")
        expect(m.errorMessage != nil, "the user is told why")
        m.errorMessage = nil
    }

    scenario("tabs/open-already-open-file") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        let url = tmp("open-twice.aseprite")
        mark(d, 1, 1, RED)
        m.fileURL = url
        m.save()
        expect(!m.isDirty)
        let other = addTab()
        mark(d, 5, 5, BLUE)
        expectEq(m.slots.count, 2)
        expect(m.activeSlotObject === other)

        m.open(url)
        expectEq(m.slots.count, 2, "opening a file that is already open adds no tab")
        expectEq(m.activeSlot, 0, "it switches to the existing tab")
        expectPixel(1, 1, RED)

        // A different spelling of the same path (symlink) is the same file.
        let link = tmp("open-twice-link.aseprite")
        try? FileManager.default.createSymbolicLink(at: link, withDestinationURL: url)
        m.activate(1)
        m.open(link)
        expectEq(m.slots.count, 2, "symlinked path is recognised")
        expectEq(m.activeSlot, 0)

        // Unsaved edits in the open tab are not lost by opening it again.
        mark(d, 2, 2, GREEN)
        m.open(url)
        expectPixel(2, 2, GREEN)
        expectEq(m.slots.count, 2)
        expect(m.isDirty)

        // A different file does get its own tab.
        let url2 = tmp("open-other.aseprite")
        m.activate(1)
        m.fileURL = url2
        m.save()
        m.closeTab(1)
        expectEq(m.slots.count, 1)
        m.open(url2)
        expectEq(m.slots.count, 2)
        expectPixel(5, 5, BLUE)
    }

    scenario("tabs/open-replaces-pristine-tab") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        let url = tmp("pristine.aseprite")
        mark(d, 1, 1, RED)
        m.fileURL = url
        m.save()
        m.closeActiveTab()
        expect(m.activeIsPristine)
        m.open(url)
        expectEq(m.slots.count, 1, "a pristine untitled tab is replaced, like a browser's blank tab")
        expectPixel(1, 1, RED)
        // New Sprite on a used tab opens another one.
        m.newSprite(width: 8, height: 8)
        expectEq(m.slots.count, 2)
        expectEq(m.width, 8)
        // New Sprite again while the new one is pristine reuses it.
        m.newSprite(width: 10, height: 10)
        expectEq(m.slots.count, 2)
        expectEq(m.width, 10)
    }

    scenario("tabs/tab-order-and-navigation") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 0, 0, RED)
        let a = m.activeSlotObject!
        let b = addTab(); mark(d, 0, 0, GREEN)
        let c = addTab(); mark(d, 0, 0, BLUE)
        expect(m.slots[0] === a && m.slots[1] === b && m.slots[2] === c)
        m.nextTab()
        expect(m.activeSlotObject === a, "next wraps around")
        m.previousTab()
        expect(m.activeSlotObject === c, "previous wraps around")
        // Dragging a tab keeps the active document active and leaves every document's content alone.
        m.moveTab(c, to: 0)
        expect(m.slots[0] === c && m.activeSlotObject === c)
        expectPixel(0, 0, BLUE)
        expectEq(slotPixel(a, 0, 0), RED)
        expectEq(slotPixel(b, 0, 0), GREEN)
        m.activate(2)
        expect(m.activeSlotObject === b)
        expectPixel(0, 0, GREEN)
        // Untitled numbering is unique per open document.
        let names = m.slots.map { m.name(of: $0) }
        expectEq(Set(names).count, 3, "tab names are distinct: \(names)")
    }

    scenario("tabs/quit-with-several-dirty-tabs") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        // Tabs: 0 dirty, 1 clean (saved), 2 dirty, 3 dirty.
        mark(d, 1, 1, RED)
        addTab(); mark(d, 1, 1, GREEN)
        m.fileURL = tmp("quit-clean.aseprite"); m.save()
        addTab(); mark(d, 1, 1, BLUE)
        addTab(); mark(d, 1, 1, WHITE)
        m.activate(1)
        expectEq(m.slots.filter { m.isDirty($0) }.count, 3)

        // Cancel on the second question: only the first two dirty tabs were asked, in tab order,
        // each one shown while asked; quitting is cancelled and nothing was closed.
        let cancelAt2 = AlertScript([AlertScript.dontSave, AlertScript.cancel])
        cancelAt2.install()
        expect(m.terminateReply() == .terminateCancel)
        expectEq(cancelAt2.prompts.count, 2)
        expectEq(cancelAt2.activeAtPrompt, [0, 2], "dirty tabs only, in order, each brought to the front")
        expectEq(m.slots.count, 4)
        expectEq(m.activeSlot, 2, "the tab that was cancelled stays in front")
        expect(!WindowCloseGuard.approved, "a cancelled quit must not pre-approve the next window close")

        // All Don't Save: three questions, then terminate; recovery copies are removed.
        let all = AlertScript([AlertScript.dontSave, AlertScript.dontSave, AlertScript.dontSave])
        all.install()
        expect(m.terminateReply() == .terminateNow)
        expectEq(all.prompts.count, 3)
        expectEq(all.activeAtPrompt, [0, 2, 3])
        for p in all.prompts { expect(p.contains("Untitled"), "prompt names the document: \(p)") }

        // Save on one of them writes it and carries on to the next.
        let saving = AlertScript([AlertScript.save, AlertScript.dontSave, AlertScript.dontSave])
        saving.install()
        m.activate(0)
        m.fileURL = tmp("quit-save.aseprite")
        expect(m.terminateReply() == .terminateNow)
        expect(FileManager.default.fileExists(atPath: tmp("quit-save.aseprite").path))
    }

    scenario("tabs/window-close-asks-once") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs(); WindowCloseGuard.approved = false }
        mark(d, 1, 1, RED)
        addTab(); mark(d, 1, 1, GREEN)
        let script = AlertScript([AlertScript.dontSave, AlertScript.dontSave])
        script.install()
        // Closing the window asks about every dirty tab (windowShouldClose); the quit that follows must not ask again.
        let approved = WindowCloseGuard.shared.windowShouldClose(d.window)
        expect(approved)
        expectEq(script.prompts.count, 2)
        expect(WindowCloseGuard.approved)
        expect(m.terminateReply() == .terminateNow)
        expectEq(script.prompts.count, 2, "no second round of questions")
    }

    scenario("tabs/autosave-several-tabs") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        Autosaver.shared.start(m)
        mark(d, 1, 1, RED)
        let a = m.activeSlotObject!
        let b = addTab(); mark(d, 2, 2, BLUE)
        let c = addTab() // clean
        m.activate(1) // b active, a in the background, c in the background and clean
        Autosaver.shared.tick()
        let fm = FileManager.default
        // The copies are written on a background queue and recorded on main when done.
        waitUntil { a.lastAutosavedRevision != nil && b.lastAutosavedRevision != nil }
        expect(fm.fileExists(atPath: recoveryFile(a).path), "background dirty tab has a recovery copy")
        expect(fm.fileExists(atPath: recoveryFile(b).path), "active dirty tab has one")
        expect(!fm.fileExists(atPath: recoveryFile(c).path), "clean tab has none")
        // Each copy holds its own tab's content.
        let ra = recoveryFile(a).path.withCString { pixmith_open($0) }
        let rb = recoveryFile(b).path.withCString { pixmith_open($0) }
        expect(ra != nil && rb != nil)
        if let ra, let rb {
            let sa = DocumentSlot(doc: ra), sb = DocumentSlot(doc: rb)
            expectEq(slotPixel(sa, 1, 1), RED)
            expectEq(slotPixel(sa, 2, 2), CLEAR)
            expectEq(slotPixel(sb, 2, 2), BLUE)
            expectEq(slotPixel(sb, 1, 1), CLEAR)
            pixmith_free(ra)
            pixmith_free(rb)
        }
        // An unchanged tab is not rewritten; an edit in the background tab is picked up after switching back.
        let stamp = (try? fm.attributesOfItem(atPath: recoveryFile(a).path)[.modificationDate]) as? Date
        Autosaver.shared.tick()
        pumpRunLoop(0.3)
        expectEq((try? fm.attributesOfItem(atPath: recoveryFile(a).path)[.modificationDate]) as? Date, stamp, "unchanged tab is skipped")
        m.activate(0)
        mark(d, 4, 4, GREEN)
        let revisionA = a.lastAutosavedRevision
        // Edits after an idle moment are published a display refresh later (ChangeCoalescer, driven by a display link
        // that does not fire in this offscreen window); the autosave timer in the real app runs after that tick.
        ChangeCoalescer.shared.flush()
        Autosaver.shared.tick()
        waitUntil { a.lastAutosavedRevision != revisionA }
        let ra2 = recoveryFile(a).path.withCString { pixmith_open($0) }
        expect(ra2 != nil)
        if let ra2 {
            expectEq(slotPixel(DocumentSlot(doc: ra2), 4, 4), GREEN, "the new edit reached the recovery copy")
            pixmith_free(ra2)
        }
        // Saving a tab removes its copy; closing a tab removes its copy; undoing to clean removes it too.
        m.fileURL = tmp("autosave-a.aseprite")
        m.save()
        expect(!fm.fileExists(atPath: recoveryFile(a).path), "saved tab's recovery copy is removed")
        m.activate(1)
        let script = AlertScript([AlertScript.dontSave])
        script.install()
        m.closeTab(1)
        expect(!fm.fileExists(atPath: recoveryFile(b).path), "closed tab's recovery copy is removed")
        Autosaver.shared.discardAll()
    }

    scenario("tabs/lua-script-on-active-tab") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 1, 1, RED)
        let a = m.activeSlotObject!
        let b = addTab(); mark(d, 2, 2, BLUE)
        let script = tmp("tab.lua")
        try? "sprite:setPixel(7, 7, color.rgba(0, 255, 0, 255), sprite.activeLayer, sprite.activeFrame)\napp.undoLabel('Tab script')"
            .write(to: script, atomically: true, encoding: .utf8)

        // The script runs as a background job on a snapshot and is applied on the main queue when it finishes.
        runScriptAndWait(m, script)
        expectPixel(7, 7, GREEN, "the script ran on the active tab")
        expectEq(slotPixel(a, 7, 7), CLEAR, "and not on the other one")
        expectEq(slotHistory(b).undo, 2, "one undo step in the active tab")
        expectEq(slotHistory(a).undo, 1)

        m.activate(0)
        runScriptAndWait(m, script)
        expectEq(slotPixel(b, 7, 7), GREEN, "tab 2 keeps its own result")
        expectPixel(7, 7, GREEN)
        m.undo()
        expectPixel(7, 7, CLEAR)
        expectEq(slotPixel(b, 7, 7), GREEN, "undo in tab 1 does not undo tab 2's script")

        // A failing script reports an error alert and changes nothing in either tab.
        let bad = tmp("bad.lua")
        try? "sprite:setPixel(1, 1, color.rgba(1, 2, 3, 255)\nthis is not lua".write(to: bad, atomically: true, encoding: .utf8)
        let (hist0, hist1) = (slotHistory(a).undo, slotHistory(b).undo)
        answerModal("OK") { runScriptAndWait(m, bad) }
        expectEq(slotHistory(a).undo, hist0)
        expectEq(slotHistory(b).undo, hist1)
        expectPixel(1, 1, RED)
    }

    scenario("tabs/many-tabs-churn") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        SpriteModel.alertRunner = { _ in AlertScript.dontSave }
        var expected: [Int: UInt32] = [:]
        mark(d, 0, 0, RED)
        let colors = [RED, GREEN, BLUE, WHITE, 0xFF00FFFF, 0x00FFFFFF]
        var ids: [UUID] = [m.activeSlotID!]
        for i in 1..<6 {
            addTab(8 + i, 8 + i)
            mark(d, i, i, colors[i])
            ids.append(m.activeSlotID!)
        }
        for (i, c) in colors.enumerated() { expected[i] = c }
        // Visit every tab, close the middle ones, and check each survivor still shows its own marker.
        for i in [3, 0, 5, 2, 4, 1] {
            guard let at = m.slots.firstIndex(where: { $0.id == ids[i] }) else { continue }
            m.activate(at)
            expectPixel(i, i, expected[i]!, "tab \(i) shows its own pixel")
            expectEq(m.width, 8 + i == 8 ? 16 : 8 + i, "tab \(i) keeps its own size")
        }
        for i in [2, 4] {
            if let at = m.slots.firstIndex(where: { $0.id == ids[i] }) { m.closeTab(at) }
        }
        expectEq(m.slots.count, 4)
        for i in [0, 1, 3, 5] {
            guard let at = m.slots.firstIndex(where: { $0.id == ids[i] }) else { expect(false, "tab \(i) vanished"); continue }
            m.activate(at)
            expectPixel(i, i, expected[i]!)
            expectEq(m.historyCounts.undo, 1)
        }
    }
}
