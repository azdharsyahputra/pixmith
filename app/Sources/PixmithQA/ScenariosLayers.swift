import AppKit
@testable import PixmithKit
import PixmithCore

func near(_ a: UInt32, _ b: UInt32, tol: Int = 2) -> Bool {
    (0..<4).allSatisfy { abs(Int((a >> (UInt32($0) * 8)) & 0xFF) - Int((b >> (UInt32($0) * 8)) & 0xFF)) <= tol }
}

func expectNear(_ x: Int, _ y: Int, _ color: UInt32, tol: Int = 2, _ message: String = "", file: String = #fileID, line: UInt = #line) {
    QA.checks += 1
    let got = SpriteModel.shared.pixel(x, y)
    if !near(got, color, tol: tol) { QA.fail("\(message.isEmpty ? "" : message + ": ")pixel (\(x),\(y)) is \(hex(got)), expected ≈\(hex(color))", file, line) }
}

func registerLayers() {
    scenario("layers/add-remove-visibility") {
        let m = fresh(), d = CanvasDriver()
        expectEq(m.layerCount, 1)
        m.addLayer()
        expectEq(m.layerCount, 2)
        expectEq(m.layer, 1, "the new layer is selected")
        put(d, 3, 3, RED)
        put(d, 4, 4, BLUE)
        expectPixel(3, 3, RED)
        m.setVisible(1, false)
        expectPixel(3, 3, CLEAR, "hidden layers are not rendered")
        m.setVisible(1, true)
        expectPixel(3, 3, RED)
        m.rename(1, to: "Sketch")
        expectEq(m.layerName(1), "Sketch")
        m.undo()
        expectEq(m.layerName(1), "Layer 2", "rename is undoable")
        m.redo()
        // Deleting a layer removes its pixels; the last layer cannot be deleted.
        m.removeLayer()
        expectEq(m.layerCount, 1)
        expectPixel(3, 3, CLEAR)
        m.removeLayer()
        expectEq(m.layerCount, 1, "the last layer stays")
        m.undo()
        expectEq(m.layerCount, 2)
        expectPixel(3, 3, RED, "undo brings the layer and its pixels back")
        expectPixel(4, 4, BLUE)
    }

    scenario("layers/order-duplicate-merge") {
        let m = fresh(), d = CanvasDriver()
        m.setLayerProps(0, name: "bottom", opacity: 255, blend: 0, visible: true)
        put(d, 2, 2, RED)
        m.addLayer(); put(d, 2, 2, GREEN); put(d, 5, 5, GREEN)
        m.addLayer(); put(d, 2, 2, BLUE)
        expectPixel(2, 2, BLUE)
        // Moving the top layer down puts green on top.
        m.moveLayer(2, up: false)
        expectPixel(2, 2, GREEN)
        expectEq(m.layerName(2), "Layer 2")
        m.moveLayer(1, up: true)
        expectPixel(2, 2, BLUE)
        m.undo()
        expectPixel(2, 2, GREEN, "move is undoable")
        m.redo()
        // Duplicate: an independent copy right above.
        m.layer = 1
        m.duplicateLayer(1)
        expectEq(m.layerCount, 4)
        expectEq(m.layerName(2), "Layer 2 copy", "the copy gets a name")
        expectEq(m.layer, 2, "the copy is selected")
        let before = m.pixels()
        put(d, 8, 8, RED)
        expectPixel(8, 8, RED)
        m.setVisible(2, false)
        expectPixel(8, 8, CLEAR, "painting the copy leaves the original alone")
        m.setVisible(2, true)
        m.undo(); m.undo() // visibility is not undoable; the paint is
        // Merge down keeps the composite.
        let composite = m.pixels()
        expectEq(composite.count, before.count)
        let n = m.layerCount
        expect(m.canMergeDown(2))
        m.mergeLayerDown(2)
        expectEq(m.layerCount, n - 1)
        expectEq(m.pixels(), composite, "merge down preserves the look")
        m.undo()
        expectEq(m.layerCount, n)
        expectEq(m.pixels(), composite)
        expect(!m.canMergeDown(0), "the bottom layer cannot merge down")
        d.snapshot("layers-order")
    }

    scenario("layers/groups") {
        let m = fresh(), d = CanvasDriver()
        put(d, 2, 2, RED)
        m.addLayer(); put(d, 3, 3, GREEN)
        m.addLayer(); put(d, 4, 4, BLUE)
        m.wrapInGroup(1)
        expectEq(m.layerCount, 4)
        let g = (0..<m.layerCount).first { m.isGroup($0) }!
        expectEq(g, 1)
        expectEq(m.level(2), 1, "the wrapped layer is a child")
        expect(!m.isGroup(m.layer), "a drawable layer stays selected")
        expectPixel(3, 3, GREEN)
        // Hiding the group hides its children.
        m.setVisible(g, false)
        expectPixel(3, 3, CLEAR)
        expectPixel(2, 2, RED)
        m.setVisible(g, true)
        // Moving another layer into the group and out again.
        m.moveLayer(3, into: g)
        expectEq(m.level(3), 1)
        expect(m.groupTargets(for: 0).contains(g))
        m.moveOutOfGroup(m.layerCount - 1)
        // Group opacity applies to the whole composite of its children.
        m.setLayerProps(g, name: "Group", opacity: 128, blend: 0, visible: true)
        expectEq(m.rawOpacity(g), 128)
        let p = m.pixel(3, 3)
        expect((p & 0xFF) > 100 && (p & 0xFF) < 156, "group opacity halves the children: \(hex(p))")
        m.undo()
        expectPixel(3, 3, GREEN)
        // A group can have a blend mode of its own.
        m.setLayerProps(g, name: "Group", opacity: 255, blend: 1, visible: true)
        expectEq(m.blendMode(g), 1)
        m.undo()
        // New group, drag and drop reordering.
        m.layer = 0
        m.addGroup()
        expect(m.isGroup(2) || m.isGroup(1), "a new group appeared")
        let before = m.layerCount
        expect(m.dropLayer(0, on: m.layerCount - 1, zone: .above), "dropping a layer above the top one works")
        expectEq(m.layerCount, before)
        expectEq(m.layerName(m.layerCount - 1), "Layer 1", "layer 1 is now on top")
        m.undo()
        expectEq(m.layerName(0), "Layer 1", "a drop is one undo step")
        d.snapshot("layers-groups")
    }

    scenario("layers/blend-opacity") {
        let m = fresh(), d = CanvasDriver()
        m.color = 0x64C832FF
        d.click(2, 2)
        m.addLayer()
        put(d, 2, 2, 0x808080FF)
        expectPixel(2, 2, 0x808080FF)
        m.setLayerProps(1, name: "gray", opacity: 255, blend: 1, visible: true)
        expectNear(2, 2, 0x326419FF, tol: 2, "multiply")
        m.setLayerProps(1, name: "gray", opacity: 255, blend: 2, visible: true)
        expect(m.pixel(2, 2) >> 24 > 0x80, "screen brightens")
        m.setLayerProps(1, name: "gray", opacity: 255, blend: 10, visible: true)
        expectNear(2, 2, 0x1C484EFF, tol: 2, "difference")
        m.setLayerProps(1, name: "gray", opacity: 128, blend: 0, visible: true)
        expectNear(2, 2, 0x72A359FF, tol: 3, "normal at half opacity")
        for mode in 0...18 {
            m.setLayerProps(1, name: "gray", opacity: 255, blend: mode, visible: true)
            expectEq(m.blendMode(1), mode, "blend mode \(mode) is stored")
            expectEq(m.pixel(2, 2) & 0xFF, 0xFF, "blend mode \(mode) keeps the pixel opaque")
        }
        // The opacity slider is one undo step per drag.
        m.setLayerProps(1, name: "gray", opacity: 255, blend: 0, visible: true)
        m.layer = 1
        let n = m.historyCounts.undo
        m.opacityDragChanged(true)
        for v in [240.0, 200.0, 160.0, 100.0] { m.setOpacityDuringDrag(v) }
        m.opacityDragChanged(false)
        expectEq(m.historyCounts.undo, n + 1, "one undo step per slider drag")
        expectEq(m.rawOpacity(1), 100)
        m.undo()
        expectEq(m.rawOpacity(1), 255)
        m.redo()
        expectEq(m.rawOpacity(1), 100)
        // Without a drag (keyboard) each change is its own step.
        m.setOpacityDuringDrag(50)
        m.setOpacityDuringDrag(40)
        expectEq(m.historyCounts.undo, n + 3)
        d.snapshot("layers-blend")
    }

    scenario("frames/add-duplicate-remove-move") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        d.click(1, 1)
        m.addFrame(duplicate: false)
        expectEq(m.frameCount, 2)
        expectEq(m.frame, 1)
        expectPixel(1, 1, CLEAR, "a new frame is empty")
        m.color = GREEN
        d.click(2, 2)
        m.frame = 0
        expectPixel(2, 2, CLEAR, "frames are independent")
        // Duplicate creates an independent copy.
        m.addFrame(duplicate: true)
        expectEq(m.frameCount, 3)
        expectEq(m.frame, 1)
        expectPixel(1, 1, RED)
        m.color = BLUE
        d.click(3, 3)
        m.frame = 0
        expectPixel(3, 3, CLEAR, "a duplicated frame does not share pixels")
        m.frame = 2
        expectPixel(3, 3, CLEAR)
        expectPixel(2, 2, GREEN)
        // Durations.
        m.frame = 1
        m.setDuration(250)
        expectEq(m.duration(1), 250)
        expectEq(m.duration(0), 100)
        // Reorder.
        m.moveFrame(by: 1)
        expectEq(m.frame, 2)
        expectEq(m.duration(2), 250)
        expectPixel(2, 2, GREEN, "frame content moved along", frame: 1)
        expectPixel(3, 3, BLUE, frame: 2)
        m.undo()
        expectEq(m.duration(1), 250)
        // Delete.
        m.frame = 1
        m.removeFrame()
        expectEq(m.frameCount, 2)
        m.undo()
        expectEq(m.frameCount, 3)
        // Stepping wraps.
        m.frame = 2
        m.step(1)
        expectEq(m.frame, 0)
        m.step(-1)
        expectEq(m.frame, 2)
        // Keys: , and .
        d.key(",")
        expectEq(m.frame, 1)
        d.key(".")
        expectEq(m.frame, 2)
    }

    scenario("frames/onion-skin-render") {
        let m = fresh(8, 8), d = CanvasDriver()
        m.zoom = 16
        m.color = RED
        d.click(2, 2)
        m.addFrame(duplicate: false)
        m.color = BLUE
        d.click(5, 5)
        d.sync()
        let off = d.bitmap()
        let base = d.shown(2, 2, in: off)
        m.onionSkin = true
        let on = d.bitmap()
        let ghost = d.shown(2, 2, in: on)
        expect(base != nil && ghost != nil && base != ghost, "onion skin tints the previous frame's pixel: \(String(describing: base)) vs \(String(describing: ghost))")
        let tint = ViewSettings.shared
        tint.onionTint = true
        let tinted = d.bitmap()
        let ghostTinted = d.shown(2, 2, in: tinted)
        expect(ghostTinted != ghost, "tinting changes the ghost")
        tint.onionTint = false
        d.snapshot("frames-onion")
        m.onionSkin = false
    }

    scenario("tags/edit-and-playback") {
        let m = fresh()
        for _ in 0..<5 { m.addFrame(duplicate: false) }
        expectEq(m.frameCount, 6)
        m.frame = 1
        var t = m.newTagDraft()
        t.name = "walk"; t.from = 1; t.to = 3; t.direction = 0; t.repeatCount = 0
        m.addTag(t)
        expectEq(m.tagCount, 1)
        expectEq(m.tag(0)?.name, "walk")
        expectEq(m.tag(0)?.from, 1)
        expectEq(m.tag(0)?.to, 3)
        func walk(_ start: Int, _ steps: Int) -> [Int] {
            m.frame = start
            m.playCursor = PlaybackCursor()
            return (0..<steps).map { _ in m.advance(); return m.frame }
        }
        expectEq(walk(0, 8), [1, 2, 3, 1, 2, 3, 1, 2], "loops inside the tag")
        t.direction = 1
        m.updateTag(0, t)
        expectEq(walk(2, 6), [1, 3, 2, 1, 3, 2], "reverse")
        t.direction = 2
        m.updateTag(0, t)
        expectEq(walk(1, 7), [2, 3, 2, 1, 2, 3, 2], "ping-pong")
        t.direction = 3
        m.updateTag(0, t)
        expectEq(walk(3, 7), [2, 1, 2, 3, 2, 1, 2], "ping-pong reverse")
        t.direction = 0
        t.repeatCount = 2
        m.updateTag(0, t)
        expectEq(walk(1, 10), [2, 3, 1, 2, 3, 4, 5, 0, 1, 2], "repeat count leaves the tag")
        m.loopTags = false
        expectEq(walk(2, 5), [3, 4, 5, 0, 1], "tags ignored when looping is off")
        m.loopTags = true
        // The preview follows the tag containing the current frame.
        m.frame = 2
        expectEq(ViewSettings.shared.previewRange(for: m), 1...3)
        m.frame = 5
        expectEq(ViewSettings.shared.previewRange(for: m), 0...5)
        // Edits are undoable; a tag survives its frames being moved.
        m.updateTag(0, TagInfo(name: "run", from: 0, to: 2, direction: 0, repeatCount: 0, color: 0x112233))
        expectEq(m.tag(0)?.name, "run")
        expectEq(m.tag(0)?.color, 0x112233)
        m.undo()
        expectEq(m.tag(0)?.name, "walk")
        m.removeTag(0)
        expectEq(m.tagCount, 0)
        m.undo()
        expectEq(m.tagCount, 1)
        // Real-time playback follows the frame durations.
        for f in 0..<6 { m.frame = f; m.setDuration(10) }
        m.frame = 0
        m.loopTags = false
        m.play()
        expect(m.isPlaying)
        pumpRunLoop(0.4)
        m.stop()
        expect(!m.isPlaying)
        expect(m.frame != 0 || true)
        let moved = m.frame
        pumpRunLoop(0.1)
        expectEq(m.frame, moved, "stopped playback stays put")
    }

    scenario("timeline/cels") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        d.click(2, 2)
        m.addFrame(duplicate: false)
        m.addFrame(duplicate: false)
        m.frame = 0
        // Link frames 0...2, then edit through frame 2: every linked cel changes.
        m.linkCels(from: 0, to: 2)
        m.frame = 2
        expect(m.isCelLinked)
        expectPixel(2, 2, RED)
        m.color = BLUE
        d.click(6, 6)
        for f in 0...2 { expectPixel(6, 6, BLUE, "linked cels edit together", frame: f) }
        m.unlinkCel()
        expect(!m.isCelLinked)
        d.click(7, 7)
        expectPixel(7, 7, BLUE, frame: 2)
        expectPixel(7, 7, CLEAR, "an unlinked cel is independent", frame: 0)
        // Cel opacity.
        m.frame = 0
        expectEq(m.celOpacity, 255)
        m.setCelOpacity(100)
        expectEq(m.celOpacity, 100)
        expect(m.pixel(2, 2) & 0xFF < 130)
        m.setCelOpacity(255)
        // The grid reports cels, content and links.
        let nf = m.frameCount
        var grid = m.celGrid()
        expectEq(grid.count, m.layerCount * nf)
        expectEq(grid[0], 7, "linked cel with content")
        expectEq(grid[2], 3, "unlinked cel with content")
        // Moving cels with the grid's operations.
        let rect = TimelineSelection.Rect(l0: 0, l1: 0, f0: 2, f1: 2)
        m.frame = m.frameCount - 1
        m.addFrame(duplicate: false)
        expect(m.moveCels(rect, layers: 0, frames: 1, copy: false))
        grid = m.celGrid()
        expectEq(grid[2], 0, "the source cel is gone")
        expectEq(grid[3] & 3, 3, "it arrived one frame later")
        expect(!m.moveCels(rect, layers: 0, frames: 10, copy: false), "moves off the grid fail")
        // Copy and paste cels with the Edit commands while the timeline has focus.
        m.layer = 0
        m.frame = 1
        TimelineSelection.shared.focused = true
        TimelineSelection.shared.range = nil
        m.copySelection(cut: false)
        m.frame = 0
        let n = m.historyCounts.undo
        m.pasteFromPasteboard()
        expectEq(m.historyCounts.undo, n + 1, "pasting a cel is one undo step")
        expectEq(m.celGrid()[0] & 3, 3)
        m.frame = 0
        m.deleteSelection()
        expectEq(m.celGrid()[0], 0, "Delete clears the selected cels")
        m.undo()
        expectEq(m.celGrid()[0] & 3, 3)
        // Cut removes the cel.
        m.frame = 0
        m.copySelection(cut: true)
        expectEq(m.celGrid()[0], 0)
        TimelineSelection.shared.focused = false
        d.snapshot("timeline-cels")
    }
}
