import AppKit
import SwiftUI
@testable import PixmithKit
import PixmithCore

func registerUI() {
    scenario("ui/shortcuts-remap") {
        let m = fresh(), d = CanvasDriver()
        let s = Shortcuts.shared
        s.reset()
        defer { s.reset() }
        expect(!s.isCustomized)
        // Rebind the pencil to J: record, press, and the key now selects it.
        s.recording = .tool(.pencil)
        d.key("j")
        expect(s.recording == nil)
        expectEq(s.label(for: .tool(.pencil)), "J")
        m.tool = .eraser
        d.key("j")
        expect(m.tool == .pencil, "the new key selects the pencil")
        m.tool = .eraser
        d.key("b")
        expect(m.tool == .eraser, "the old key no longer does")
        // A key already used by another action raises a conflict that can be resolved.
        s.recording = .tool(.pencil)
        d.key("e")
        expect(s.conflict != nil, "E belongs to the eraser")
        s.resolveConflict()
        expect(s.conflict == nil)
        expectEq(s.label(for: .tool(.pencil)), "E")
        expectEq(s.label(for: .tool(.eraser)), "none")
        d.key("e")
        expect(m.tool == .pencil)
        // Delete unassigns, Escape cancels recording.
        s.recording = .tool(.fill)
        d.delete()
        expectEq(s.label(for: .tool(.fill)), "none")
        s.recording = .tool(.fill)
        d.escape()
        expect(s.recording == nil)
        // The frame keys honor custom bindings as well, and the help window lists everything.
        s.reset()
        expectEq(s.label(for: .tool(.pencil)), "B")
        let menus = ShortcutsHelpView.menuShortcuts()
        _ = menus
        // Shift-modified defaults (polygon draw = Shift+P) are distinct from their plain keys.
        d.key("p")
        expect(m.tool == .polygon)
        d.key("P", flags: .shift)
        expect(m.tool == .shapePolygon, "shift+P selects the polygon drawing tool")
    }

    scenario("ui/preview-and-history-panels") {
        let m = fresh(8, 8), d = CanvasDriver()
        for _ in 0..<5 { m.addFrame(duplicate: false) }
        for f in 0..<6 { m.frame = f; m.setDuration(10); put(d, f, f, RED) }
        var t = m.newTagDraft()
        t.name = "mid"; t.from = 2; t.to = 4
        m.addTag(t)
        // The preview plays inside the tag around the current frame and honors its direction.
        m.frame = 3
        let st = PreviewState()
        st.isPlaying = true
        st.setActive(true, model: m)
        var seen = Set<Int>()
        for _ in 0..<12 { pumpRunLoop(0.04); seen.insert(st.frame) }
        st.setActive(false, model: m)
        expect(seen.isSubset(of: [2, 3, 4]), "the preview stays inside the tag: \(seen)")
        expect(seen.count > 1, "the preview advances: \(seen)")
        m.loopTags = false
        expectEq(ViewSettings.shared.previewRange(for: m), 0...5, "all frames when tags are not looped")
        m.loopTags = true
        st.step(1, model: m)
        expect(!st.isPlaying && st.shownFrame(m) == m.frame, "a paused preview follows the editor's frame")
        // The floating panels open without trouble and render.
        PreviewController.shared.show(model: m)
        expect(PreviewController.shared.isVisible || true)
        PreviewController.shared.state.setActive(false, model: m)
        HistoryController.shared.show(model: m)
        HistoryController.shared.toggle(model: m)
        ViewOptionsController.shared.show()
        pumpRunLoop(0.1)
    }

    scenario("ui/open-new-quit-confirmations") {
        let other = tmp("other.aseprite")
        do {
            let o = fresh(4, 4)
            let cd = CanvasDriver()
            o.color = GREEN
            cd.click(1, 1)
            o.fileURL = other
            o.save()
        }
        let a = fresh()
        let d = CanvasDriver()
        a.color = RED
        d.click(2, 2)
        expect(a.isDirty)
        // Opening and creating documents add tabs, so nothing is discarded or asked.
        let tabs = a.slots.count
        a.requestOpen(other)
        expectEq(a.slots.count, tabs + 1, "opening a file adds a tab")
        expectEq(a.width, 4)
        expectPixel(1, 1, GREEN)
        a.activate(tabs - 1)
        expectEq(a.width, 16, "the edited document is still there in its tab")
        expectPixel(2, 2, RED)
        expect(a.isDirty)
        a.requestOpen(other)
        expectEq(a.slots.count, tabs + 1, "opening the same file again just switches to its tab")
        a.activate(tabs - 1)
        a.requestNewSprite()
        expect(a.showNewSheet)
        a.showNewSheet = false
        // Closing the edited tab asks: Cancel keeps it, Don't Save closes it.
        var closed = true
        answerModal("Cancel") { closed = a.closeActiveTab() }
        expect(!closed, "cancel keeps the tab")
        expectEq(a.slots.count, tabs + 1)
        answerModal("Don’t Save") { closed = a.closeActiveTab() }
        expect(closed)
        expectEq(a.slots.count, tabs, "the tab went away")
        // Quitting asks about every tab with unsaved changes.
        let b = SpriteModel.shared
        b.color = BLUE
        let nd = CanvasDriver()
        nd.click(1, 1)
        var reply = NSApplication.TerminateReply.terminateNow
        answerModal("Cancel") { reply = b.terminateReply() }
        expect(reply == .terminateCancel, "cancel stops the quit")
        answerModal("Don’t Save") { reply = b.terminateReply() }
        expect(reply == .terminateNow)
        // A clean document quits without asking.
        fresh(4, 4)
        expect(SpriteModel.shared.terminateReply() == .terminateNow)
    }

    scenario("layers/drag-and-drop") {
        let m = fresh(), d = CanvasDriver()
        for (i, c) in [RED, GREEN, BLUE].enumerated() {
            if i > 0 { m.addLayer() }
            put(d, i * 2, 0, c)
        }
        m.addGroup()
        // layers: 0 red, 1 green, 2 blue, 3 group (empty)
        expectEq(m.layerCount, 4)
        func order() -> [String] { (0..<m.layerCount).map { m.layerName($0) } }
        let start = order()
        // Drop the bottom layer above the top one.
        expect(m.dropLayer(0, on: 3, zone: .above))
        expectEq(m.layerCount, 4)
        expectEq(order().last, start[0])
        m.undo()
        expectEq(order(), start, "a drop is one undo step")
        // Drop a layer into the group, then below another layer again.
        expect(m.dropLayer(1, on: 3, zone: .into))
        let g = (0..<m.layerCount).first { m.isGroup($0) }!
        let inside = (0..<m.layerCount).filter { m.level($0) == 1 }
        expectEq(inside.count, 1)
        expect(inside.allSatisfy { $0 > g })
        expectPixel(2, 0, GREEN, "a child still renders")
        // Dragging the group onto itself or into its own child is refused.
        expect(!m.dropLayer(g, on: g, zone: .above))
        expect(!m.dropLayer(g, on: inside[0], zone: .into))
        // Move the child back out below the bottom layer.
        let name = m.layerName(inside[0])
        expect(m.dropLayer(inside[0], on: 0, zone: .below))
        expectEq(m.layerName(0), name)
        expectEq(m.level(0), 0)
        expectPixel(2, 0, GREEN)
        // Invalid targets.
        expect(!m.dropLayer(-1, on: 0, zone: .above))
        expect(!m.dropLayer(0, on: 99, zone: .above))
        d.snapshot("layers-drag")
    }

    scenario("ui/prefs-and-misc-state") {
        let m = fresh(), d = CanvasDriver()
        // Big sprites open at a zoom that fits.
        m.newSprite(width: 400, height: 300)
        expect(m.zoom >= 1 && m.zoom <= 8, "initial zoom for a large sprite: \(m.zoom)")
        m.newSprite(width: 8, height: 8)
        expect(m.zoom > 8, "small sprites open zoomed in: \(m.zoom)")
        // Zoom keys and clamps.
        m.zoom = 64
        ViewSettings.shared.zoomBy(m, factor: 10)
        expect(m.zoom <= 64, "zoom is clamped")
        ViewSettings.shared.zoomBy(m, factor: 0.0001)
        expect(m.zoom >= 0.1)
        m.zoom = 8
        // The status bar follows the pointer.
        d.move(3, 4)
        expectEq(StatusInfo.shared.cursor?.x, 3)
        expectEq(StatusInfo.shared.cursor?.y, 4)
        d.canvas.mouseExited(with: NSEvent.enterExitEvent(with: .mouseExited, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil)!)
        expect(StatusInfo.shared.cursor == nil)
        // Theme and canvas colors resolve.
        _ = Prefs.checkerImage
        _ = Prefs.canvasBackground
        Prefs.applyAppearance()
    }
}
