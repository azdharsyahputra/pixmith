import AppKit
import PixmithCore
import SwiftUI

extension SpriteModel {
    var presetNames: [String] {
        (0..<pixmith_pe_preset_count()).map { i in
            guard let p = pixmith_pe_preset_name(i) else { return "" }
            defer { pixmith_string_free(p) }
            return String(cString: p)
        }
    }

    /// Replaces the palette with a built-in preset (one undo step).
    func loadPreset(_ index: Int) {
        stop()
        guard pixmith_pe_preset_load(doc, UInt32(index)) else {
            errorMessage = String(cString: pixmith_last_error())
            return
        }
        labelLastStep("Load Palette Preset")
        snapDrawColor()
        changed()
    }

    /// Moves palette entries to just before `dest`; returns the first moved entry's new index.
    func movePaletteEntries(_ indices: [Int], to dest: Int) -> Int? {
        let list = indices.map { UInt32($0) }
        let first = pixmith_pe_move(doc, list, UInt32(list.count), UInt32(dest))
        guard first >= 0 else { return nil }
        labelLastStep("Move Palette Colors")
        changed()
        return Int(first)
    }

    func insertPaletteEntry(at index: Int, color: UInt32) -> Bool {
        guard pixmith_pe_insert(doc, UInt32(index), color) else {
            errorMessage = String(cString: pixmith_last_error())
            return false
        }
        labelLastStep("Insert Palette Color")
        changed()
        return true
    }

    func duplicatePaletteEntries(_ indices: [Int]) -> Bool {
        let list = indices.map { UInt32($0) }
        guard pixmith_pe_duplicate(doc, list, UInt32(list.count)) > 0 else { return false }
        labelLastStep("Duplicate Palette Colors")
        changed()
        return true
    }

    func removePaletteEntries(_ indices: [Int]) -> Bool {
        let list = indices.map { UInt32($0) }
        guard pixmith_pe_remove(doc, list, UInt32(list.count)) else { return false }
        labelLastStep("Remove Palette Colors")
        snapDrawColor()
        changed()
        return true
    }

    /// Live edit of a palette entry; `begin` starts a new edit session (one undo step).
    func editPaletteEntry(_ index: Int, to rgba: UInt32, begin: Bool) -> Bool {
        let followsPrimary = index < palette.count && color == palette[index]
        guard pixmith_pe_set_live(doc, UInt32(index), rgba, begin) else { return false }
        if begin { labelLastStep("Edit Palette Color") }
        if followsPrimary { color = rgba }
        changed()
        return true
    }

    /// Inserts `count` colors between two entries; returns the index of the first new color.
    func makeRamp(from: Int, to: Int, count: Int, hsv: Bool) -> Int? {
        let first = pixmith_pe_ramp(doc, UInt32(from), UInt32(to), UInt32(count), hsv)
        guard first >= 0 else {
            errorMessage = String(cString: pixmith_last_error())
            return nil
        }
        labelLastStep("Palette Ramp")
        changed()
        return Int(first)
    }
}

/// "Load Palette Preset" submenu, shared by the Sprite and Palette menus.
struct PalettePresetMenu: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Menu("Load Palette Preset") {
            ForEach(Array(model.presetNames.enumerated()), id: \.offset) { i, name in
                Button(name) { model.loadPreset(i) }
            }
        }
    }
}

/// Palette grid with multi-select, drag to reorder, insert/duplicate/delete, color ramps,
/// presets and live editing of the selected entry.
struct PaletteEditor: View {
    @ObservedObject var model: SpriteModel
    @State private var selection: Set<Int> = []
    @State private var anchor: Int?
    @State private var dragging: [Int] = []
    @State private var dropTarget: Int?
    @State private var session: (index: Int, last: Date, depth: Int)?
    @State private var showRamp = false
    @State private var rampCount = 4
    @State private var rampHSV = false

    private var activeIndex: Int? {
        if selection.count == 1 { return selection.first }
        if let a = anchor, selection.contains(a) { return a }
        return nil
    }

    private func select(_ i: Int, _ palette: [UInt32]) {
        let flags = NSEvent.modifierFlags
        if flags.contains(.command) {
            if selection.contains(i) { selection.remove(i) } else { selection.insert(i) }
            anchor = i
        } else if flags.contains(.shift), let a = anchor {
            selection = Set(min(a, i)...max(a, i))
        } else {
            selection = [i]
            anchor = i
            model.color = palette[i]
        }
    }

    private func drop(onto i: Int) {
        defer { dragging = []; dropTarget = nil }
        guard let lowest = dragging.min() else { return }
        let dest = lowest < i ? i + 1 : i
        if let first = model.movePaletteEntries(dragging, to: dest) {
            selection = Set(first..<first + dragging.count)
            anchor = first
        }
    }

    private func swatch(_ i: Int, _ c: UInt32, transparentIndex: Int?) -> some View {
        let selected = selection.contains(i)
        let isPrimary = c == model.color
        return ZStack {
            DSChecker(cell: 4)
            Color(rgba: c)
            if i == transparentIndex {
                Image(systemName: "circle.slash").font(.system(size: 9)).foregroundStyle(.secondary)
            }
        }
        .aspectRatio(1, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 3))
        .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.white.opacity(isPrimary && !selected ? 0.7 : 0.08),
                                                         lineWidth: isPrimary && !selected ? 1.5 : 1))
        .overlay {
            if selected { RoundedRectangle(cornerRadius: 4).stroke(DSColor.accent, lineWidth: 2).padding(-2) }
        }
        .overlay(alignment: .leading) {
            if dropTarget == i { Rectangle().fill(DSColor.accent).frame(width: 3).padding(.leading, -3) }
        }
        .contentShape(Rectangle())
        .onTapGesture { select(i, model.palette) }
        .onDrag {
            if !selection.contains(i) { selection = [i]; anchor = i }
            dragging = selection.sorted()
            return NSItemProvider(object: "pixmith-palette-entry" as NSString)
        }
        .onDrop(of: [.plainText], delegate: SwatchDrop(index: i, target: $dropTarget, perform: drop(onto:)))
        .contextMenu { model.paletteEntryMenu(i, c) }
        .help("#\(i)")
        .accessibilityLabel("Palette color \(i)")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    var body: some View {
        let palette = model.palette
        let ti: Int? = model.colorMode == .indexed ? model.transparentIndex : nil
        InspectorSection("Palette", id: "palette", subtitle: "\(palette.count)") {
            IconButton("plus", help: "Insert the current color after the selection", compact: true) { insertEntry(palette.count) }
            DSMenuButton(help: "Palette actions: duplicate, delete, ramp, presets, load, save, sort") {
                Button("Add Primary Color to Palette") { model.addColorToPalette() }
                Button("Duplicate Selected") { duplicate() }.disabled(selection.isEmpty)
                Button("Delete Selected") { remove() }.disabled(selection.isEmpty || selection.count >= palette.count)
                Button("Create Color Ramp…") { showRamp = true }.disabled(selection.count < 2)
                Divider()
                Menu("Load Preset") {
                    ForEach(Array(model.presetNames.enumerated()), id: \.offset) { i, name in
                        Button(name) { model.loadPreset(i) }
                    }
                }
                Button("Load Palette…") { model.loadPalette() }
                Button("Save Palette…") { model.savePalette() }
                Divider()
                Button("Sort by Hue") { model.sortPalette(byHue: true) }
                Button("Sort by Luminance") { model.sortPalette(byHue: false) }
            }
            .popover(isPresented: $showRamp) { rampSheet }
        } content: {
            VStack(alignment: .leading, spacing: DS.Space.sm) {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 3), count: 8), spacing: 3) {
                    ForEach(Array(palette.enumerated()), id: \.offset) { i, c in
                        swatch(i, c, transparentIndex: ti)
                    }
                }
                .padding(2)
                if let i = activeIndex, i < palette.count {
                    HStack(spacing: DS.Space.sm) {
                        Text("Entry #\(i)").font(DS.Font.caption).foregroundStyle(DSColor.textSecondary)
                        Spacer()
                        ColorPicker("Selected palette entry", selection: Binding(
                            get: { Color(cgColor: model.palette[min(i, model.palette.count - 1)].cgColor) },
                            set: { edit(i, $0) }))
                            .labelsHidden().help("Edit the selected entry; pixels using it update live")
                    }
                } else if selection.count > 1 {
                    Text("\(selection.count) selected").font(DS.Font.caption).foregroundStyle(DSColor.textSecondary)
                }
                if model.colorMode == .indexed {
                    Text("Indexed · transparent #\(model.transparentIndex)").font(DS.Font.caption)
                        .foregroundStyle(DSColor.textTertiary)
                }
            }
        }
        .onChange(of: model.revision) { _ in
            let n = model.palette.count
            if selection.contains(where: { $0 >= n }) { selection = selection.filter { $0 < n } }
        }
        .onChange(of: selection) { _ in session = nil }
    }

    private var rampSheet: some View {
        VStack(alignment: .leading, spacing: DS.Space.md) {
            Text("Color Ramp").font(DS.Font.title)
            DSNumberField("Colors", value: $rampCount, range: 1...64, width: 120)
            DSSegmented(selection: $rampHSV, options: [.init(false, title: "RGB"), .init(true, title: "HSV")])
            HStack {
                Spacer()
                Button("Create") {
                    if let a = selection.min(), let b = selection.max(),
                       let first = model.makeRamp(from: a, to: b, count: rampCount, hsv: rampHSV) {
                        selection = Set(first..<first + rampCount)
                        anchor = first
                    }
                    showRamp = false
                }
                .buttonStyle(DSPrimaryButtonStyle())
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(DS.Space.lg).frame(width: 240)
    }

    private func insertEntry(_ end: Int) {
        let at = selection.max().map { $0 + 1 } ?? end
        if model.insertPaletteEntry(at: at, color: model.color) {
            selection = [at]
            anchor = at
        }
    }

    private func duplicate() {
        let sorted = selection.sorted()
        if model.duplicatePaletteEntries(sorted) {
            selection = Set(sorted.enumerated().map { $0.element + $0.offset + 1 })
            anchor = selection.min()
        }
    }

    private func remove() {
        if model.removePaletteEntries(selection.sorted()) {
            selection = []
            anchor = nil
        }
    }

    /// One undo step per edit session: a session continues while the same entry keeps changing.
    private func edit(_ index: Int, _ color: Color) {
        let rgba = UInt32(color.cgColor ?? .black)
        // An undo, redo or any other edit in between ends the session.
        let depth = model.historyCounts.undo
        let begin = session.map { $0.index != index || $0.depth != depth || Date().timeIntervalSince($0.last) > 1.5 } ?? true
        if model.editPaletteEntry(index, to: rgba, begin: begin) {
            session = (index, Date(), model.historyCounts.undo)
        }
    }
}

private struct SwatchDrop: DropDelegate {
    let index: Int
    @Binding var target: Int?
    let perform: (Int) -> Void

    func validateDrop(info: DropInfo) -> Bool { true }
    func dropEntered(info: DropInfo) { target = index }
    func dropExited(info: DropInfo) { if target == index { target = nil } }
    func dropUpdated(info: DropInfo) -> DropProposal? { DropProposal(operation: .move) }
    func performDrop(info: DropInfo) -> Bool {
        perform(index)
        return true
    }
}
