import AppKit
import PixmithCore
import SwiftUI

struct SliceInfo {
    var x: Int, y: Int, w: Int, h: Int
    var center: (x: Int, y: Int, w: Int, h: Int)?
    var pivot: (x: Int, y: Int)?
}

/// Selection and drag state of the slice tool (the model itself cannot hold extension state).
private final class SliceState {
    static let shared = SliceState()
    enum Mode { case create, move, resize(Int, Int) }
    var selected: Int?
    var active = false
    var mode = Mode.create
    var start = (0, 0)
    var original = (x: 0, y: 0, w: 0, h: 0)
    var began = false
    weak var canvas: NSView?
}

extension SpriteModel {
    var sliceCount: Int { Int(pixmith_slice_count(doc)) }
    var sliceDragging: Bool { SliceState.shared.active }
    var hasSelectedSlice: Bool { SliceState.shared.selected.map { $0 < sliceCount } ?? false }
    var selectedSliceIndex: Int? { SliceState.shared.selected.flatMap { $0 < sliceCount ? $0 : nil } }

    /// Selects a slice from the inspector list (view state only; the document is untouched).
    func selectSlice(_ i: Int?) {
        SliceState.shared.selected = i
        objectWillChange.send()
    }

    func sliceName(_ i: Int) -> String {
        guard let p = pixmith_slice_name(doc, UInt32(i)) else { return "" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    func sliceInfo(_ i: Int) -> SliceInfo? {
        var v = [Int32](repeating: 0, count: 13)
        guard pixmith_slice_get(doc, UInt32(i), UInt32(frame), &v) else { return nil }
        let n = v.map(Int.init)
        return SliceInfo(x: n[0], y: n[1], w: n[2], h: n[3],
                         center: n[4] != 0 ? (n[5], n[6], n[7], n[8]) : nil,
                         pivot: n[9] != 0 ? (n[10], n[11]) : nil)
    }


    func tagName(_ i: Int) -> String {
        guard let p = pixmith_ud_tag_name(doc, UInt32(i)) else { return "" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    func userDataColor(_ kind: UInt32, _ a: Int, _ b: Int = 0) -> UInt32? {
        var c: UInt32 = 0
        return pixmith_userdata_color(doc, kind, UInt32(a), UInt32(b), &c) ? c : nil
    }

    func userDataText(_ kind: UInt32, _ a: Int, _ b: Int = 0) -> String {
        guard let p = pixmith_userdata_text(doc, kind, UInt32(a), UInt32(b)) else { return "" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    // MARK: Mouse

    private static let handles = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]

    private func handlePoint(_ info: SliceInfo, _ h: (Int, Int)) -> CGPoint {
        let z = zoom
        let x = h.0 < 0 ? CGFloat(info.x) : h.0 > 0 ? CGFloat(info.x + info.w) : CGFloat(info.x) + CGFloat(info.w) / 2
        let y = h.1 < 0 ? CGFloat(info.y) : h.1 > 0 ? CGFloat(info.y + info.h) : CGFloat(info.y) + CGFloat(info.h) / 2
        return CGPoint(x: x * z, y: y * z)
    }

    func sliceMouseDown(_ p: (Int, Int), event e: NSEvent) {
        stop()
        let s = SliceState.shared
        s.start = p
        s.began = false
        s.active = true
        if let sel = s.selected, let info = sliceInfo(sel), let canvas = s.canvas {
            let pt = canvas.convert(e.locationInWindow, from: nil)
            if let h = Self.handles.first(where: { hypot(handlePoint(info, $0).x - pt.x, handlePoint(info, $0).y - pt.y) <= min(7, CGFloat(min(info.w, info.h)) * zoom / 3) }) {
                s.mode = .resize(h.0, h.1)
                s.original = (info.x, info.y, info.w, info.h)
                return
            }
        }
        if let hit = (0..<sliceCount).reversed().first(where: {
            guard let i = sliceInfo($0) else { return false }
            return p.0 >= i.x && p.0 < i.x + i.w && p.1 >= i.y && p.1 < i.y + i.h
        }), let info = sliceInfo(hit) {
            s.selected = hit
            s.mode = .move
            s.original = (info.x, info.y, info.w, info.h)
            changed()
            if e.clickCount >= 2 { s.active = false; showSliceSheet(hit) }
            return
        }
        s.selected = nil
        s.mode = .create
        changed()
    }

    func sliceMouseDragged(_ p: (Int, Int)) {
        let s = SliceState.shared
        guard s.active, p != s.start || s.began else { return }
        let o = s.original
        switch s.mode {
        case .create:
            let (x0, x1) = (min(s.start.0, p.0), max(s.start.0, p.0))
            let (y0, y1) = (min(s.start.1, p.1), max(s.start.1, p.1))
            if !s.began {
                s.selected = Int(pixmith_slice_add(doc, Int32(x0), Int32(y0), UInt32(x1 - x0 + 1), UInt32(y1 - y0 + 1)))
                labelLastStep("New Slice")
            } else if let i = s.selected {
                _ = pixmith_slice_set_bounds(doc, UInt32(i), UInt32(frame), Int32(x0), Int32(y0), UInt32(x1 - x0 + 1), UInt32(y1 - y0 + 1))
            }
        case .move:
            guard let i = s.selected else { return }
            if !s.began { pixmith_begin_stroke(doc); labelLastStep("Move Slice") }
            _ = pixmith_slice_set_bounds(doc, UInt32(i), UInt32(frame), Int32(o.x + p.0 - s.start.0),
                                      Int32(o.y + p.1 - s.start.1), UInt32(o.w), UInt32(o.h))
        case let .resize(hx, hy):
            guard let i = s.selected else { return }
            if !s.began { pixmith_begin_stroke(doc); labelLastStep("Resize Slice") }
            var (x0, x1, y0, y1) = (o.x, o.x + o.w - 1, o.y, o.y + o.h - 1)
            if hx < 0 { x0 = p.0 } else if hx > 0 { x1 = p.0 }
            if hy < 0 { y0 = p.1 } else if hy > 0 { y1 = p.1 }
            let (nx, nw) = (min(x0, x1), abs(x1 - x0) + 1)
            let (ny, nh) = (min(y0, y1), abs(y1 - y0) + 1)
            _ = pixmith_slice_set_bounds(doc, UInt32(i), UInt32(frame), Int32(nx), Int32(ny), UInt32(nw), UInt32(nh))
        }
        s.began = true
        changed()
    }

    func sliceMouseUp() {
        SliceState.shared.active = false
    }

    func deleteSelectedSlice() {
        guard let i = SliceState.shared.selected, i < sliceCount else { return }
        if pixmith_slice_remove(doc, UInt32(i)) {
            labelLastStep("Delete Slice")
            SliceState.shared.selected = nil
            changed()
        }
    }

    func showSliceSheet(_ i: Int? = nil) {
        guard let i = i ?? SliceState.shared.selected, i < sliceCount else { return }
        presentSheet { dismiss in SliceSheet(model: self, index: i, dismiss: dismiss) }
    }

    func showUserDataSheet(kind: UInt32) {
        presentSheet { dismiss in UserDataSheet(model: self, kind: kind, dismiss: dismiss) }
    }
}

/// Shows a SwiftUI view as a sheet on the key window; the view calls `dismiss` to close it.
func presentSheet<V: View>(_ build: (@escaping () -> Void) -> V) {
    guard let parent = NSApp.keyWindow, parent.attachedSheet == nil else { return }
    let win = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: false)
    win.contentViewController = NSHostingController(rootView: build { [weak parent, weak win] in
        if let win { parent?.endSheet(win) }
    })
    parent.beginSheet(win)
}

extension CanvasNSView {
    /// Slice outlines with name labels; the selected slice also shows resize handles.
    func drawSlices(_ ctx: CGContext, _ m: SpriteModel) {
        let s = SliceState.shared
        s.canvas = self
        let z = m.zoom
        for i in 0..<m.sliceCount {
            guard let info = m.sliceInfo(i) else { continue }
            let selected = s.selected == i
            let color = m.userDataColor(4, i).map { NSColor(cgColor: $0.cgColor) ?? .systemBlue } ?? .systemBlue
            let r = CGRect(x: CGFloat(info.x) * z, y: CGFloat(info.y) * z, width: CGFloat(info.w) * z, height: CGFloat(info.h) * z)
            ctx.saveGState()
            ctx.setLineWidth(selected ? 2 : 1)
            ctx.setStrokeColor(NSColor.white.withAlphaComponent(0.8).cgColor)
            ctx.stroke(r.insetBy(dx: -1, dy: -1))
            ctx.setStrokeColor(color.cgColor)
            ctx.stroke(r)
            if let c = info.center {
                ctx.setLineDash(phase: 0, lengths: [3, 3])
                ctx.stroke(CGRect(x: r.minX + CGFloat(c.x) * z, y: r.minY + CGFloat(c.y) * z,
                                  width: CGFloat(c.w) * z, height: CGFloat(c.h) * z))
                ctx.setLineDash(phase: 0, lengths: [])
            }
            if let p = info.pivot {
                let pt = CGPoint(x: r.minX + (CGFloat(p.x) + 0.5) * z, y: r.minY + (CGFloat(p.y) + 0.5) * z)
                ctx.move(to: CGPoint(x: pt.x - 5, y: pt.y))
                ctx.addLine(to: CGPoint(x: pt.x + 5, y: pt.y))
                ctx.move(to: CGPoint(x: pt.x, y: pt.y - 5))
                ctx.addLine(to: CGPoint(x: pt.x, y: pt.y + 5))
                ctx.strokePath()
            }
            ctx.restoreGState()

            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 10, weight: .medium),
                .foregroundColor: NSColor.white,
            ]
            let label = NSAttributedString(string: " \(m.sliceName(i)) ", attributes: attrs)
            let size = label.size()
            let box = CGRect(x: r.minX, y: max(0, r.minY - size.height), width: size.width, height: size.height)
            ctx.setFillColor(color.withAlphaComponent(0.85).cgColor)
            ctx.fill(box)
            label.draw(at: box.origin)

            if selected {
                for h in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let x = h.0 < 0 ? r.minX : h.0 > 0 ? r.maxX : r.midX
                    let y = h.1 < 0 ? r.minY : h.1 > 0 ? r.maxY : r.midY
                    let box = CGRect(x: x - 3, y: y - 3, width: 6, height: 6)
                    ctx.setFillColor(NSColor.white.cgColor)
                    ctx.fill(box)
                    ctx.setStrokeColor(NSColor.black.cgColor)
                    ctx.setLineWidth(1)
                    ctx.stroke(box)
                }
            }
        }
    }
}

// MARK: Sheets

/// Text and optional color of an object's user data.
struct UserDataFields: View {
    @Binding var text: String
    @Binding var hasColor: Bool
    @Binding var color: Color

    var body: some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            Text("User data").font(DS.Font.section).foregroundStyle(DSColor.textSecondary)
            TextEditor(text: $text)
                .font(.system(size: 12))
                .scrollContentBackground(.hidden)
                .padding(DS.Space.xs)
                .frame(height: 64)
                .dsFieldBackground()
            HStack(spacing: DS.Space.sm) {
                Toggle("Color", isOn: $hasColor).toggleStyle(.checkbox)
                ColorPicker("", selection: $color).labelsHidden().disabled(!hasColor)
                Spacer(minLength: 0)
            }
        }
    }
}

struct SliceSheet: View {
    @ObservedObject var model: SpriteModel
    let index: Int
    let dismiss: () -> Void
    @State private var name = ""
    @State private var nine = false
    @State private var center = [0, 0, 0, 0]
    @State private var pivotOn = false
    @State private var pivot = [0, 0]
    @State private var text = ""
    @State private var hasColor = false
    @State private var color = Color.blue

    private func intField(_ title: String, _ value: Binding<Int>) -> some View {
        DSNumberField(title, value: value, range: -99999...99999, width: 72)
    }

    var body: some View {
        DSDialog(title: "Slice Properties", primary: "OK",
                 destructive: ("Delete", {
                     SliceState.shared.selected = index
                     model.deleteSelectedSlice()
                     dismiss()
                 }), width: 380,
                 onPrimary: { apply(); dismiss() }, onCancel: { dismiss() }) {
            DSFormRow(label: "Name") { TextField("Name", text: $name).dsTextField() }
            Toggle("9-patch center", isOn: $nine).toggleStyle(.checkbox)
            HStack(spacing: DS.Space.xs + 2) {
                intField("X", $center[0]); intField("Y", $center[1])
                intField("W", $center[2]); intField("H", $center[3])
            }
            .disabled(!nine)
            Toggle("Pivot", isOn: $pivotOn).toggleStyle(.checkbox)
            HStack(spacing: DS.Space.xs + 2) { intField("X", $pivot[0]); intField("Y", $pivot[1]) }
                .disabled(!pivotOn)
            DSDivider()
            UserDataFields(text: $text, hasColor: $hasColor, color: $color)
        }
        .onAppear(perform: load)
    }

    private func load() {
        name = model.sliceName(index)
        if let i = model.sliceInfo(index) {
            nine = i.center != nil
            center = i.center.map { [$0.x, $0.y, $0.w, $0.h] } ?? [0, 0, max(1, i.w), max(1, i.h)]
            pivotOn = i.pivot != nil
            pivot = i.pivot.map { [$0.x, $0.y] } ?? [0, 0]
        }
        text = model.userDataText(4, index)
        let c = model.userDataColor(4, index)
        hasColor = c != nil
        color = SlicesColor.color(c ?? 0x0000FFFF)
    }

    private func apply() {
        let c = center.map(Int32.init), p = pivot.map(Int32.init)
        model.inTransaction {
            name.withCString { n in
                _ = pixmith_slice_set_props(model.doc, UInt32(index), UInt32(model.frame), n, nine ? c : nil, pivotOn ? p : nil)
            }
            let rgba = UInt32(color.cgColor ?? .black)
            _ = pixmith_userdata_set(model.doc, 4, UInt32(index), 0, text, hasColor, rgba)
            model.labelLastStep("Slice Properties")
        }
        model.changed()
    }
}

/// Edits the user data of the sprite (kind 0), current layer (1), current cel (2) or a tag (3).
struct UserDataSheet: View {
    @ObservedObject var model: SpriteModel
    let kind: UInt32
    let dismiss: () -> Void
    @State private var tag = 0
    @State private var text = ""
    @State private var hasColor = false
    @State private var color = Color.black

    private var title: String {
        switch kind {
        case 0: "Sprite User Data"
        case 1: "Layer \u{201C}\(model.layerName(model.layer))\u{201D} User Data"
        case 2: "Cel User Data (frame \(model.frame + 1), \(model.layerName(model.layer)))"
        default: "Tag User Data"
        }
    }

    private var target: (a: Int, b: Int) {
        switch kind {
        case 1: (model.layer, 0)
        case 2: (model.frame, model.layer)
        case 3: (tag, 0)
        default: (0, 0)
        }
    }

    var body: some View {
        DSDialog(title: title, primary: "OK", width: 380, onPrimary: {
            _ = pixmith_userdata_set(model.doc, kind, UInt32(target.a), UInt32(target.b), text, hasColor,
                                  UInt32(color.cgColor ?? .black))
            model.labelLastStep("User Data")
            model.changed()
            dismiss()
        }, onCancel: { dismiss() }) {
            if kind == 3 {
                DSFormRow(label: "Tag") {
                    DSPopup(selection: $tag, options: (0..<model.tagCount).map { ($0, model.tagName($0)) }, fill: true)
                }
                .onChange(of: tag) { _ in load() }
            }
            UserDataFields(text: $text, hasColor: $hasColor, color: $color)
        }
        .onAppear(perform: load)
    }

    private func load() {
        let t = target
        text = model.userDataText(kind, t.a, t.b)
        let c = model.userDataColor(kind, t.a, t.b)
        hasColor = c != nil
        color = SlicesColor.color(c)
    }
}

enum SlicesColor {
    static func color(_ rgba: UInt32?) -> Color { Color(cgColor: (rgba ?? 0x000000FF).cgColor) }
}

struct SlicesMenuItems: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Divider()
        Button("Slice Properties…") { model.showSliceSheet() }.disabled(!model.hasSelectedSlice)
        Button("Delete Slice") { model.deleteSelectedSlice() }.disabled(!model.hasSelectedSlice)
        Divider()
        Button("Sprite User Data…") { model.showUserDataSheet(kind: 0) }
        Button("Layer User Data…") { model.showUserDataSheet(kind: 1) }
        Button("Cel User Data…") { model.showUserDataSheet(kind: 2) }
        Button("Tag User Data…") { model.showUserDataSheet(kind: 3) }.disabled(model.tagCount == 0)
    }
}

extension SpriteModel {
    /// Tab switch or document replacement: slice indices and drags belong to the document being left.
    func resetSliceTransients() {
        let s = SliceState.shared
        s.active = false
        s.began = false
        s.selected = nil
    }
}
