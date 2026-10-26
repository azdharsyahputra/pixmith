import AppKit
import PixmithCore
import SwiftUI

/// Which cels of the timeline grid are selected. With no range only the current cel (layer + frame) is.
final class TimelineSelection: ObservableObject {
    static let shared = TimelineSelection()

    struct Rect: Equatable {
        var l0: Int, l1: Int, f0: Int, f1: Int

        func contains(layer: Int, frame: Int) -> Bool { (l0...l1).contains(layer) && (f0...f1).contains(frame) }
        func shifted(layers dl: Int, frames df: Int) -> Rect { Rect(l0: l0 + dl, l1: l1 + dl, f0: f0 + df, f1: f1 + df) }
    }

    @Published var range: Rect?
    var anchor: (layer: Int, frame: Int)?
    /// True after a click in the grid: copy, paste and delete then act on cels instead of pixels.
    var focused = false
}

extension SpriteModel {
    /// The selected cels, clamped to the sprite.
    var selectedCels: TimelineSelection.Rect {
        let one = TimelineSelection.Rect(l0: layer, l1: layer, f0: frame, f1: frame)
        guard var r = TimelineSelection.shared.range else { return one }
        r.l1 = min(r.l1, layerCount - 1)
        r.f1 = min(r.f1, frameCount - 1)
        return r.l0 <= r.l1 && r.f0 <= r.f1 ? r : one
    }

    func celGrid() -> [UInt8] {
        var out = [UInt8](repeating: 0, count: layerCount * frameCount)
        return pixmith_cel_grid(doc, &out, out.count) ? out : []
    }

    func copyCels(cut: Bool) {
        let r = selectedCels
        guard pixmith_cels_copy(doc, UInt32(r.l0), UInt32(r.l1), UInt32(r.f0), UInt32(r.f1)) else { return }
        if cut { clearSelectedCels() }
    }

    /// Pastes the cel clipboard with its lower-left cel at the lowest selected layer and first selected frame.
    func pasteCels() {
        stop()
        let r = selectedCels
        if pixmith_cels_paste(doc, UInt32(r.l0), UInt32(r.f0)) { labelLastStep("Paste Cels"); changed() } else { NSSound.beep() }
    }

    func clearSelectedCels() {
        stop()
        let r = selectedCels
        if pixmith_cels_clear(doc, UInt32(r.l0), UInt32(r.l1), UInt32(r.f0), UInt32(r.f1)) { labelLastStep("Clear Cels"); changed() }
    }

    func moveCels(_ r: TimelineSelection.Rect, layers dl: Int, frames df: Int, copy: Bool) -> Bool {
        stop()
        guard pixmith_cels_move(doc, UInt32(r.l0), UInt32(r.l1), UInt32(r.f0), UInt32(r.f1), Int32(dl), Int32(df), copy)
        else { return false }
        labelLastStep(copy ? "Copy Cels" : "Move Cels")
        changed()
        return true
    }
}

/// The cel grid's canvas as an `Equatable` view: SwiftUI skips `body` (and the redraw) while the input is unchanged.
private struct TimelineCanvas: View, Equatable {
    struct Input: Equatable {
        /// Layer index shown on each row, top to bottom (collapsed groups hide their children).
        var rows: [Int]
        var nf: Int
        var cells: [UInt8]
        var frame: Int, layer: Int
        var selected: TimelineSelection.Rect
        /// Per layer: bit 0 group, bit 1 visible.
        var layerFlags: [Int]
        var dragRect: TimelineSelection.Rect?
        var viewport: CGSize
        var dark: Bool
    }

    let input: Input
    private let rowHeight = DS.Metric.timelineRow
    private let pitch = Timeline.cellWidth + Timeline.gap

    static func size(_ input: Input) -> CGSize {
        let pitch = Timeline.cellWidth + Timeline.gap
        return CGSize(width: max(CGFloat(input.nf + 1) * pitch, input.viewport.width),
                      height: max(CGFloat(input.rows.count) * DS.Metric.timelineRow, input.viewport.height))
    }

    var body: some View {
        let size = Self.size(input)
        Canvas { ctx, _ in draw(ctx, size) }
            .frame(width: size.width, height: size.height)
    }

    private func row(of layer: Int) -> Int? { input.rows.firstIndex(of: layer) }

    private func rect(layer: Int, frame: Int) -> CGRect? {
        row(of: layer).map { CGRect(x: CGFloat(frame) * pitch, y: CGFloat($0) * rowHeight, width: pitch, height: rowHeight) }
    }

    private func draw(_ ctx: GraphicsContext, _ size: CGSize) {
        let (nf, cells, selected) = (input.nf, input.cells, input.selected)
        let dark = input.dark
        let guide = dark ? Color.white.opacity(0.035) : Color.black.opacity(0.05)
        let dot = dark ? Color(white: 0.64) : Color(white: 0.40)
        let faint = dark ? Color(white: 0.46) : Color(white: 0.60)
        let accent = DSColor.accent

        // Selected layer row, current frame column.
        if let r = row(of: input.layer) {
            ctx.fill(Path(CGRect(x: 0, y: CGFloat(r) * rowHeight, width: size.width, height: rowHeight)),
                     with: .color(accent.opacity(0.18)))
        }
        ctx.fill(Path(CGRect(x: CGFloat(input.frame) * pitch, y: 0, width: pitch, height: size.height)),
                 with: .color(dark ? Color.white.opacity(0.04) : Color.black.opacity(0.04)))
        // Faint frame-column guides all the way to the right and bottom edge.
        var lines = Path()
        var x = pitch - 0.5
        while x < size.width {
            lines.move(to: CGPoint(x: x, y: 0))
            lines.addLine(to: CGPoint(x: x, y: size.height))
            x += pitch
        }
        ctx.stroke(lines, with: .color(guide), lineWidth: 1)

        let marks = cells.count == input.layerFlags.count * nf
        for (r, l) in input.rows.enumerated() {
            let group = input.layerFlags[l] & 1 != 0
            let dim = input.layerFlags[l] & 2 != 0 ? 1.0 : 0.4
            for f in 0..<nf {
                let cell = CGRect(x: CGFloat(f) * pitch, y: CGFloat(r) * rowHeight, width: pitch, height: rowHeight)
                if selected.contains(layer: l, frame: f), selected.l0 != selected.l1 || selected.f0 != selected.f1 {
                    ctx.fill(Path(cell), with: .color(accent.opacity(0.30)))
                }
                guard marks, !group else { continue }
                let bits = cells[l * nf + f]
                if bits & 1 == 0 { continue }
                let current = l == input.layer && f == input.frame
                let c = cell.midX, m = cell.midY
                if bits & 4 != 0 {
                    // Linked cel: a short bar, longer on the current frame.
                    let w: CGFloat = f == input.frame ? 14 : 4
                    ctx.fill(Path(roundedRect: CGRect(x: c - w / 2, y: m - 2, width: w, height: 4), cornerRadius: 2),
                             with: .color((current ? accent : faint).opacity(dim)))
                } else if bits & 2 != 0 {
                    ctx.fill(Path(ellipseIn: CGRect(x: c - 5, y: m - 5, width: 10, height: 10)),
                             with: .color((current ? accent : dot).opacity(dim)))
                } else {
                    ctx.stroke(Path(ellipseIn: CGRect(x: c - 4.25, y: m - 4.25, width: 8.5, height: 8.5)),
                               with: .color((current ? accent : faint).opacity(dim)), lineWidth: 1.5)
                }
            }
        }
        if let cur = rect(layer: input.layer, frame: input.frame) {
            ctx.stroke(Path(roundedRect: cur.insetBy(dx: 2, dy: 2), cornerRadius: 4), with: .color(accent.opacity(0.55)), lineWidth: 1)
        }
        if let moved = input.dragRect,
           let a = rect(layer: moved.l0, frame: moved.f0) ?? rect(layer: moved.l1, frame: moved.f0),
           let b = rect(layer: moved.l1, frame: moved.f1) ?? rect(layer: moved.l0, frame: moved.f1) {
            ctx.stroke(Path(roundedRect: a.union(b).insetBy(dx: 1, dy: 1), cornerRadius: 4), with: .color(accent),
                       style: StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
        }
    }
}

private struct ScrollOffsetKey: PreferenceKey {
    static var defaultValue = CGPoint.zero
    static func reduce(value: inout CGPoint, nextValue: () -> CGPoint) { value = nextValue() }
}

/// Persisted collapsed state of the timeline panel.
private let timelineCollapsedKey = "panel.timeline.collapsed"

/// The timeline panel: transport and frame controls on top, then the layers column next to the frames grid.
struct TimelinePanel: View {
    @ObservedObject var model: SpriteModel
    @AppStorage(timelineCollapsedKey) private var collapsed = false

    var body: some View {
        let rows = CGFloat(model.layerCount)
        let expanded = min(DS.Metric.timelineHeight, max(168, DS.Metric.timelineHeader + 48 + rows * DS.Metric.timelineRow + 8))
        VStack(spacing: 0) {
            TimelineHeader(model: model, collapsed: $collapsed)
            if !collapsed {
                Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
                TimelineGrid(model: model)
            }
        }
        .frame(height: collapsed ? DS.Metric.timelineHeader : expanded)
        .background(DSColor.panel)
        .overlay(alignment: .top) { Rectangle().fill(DSColor.separator).frame(height: 1) }
        .animation(DS.Anim.quick, value: collapsed)
    }
}

/// Play, step, frame and duration fields, loop / onion toggles and the frame actions.
struct TimelineHeader: View {
    @ObservedObject var model: SpriteModel
    @Binding var collapsed: Bool

    var body: some View {
        HStack(spacing: DS.Space.xxs) {
            IconButton("backward.frame.fill", help: "Previous frame", shortcut: Shortcuts.shared.label(for: .previousFrame)) {
                model.stop(); model.step(-1)
            }
            IconButton(model.isPlaying ? "pause.fill" : "play.fill", help: model.isPlaying ? "Pause" : "Play",
                       shortcut: Shortcuts.shared.label(for: .playPause), selected: model.isPlaying) { model.togglePlay() }
            IconButton("forward.frame.fill", help: "Next frame", shortcut: Shortcuts.shared.label(for: .nextFrame)) {
                model.stop(); model.step(1)
            }
            DSNumberField("Frame", value: Binding(get: { model.frame + 1 }, set: { model.stop(); model.frame = min(max($0 - 1, 0), model.frameCount - 1) }),
                          range: 1...max(1, model.frameCount), unit: "/ \(model.frameCount)", width: 112,
                          help: "Current frame. Drag the label to scrub.")
                .padding(.leading, DS.Space.xs)
            DSNumberField("Time", value: Binding(get: { model.duration(model.frame) }, set: { model.setDuration($0) }),
                          range: 10...10000, unit: "ms", width: 96, help: "Duration of this frame. Drag the label to scrub.",
                          deferScrub: true)
                .padding(.leading, DS.Space.xs)
            DSGroupDivider()
            DSChip(symbol: "repeat", help: "Loop tag: play inside the tag that contains the current frame", isOn: $model.loopTags)
            DSChip(symbol: "square.3.layers.3d", help: "Onion skin (options in View > View Options)", isOn: $model.onionSkin)
            DSChip(symbol: "plusminus.circle", help: "Frame difference: tint the pixels that differ from the previous frame (options in View > View Options)",
                   isOn: $model.frameDiff)
            DSChip(symbol: "grid", help: "Pixel grid", isOn: $model.showGrid)
            Spacer(minLength: DS.Space.sm)
            IconButton("plus.square", help: "New frame", shortcut: "⌥⌘N") { model.addFrame(duplicate: false) }
            IconButton("square.on.square", help: "Duplicate frame", shortcut: "⇧⌘D") { model.addFrame(duplicate: true) }
            IconButton("arrow.left.to.line", help: "Move frame left") { model.moveFrame(by: -1) }
                .disabled(model.frame == 0)
            IconButton("arrow.right.to.line", help: "Move frame right") { model.moveFrame(by: 1) }
                .disabled(model.frame >= model.frameCount - 1)
            IconButton("tag", help: "New tag from the current frame") { model.toolSheet = .tag(nil) }
            IconButton("trash", help: "Delete frame", destructive: true) { model.removeFrame() }
                .disabled(model.frameCount <= 1)
            DSGroupDivider()
            IconButton(collapsed ? "chevron.up" : "chevron.down", help: collapsed ? "Show timeline" : "Hide timeline") {
                collapsed.toggle()
            }
        }
        .padding(.horizontal, DS.Space.sm)
        .frame(height: DS.Metric.timelineHeader)
    }
}

/// Layers × frames: the layers column on the left, the frame numbers, tag bars and cel grid on the right. Click selects
/// a cel, shift-click a range, dragging moves the selection (option copies).
struct TimelineGrid: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var sel = TimelineSelection.shared
    @Environment(\.colorScheme) private var scheme
    @State private var cells: [UInt8] = []
    @State private var scroll = CGPoint.zero
    @State private var collapsedGroups: Set<Int> = []
    @State private var dragStart: (layer: Int, frame: Int)?
    @State private var dragRect = TimelineSelection.Rect(l0: 0, l1: 0, f0: 0, f1: 0)
    @State private var dragDelta = (layers: 0, frames: 0)
    /// Set between `begin` and the end of the gesture so a shift-click (which starts no drag) begins only once.
    @State private var pressed = false

    private let nameWidth: CGFloat = 200
    private let rowHeight = DS.Metric.timelineRow
    private let pitch = Timeline.cellWidth + Timeline.gap

    /// Layer indices shown on the rows, top to bottom. Children of a collapsed group are hidden.
    private func visibleRows() -> [Int] {
        var hidden = Set<Int>()
        for g in collapsedGroups where g < model.layerCount && model.isGroup(g) {
            var j = g + 1
            while j < model.layerCount, model.level(j) > model.level(g) { hidden.insert(j); j += 1 }
        }
        return (0..<model.layerCount).reversed().filter { !hidden.contains($0) }
    }

    var body: some View {
        let nl = model.layerCount, nf = model.frameCount
        let rows = visibleRows()
        let tags = TagLayout(model: model)
        let headerHeight = tags.height + Timeline.numberRow
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                layersHeader.frame(height: headerHeight)
                names(rows)
            }
            .frame(width: nameWidth)
            .overlay(alignment: .trailing) { Rectangle().fill(DSColor.separatorSoft).frame(width: 1) }
            VStack(spacing: 0) {
                frameHeader(nf, height: headerHeight)
                grid(rows, nl, nf)
            }
        }
        .task(id: model.revision) {
            if cells.count != nl * nf {
                cells = model.celGrid()
            } else {
                try? await Task.sleep(nanoseconds: 80_000_000)
                if !Task.isCancelled { cells = model.celGrid() }
            }
        }
    }

    // MARK: Layers column

    private var layersHeader: some View {
        HStack(spacing: 0) {
            Text("Layers").font(DS.Font.section).foregroundStyle(DSColor.textSecondary)
            Spacer(minLength: 0)
            IconButton("plus", help: "New layer", shortcut: "⇧⌘N", compact: true) { model.addLayer() }
            IconButton("folder.badge.plus", help: "New group", compact: true) { model.addGroup() }
            DSMenuButton(help: "Layer actions: move, duplicate, merge, group, properties, tilemap") {
                Button("Delete Layer") { model.removeLayer() }.disabled(model.layerCount <= 1)
                LayerMenuItems(model: model, index: model.layer)
                Divider()
                TilemapMenuItems(model: model)
            }
        }
        .padding(.leading, 12)
        .padding(.trailing, DS.Space.sm)
        .frame(maxHeight: .infinity, alignment: .bottom)
        .padding(.bottom, 4)
    }

    private func names(_ rows: [Int]) -> some View {
        VStack(spacing: 0) {
            ForEach(rows, id: \.self) { i in
                TimelineLayerRow(model: model, index: i, collapsed: collapsedGroups.contains(i)) {
                    withAnimation(DS.Anim.quick) {
                        if collapsedGroups.contains(i) { collapsedGroups.remove(i) } else { collapsedGroups.insert(i) }
                    }
                }
                .frame(height: rowHeight)
            }
        }
        .offset(y: -scroll.y)
        .frame(width: nameWidth, alignment: .top)
        .frame(minHeight: 0, maxHeight: .infinity, alignment: .top)
        .clipped()
    }

    // MARK: Frames

    private func frameHeader(_ nf: Int, height: CGFloat) -> some View {
        ZStack(alignment: .topLeading) {
            Canvas { ctx, size in
                var lines = Path()
                var x = pitch - 0.5 - scroll.x.truncatingRemainder(dividingBy: pitch)
                while x < size.width {
                    lines.move(to: CGPoint(x: x, y: 0))
                    lines.addLine(to: CGPoint(x: x, y: size.height))
                    x += pitch
                }
                ctx.stroke(lines, with: .color(scheme == .dark ? Color.white.opacity(0.035) : Color.black.opacity(0.05)), lineWidth: 1)
            }
            VStack(alignment: .leading, spacing: 0) {
                TagsBar(model: model)
                HStack(spacing: Timeline.gap) {
                    ForEach(0..<nf, id: \.self) { f in
                        Text("\(f + 1)")
                            .font(.system(size: 11, weight: f == model.frame ? .semibold : .regular).monospacedDigit())
                            .foregroundStyle(f == model.frame ? DSColor.text : DSColor.textTertiary)
                            .frame(width: Timeline.cellWidth, height: Timeline.numberRow)
                            .contentShape(Rectangle())
                            .onTapGesture { model.stop(); model.frame = f; sel.range = nil; sel.focused = true }
                            .accessibilityLabel("Frame \(f + 1)")
                    }
                    Button { model.addFrame(duplicate: false) } label: {
                        Image(systemName: "plus").font(.system(size: 10, weight: .medium))
                            .frame(width: Timeline.cellWidth, height: Timeline.numberRow)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(DSIconButtonStyle(radius: 4))
                    .help("New frame")
                }
            }
            .offset(x: -scroll.x)
        }
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .topLeading)
        .frame(height: height)
        .clipped()
    }

    private func grid(_ rows: [Int], _ nl: Int, _ nf: Int) -> some View {
        GeometryReader { vp in
            ScrollView([.horizontal, .vertical]) {
                ZStack(alignment: .topLeading) {
                    TimelineCanvas(input: canvasInput(rows, nl, nf, vp.size)).equatable()
                        .gesture(
                            DragGesture(minimumDistance: 0)
                                .onChanged { v in dragChanged(v, rows, nl, nf) }
                                .onEnded { v in dragEnded(v, rows, nl, nf) }
                        )
                        .background(GeometryReader { g in
                            Color.clear.preference(key: ScrollOffsetKey.self, value: g.frame(in: .named("timeline")).origin)
                        })
                }
                .frame(minWidth: vp.size.width, minHeight: vp.size.height, alignment: .topLeading)
            }
            .coordinateSpace(name: "timeline")
            .onPreferenceChange(ScrollOffsetKey.self) { scroll = CGPoint(x: -$0.x, y: -$0.y) }
        }
    }

    /// Everything the cel canvas draws from. The canvas only redraws when this value changes, not on every
    /// model update (each stroke segment publishes one).
    private func canvasInput(_ rows: [Int], _ nl: Int, _ nf: Int, _ viewport: CGSize) -> TimelineCanvas.Input {
        TimelineCanvas.Input(
            rows: rows, nf: nf, cells: cells, frame: model.frame, layer: model.layer, selected: model.selectedCels,
            layerFlags: (0..<nl).map { (model.isGroup($0) ? 1 : 0) | (model.isVisible($0) ? 2 : 0) },
            dragRect: dragStart != nil && dragDelta != (0, 0)
                ? dragRect.shifted(layers: dragDelta.layers, frames: dragDelta.frames) : nil,
            viewport: viewport, dark: scheme == .dark)
    }

    // MARK: Mouse

    private func cell(at p: CGPoint, _ rows: [Int], _ nf: Int) -> (layer: Int, frame: Int)? {
        guard !rows.isEmpty else { return nil }
        let f = min(max(Int(p.x / pitch), 0), nf - 1)
        let row = min(max(Int(p.y / rowHeight), 0), rows.count - 1)
        return (rows[row], f)
    }

    private func dragChanged(_ v: DragGesture.Value, _ rows: [Int], _ nl: Int, _ nf: Int) {
        if !pressed {
            guard let c = cell(at: v.startLocation, rows, nf) else { return }
            begin(c, nl, nf)
        }
        guard let start = dragStart, let c = cell(at: v.location, rows, nf) else { return }
        dragDelta = (
            min(max(c.layer - start.layer, -dragRect.l0), nl - 1 - dragRect.l1),
            min(max(c.frame - start.frame, -dragRect.f0), nf - 1 - dragRect.f1)
        )
    }

    private func begin(_ c: (layer: Int, frame: Int), _ nl: Int, _ nf: Int) {
        pressed = true
        sel.focused = true
        model.stop()
        if NSEvent.modifierFlags.contains(.shift) {
            let a = sel.anchor ?? (model.layer, model.frame)
            sel.range = .init(l0: min(a.layer, c.layer), l1: max(a.layer, c.layer), f0: min(a.frame, c.frame), f1: max(a.frame, c.frame))
            model.frame = c.frame
            if !model.isGroup(c.layer) { model.layer = c.layer }
            return
        }
        let current = model.selectedCels
        if !current.contains(layer: c.layer, frame: c.frame) || model.isGroup(c.layer) {
            sel.range = nil
            if !model.isGroup(c.layer) { model.layer = c.layer }
            model.frame = c.frame
            sel.anchor = c
        }
        guard !model.isGroup(c.layer) else { return }
        dragStart = c
        dragRect = model.selectedCels
        dragDelta = (0, 0)
    }

    private func dragEnded(_ v: DragGesture.Value, _ rows: [Int], _ nl: Int, _ nf: Int) {
        defer {
            dragStart = nil
            dragDelta = (0, 0)
            pressed = false
        }
        guard let start = dragStart else { return }
        let (dl, df) = dragDelta
        if (dl, df) == (0, 0) {
            // A plain click inside a multi-cel selection narrows it to the clicked cel.
            if dragRect.l0 != dragRect.l1 || dragRect.f0 != dragRect.f1 {
                sel.range = nil
                model.layer = start.layer
                model.frame = start.frame
                sel.anchor = start
            }
            return
        }
        let copy = NSEvent.modifierFlags.contains(.option)
        guard model.moveCels(dragRect, layers: dl, frames: df, copy: copy) else { return }
        let moved = dragRect.shifted(layers: dl, frames: df)
        let multi = dragRect.l0 != dragRect.l1 || dragRect.f0 != dragRect.f1
        sel.range = multi ? moved : nil
        sel.anchor = (start.layer + dl, start.frame + df)
        if !model.isGroup(start.layer + dl) { model.layer = start.layer + dl }
        model.frame = start.frame + df
    }
}

/// One row of the layers column: group disclosure, thumbnail, name and the visibility eye. Drag to reorder, double-click
/// to rename, right-click for the layer menu.
private struct TimelineLayerRow: View {
    @ObservedObject var model: SpriteModel
    let index: Int
    let collapsed: Bool
    let toggleGroup: () -> Void
    @State private var hover = false

    var body: some View {
        let group = model.isGroup(index)
        let visible = model.isVisible(index)
        let selected = index == model.layer
        HStack(spacing: DS.Space.sm) {
            if group {
                Button(action: toggleGroup) {
                    Image(systemName: "chevron.down").font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(collapsed ? -90 : 0))
                        .foregroundStyle(DSColor.textTertiary)
                        .frame(width: 14, height: DS.Metric.control)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(collapsed ? "Expand group" : "Collapse group")
                Image(systemName: "folder").font(.system(size: 13)).foregroundStyle(DSColor.textSecondary)
                    .frame(width: 16)
            } else {
                ZStack {
                    DSChecker(cell: 4)
                    if pixmith_layer_is_tilemap(model.doc, UInt32(index)) {
                        Image(systemName: "square.grid.3x3").font(.system(size: 9)).foregroundStyle(DSColor.textSecondary)
                    }
                    if model.isReference(index) {
                        Image(systemName: "photo").font(.system(size: 10, weight: .semibold)).foregroundStyle(DSColor.accent)
                    }
                }
                .frame(width: 16, height: 16)
                .clipShape(RoundedRectangle(cornerRadius: 3))
                .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.white.opacity(0.08), lineWidth: 1))
                .opacity(visible ? 1 : 0.4)
            }
            Text(model.layerName(index))
                .font(DS.Font.body.weight(selected ? .medium : .regular))
                .foregroundStyle(selected ? DSColor.text : DSColor.textSecondary)
                .opacity(visible ? 1 : 0.5)
                .lineLimit(1)
            Spacer(minLength: 0)
            if !group, model.isReference(index) {
                Button { model.setLocked(index, !model.isLocked(index)) } label: {
                    Image(systemName: model.isLocked(index) ? "lock.fill" : "lock.open").font(.system(size: 10))
                        .foregroundStyle(DSColor.textTertiary)
                        .frame(width: DS.Metric.control, height: DS.Metric.control)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(model.isLocked(index) ? "Reference layer is locked: click to allow drawing" : "Lock the reference layer")
            }
            Button { model.setVisible(index, !visible) } label: {
                Image(systemName: visible ? "eye" : "eye.slash").font(.system(size: 12))
                    .foregroundStyle(visible ? DSColor.textTertiary : DSColor.textSecondary)
                    .frame(width: DS.Metric.control, height: DS.Metric.control)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(visible ? "Hide layer" : "Show layer")
        }
        .padding(.leading, 8 + CGFloat(model.level(index)) * 14)
        .padding(.trailing, DS.Space.xs)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .background(selected ? DSColor.selected : hover ? DSColor.hover.opacity(0.6) : Color.clear)
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture(count: 2) { model.renamingLayer = index }
        .onTapGesture { TimelineSelection.shared.focused = false; if !group { model.layer = index; TimelineSelection.shared.range = nil } }
        .layerReorder(model: model, index: index)
        .contextMenu {
            Button("Rename…") { model.renamingLayer = index }
            Button("Delete") { model.layer = index; model.removeLayer() }
            LayerMenuItems(model: model, index: index)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(model.layerName(index))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
