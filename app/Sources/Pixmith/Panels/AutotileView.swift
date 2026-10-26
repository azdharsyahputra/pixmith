import AppKit
import Combine
import PixmithCore
import SwiftUI

/// UI state for the terrain brush (core/src/autotile.rs). The terrain data itself lives in the document.
final class AutotileState: ObservableObject {
    static let shared = AutotileState()

    enum Tool: String, CaseIterable, Identifiable {
        case paint = "Brush", rect = "Rect", fill = "Fill", pick = "Pick", override = "Stamp", auto = "Auto"
        var id: String { rawValue }
        var help: String {
            switch self {
            case .paint: return "Paint terrain. Shift or right click erases."
            case .rect: return "Drag a rectangle of terrain. Shift or right click erases."
            case .fill: return "Flood fill a region with terrain. Shift or right click erases."
            case .pick: return "Pick the terrain under the cursor."
            case .override: return "Place the selected tile by hand (an override that auto-tiling never replaces)."
            case .auto: return "Remove a manual tile so the automatic result returns."
            }
        }
    }

    /// While on, canvas clicks use the terrain tools on the current tilemap layer.
    @Published var brush = false
    @Published var tool: Tool = .paint
    @Published var terrain: UInt32 = 1
    @Published var size = 1
    @Published var round = false
    @Published var showMasks = false
    @Published var newName = "Terrain"
    @Published var newMode: Int32 = 1
    @Published var weightText = "1"
    @Published var seedText = "0"
    var lastCell: (Int, Int)?
    var rectStart: (Int, Int)?
    var rectEnd: (Int, Int)?
    var rectErase = false
    private var bag = Set<AnyCancellable>()

    private init() {
        SpriteModel.shared.$tool.dropFirst().sink { [weak self] _ in self?.brush = false }.store(in: &bag)
        TilemapState.shared.$stamp.sink { [weak self] on in if on { self?.brush = false } }.store(in: &bag)
    }

    static let modeNames = ["Sides (16)", "Blob (47)", "Corners / dual grid (16)"]
    /// Ring direction of each cell of the 3x3 mask grid (row-major, nil = center).
    static let ring: [Int?] = [7, 0, 1, 6, nil, 2, 5, 4, 3]
    static func usesDir(_ mode: Int32, _ dir: Int) -> Bool {
        switch mode {
        case 0: return dir % 2 == 0
        case 2: return dir % 2 == 1
        default: return true
        }
    }
}

extension SpriteModel {
    var terrainMode: Int32 { pixmith_autotile_mode(doc, UInt32(layer)) }

    var terrainIds: [UInt32] {
        (0..<pixmith_autotile_terrain_count(doc, UInt32(layer))).map { pixmith_autotile_terrain_id(doc, UInt32(layer), $0) }
    }

    func terrainName(_ id: UInt32) -> String {
        guard let p = pixmith_autotile_terrain_name(doc, UInt32(layer), id) else { return "?" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    func terrainColor(_ id: UInt32) -> Color {
        let c = pixmith_autotile_terrain_color(doc, UInt32(layer), id)
        return Color(red: Double(c >> 24 & 255) / 255, green: Double(c >> 16 & 255) / 255, blue: Double(c >> 8 & 255) / 255)
    }

    private var mirrorFlags: UInt32 { (opts.mirrorX ? 1 : 0) | (opts.mirrorY ? 2 : 0) }

    /// Handles a canvas mouse event in terrain-brush mode. Left click paints, Shift or right click erases.
    func handleTerrainMouse(_ e: NSEvent, at p: (Int, Int), began: Bool) -> Bool {
        let state = AutotileState.shared
        guard state.brush, isTilemapLayer, terrainMode >= 0 else { return false }
        let erase = e.modifierFlags.contains(.shift) || e.type == .rightMouseDown || e.type == .rightMouseDragged
        let (d, l, f) = (doc, UInt32(layer), UInt32(frame))
        let (px, py) = (Int32(p.0), Int32(p.1))
        let t: UInt32 = erase ? 0 : state.terrain
        switch state.tool {
        case .pick:
            if began {
                let id = pixmith_autotile_terrain_at(d, l, f, px, py)
                if id != 0 { state.terrain = id }
            }
        case .rect:
            if began { state.rectStart = p; state.rectErase = erase }
            state.rectEnd = p
            revision &+= 1
        case .fill:
            if began {
                beginStroke("Terrain Fill")
                if pixmith_autotile_flood(d, l, f, px, py, t, mirrorFlags) { revision &+= 1 }
            }
        case .override:
            if began { beginStroke("Manual Tile") }
            if pixmith_autotile_set_override(d, l, f, px, py, UInt32(TilemapState.shared.value)) { revision &+= 1 }
        case .auto:
            if began { beginStroke("Auto Tile") }
            if pixmith_autotile_set_override(d, l, f, px, py, 0) { revision &+= 1 }
        case .paint:
            if began { beginStroke("Terrain Brush"); state.lastCell = nil }
            var points = [p]
            if let a = state.lastCell, abs(a.0 - p.0) + abs(a.1 - p.1) > 1 {
                // Fill the gap of a fast drag so the stroke has no holes.
                let n = max(abs(a.0 - p.0), abs(a.1 - p.1))
                points = (1...n).map { (a.0 + (p.0 - a.0) * $0 / n, a.1 + (p.1 - a.1) * $0 / n) }
            }
            var changed = false
            for q in points where pixmith_autotile_paint_brush(d, l, f, Int32(q.0), Int32(q.1), t, UInt32(state.size), state.round, mirrorFlags) {
                changed = true
            }
            state.lastCell = p
            if changed { revision &+= 1 }
        }
        return true
    }

    /// Commits a rectangle drag. Returns true if the event was consumed.
    func handleTerrainMouseUp() -> Bool {
        let state = AutotileState.shared
        guard state.brush, state.tool == .rect, let a = state.rectStart, let b = state.rectEnd else { return false }
        state.rectStart = nil
        state.rectEnd = nil
        beginStroke("Terrain Rectangle")
        if pixmith_autotile_rect(doc, UInt32(layer), UInt32(frame), Int32(a.0), Int32(a.1), Int32(b.0), Int32(b.1),
                              state.rectErase ? 0 : state.terrain, mirrorFlags) {
            revision &+= 1
        }
        return true
    }

    /// Draws the rectangle preview and marks cells that use a fallback tile because no tile has their exact mask.
    func drawMissingCells(_ ctx: CGContext) {
        guard isTilemapLayer, let ts = tilesetIndex, terrainMode >= 0, !isIsoLayer else { return }
        let (tw, th) = tileSize(of: ts)
        let state = AutotileState.shared
        if state.brush, state.tool == .rect, let a = state.rectStart, let b = state.rectEnd, tw > 0, th > 0 {
            let x0 = min(a.0, b.0) / tw * tw, x1 = (max(a.0, b.0) / tw + 1) * tw
            let y0 = min(a.1, b.1) / th * th, y1 = (max(a.1, b.1) / th + 1) * th
            ctx.saveGState()
            ctx.setFillColor(NSColor.systemGreen.withAlphaComponent(0.25).cgColor)
            ctx.fill(CGRect(x: CGFloat(x0) * zoom, y: CGFloat(y0) * zoom, width: CGFloat(x1 - x0) * zoom, height: CGFloat(y1 - y0) * zoom))
            ctx.restoreGState()
        }
        let n = pixmith_autotile_missing_cells(doc, UInt32(layer), UInt32(frame), nil, 0)
        guard n > 0 else { return }
        var buf = [Int32](repeating: 0, count: n * 2)
        _ = pixmith_autotile_missing_cells(doc, UInt32(layer), UInt32(frame), &buf, buf.count)
        ctx.saveGState()
        ctx.setStrokeColor(NSColor.systemRed.cgColor)
        ctx.setLineWidth(2)
        for i in 0..<n {
            let r = CGRect(x: CGFloat(buf[i * 2]) * zoom, y: CGFloat(buf[i * 2 + 1]) * zoom, width: CGFloat(tw) * zoom, height: CGFloat(th) * zoom)
            ctx.stroke(r.insetBy(dx: 1, dy: 1))
            ctx.move(to: r.origin)
            ctx.addLine(to: CGPoint(x: r.maxX, y: r.maxY))
            ctx.move(to: CGPoint(x: r.maxX, y: r.minY))
            ctx.addLine(to: CGPoint(x: r.minX, y: r.maxY))
            ctx.strokePath()
        }
        ctx.restoreGState()
    }

    private func done(_ ok: Bool, _ label: String) {
        if ok { labelLastStep(label); revision &+= 1 }
    }

    func createTerrain(name: String, mode: Int32) {
        done(name.withCString { pixmith_autotile_set_terrain(doc, UInt32(layer), $0, 0x5AAA_46FF, mode) }, "Set Terrain")
    }

    func addTerrain() {
        let colors: [UInt32] = [0x8B5A2BFF, 0x3C78D8FF, 0xE6B422FF, 0x9E9E9EFF, 0xC0504DFF]
        let n = Int(pixmith_autotile_terrain_count(doc, UInt32(layer)))
        let id = "Terrain \(n + 1)".withCString { pixmith_autotile_add_terrain(doc, UInt32(layer), $0, colors[n % colors.count]) }
        if id != 0 { AutotileState.shared.terrain = id }
        done(id != 0, "Add Terrain")
    }

    func renameTerrain(_ id: UInt32, _ name: String) {
        let c = pixmith_autotile_terrain_color(doc, UInt32(layer), id)
        done(name.withCString { pixmith_autotile_edit_terrain(doc, UInt32(layer), id, $0, c) }, "Rename Terrain")
    }

    func removeTerrain(_ id: UInt32) {
        done(pixmith_autotile_remove_terrain(doc, UInt32(layer), id), "Remove Terrain")
        if let first = terrainIds.first { AutotileState.shared.terrain = first }
    }

    func moveTerrain(_ id: UInt32, to: Int) {
        done(pixmith_autotile_move_terrain(doc, UInt32(layer), id, UInt32(max(0, to))), "Terrain Priority")
    }

    func setTerrainMode(_ mode: Int32) { done(pixmith_autotile_set_mode(doc, UInt32(layer), mode), "Terrain Mode") }
    func setTileMask(_ tile: Int, _ mask: Int32) { done(pixmith_autotile_set_tile_mask(doc, UInt32(layer), UInt32(tile), mask), "Set Tile Mask") }
    func setTileWeight(_ tile: Int, _ w: Float) { done(pixmith_autotile_set_tile_weight(doc, UInt32(layer), UInt32(tile), w), "Set Tile Weight") }
    func setTilePeer(_ tile: Int, dir: Int, terrain: UInt32) { done(pixmith_autotile_set_tile_peer(doc, UInt32(layer), UInt32(tile), UInt32(dir), terrain), "Set Tile Peer") }
    func setTileTerrain(_ tile: Int, _ terrain: UInt32) { done(pixmith_autotile_set_tile_terrain(doc, UInt32(layer), UInt32(tile), terrain), "Set Tile Terrain") }
    func setTileSym(_ tile: Int, _ sym: UInt32) { done(pixmith_autotile_set_tile_sym(doc, UInt32(layer), UInt32(tile), sym), "Set Tile Symmetry") }
    func generateTemplate(reduced: Bool) { done(pixmith_autotile_generate_template(doc, UInt32(layer), reduced) > 0, "Generate Template") }
    func assignTemplateMasks() { done(pixmith_autotile_assign_masks_from_template(doc, UInt32(layer)) > 0, "Assign Masks") }
    func rebuildTerrain() { done(pixmith_autotile_rebuild(doc, UInt32(layer)), "Rebuild Auto-tiles") }
    func clearOverrides() { done(pixmith_autotile_clear_overrides(doc, UInt32(layer), UInt32(frame)), "Clear Manual Tiles") }
    func setEdgeSame(_ on: Bool) { done(pixmith_autotile_set_edge_same(doc, UInt32(layer), on), "Map Edge Option") }
    func setTerrainSeed(_ s: UInt32) { done(pixmith_autotile_set_seed(doc, UInt32(layer), s), "Variation Seed") }

    func missingReport() -> [String] {
        guard let p = pixmith_autotile_missing_report(doc, UInt32(layer), UInt32(frame)) else { return [] }
        defer { pixmith_string_free(p) }
        return String(cString: p).split(separator: "\n").map(String.init)
    }
}

/// Small mask number shown on a tileset cell.
struct AutotileBadge: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = AutotileState.shared
    let tile: Int

    var body: some View {
        let _ = model.revision
        let mask = state.showMasks && model.terrainMode >= 0 ? pixmith_autotile_tile_mask(model.doc, UInt32(model.layer), UInt32(tile)) : -1
        if mask >= 0 {
            Text("\(mask)").font(.system(size: 8, weight: .bold).monospacedDigit()).padding(.horizontal, 2)
                .background(Color.green.opacity(0.75)).foregroundColor(.white)
                .clipShape(RoundedRectangle(cornerRadius: 2)).padding(1)
        }
    }
}

extension AutotileState.Tool {
    /// Glyph of the tool in the context bar.
    var symbol: String {
        switch self {
        case .paint: return "paintbrush.pointed"
        case .rect: return "square.dashed"
        case .fill: return "drop.fill"
        case .pick: return "eyedropper"
        case .override: return "square.on.square"
        case .auto: return "wand.and.stars"
        }
    }
}

/// A terrain's color as a small rounded swatch.
private struct TerrainSwatch: View {
    let color: Color
    var size: CGFloat = 14
    var body: some View {
        RoundedRectangle(cornerRadius: 3).fill(color)
            .frame(width: size, height: size)
            .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.white.opacity(0.12), lineWidth: 1))
    }
}

/// Context bar options while the terrain brush is on: which terrain, which tool, brush size and the mask overlay.
struct TerrainOptions: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = AutotileState.shared

    var body: some View {
        let _ = model.revision
        HStack(spacing: 6) {
            DSGroupDivider()
            terrainMenu
            DSGroupDivider()
            DSSegmented(selection: $state.tool, options: AutotileState.Tool.allCases.map {
                .init($0, symbol: $0.symbol, help: "\($0.rawValue): \($0.help)")
            })
            if state.tool == .paint {
                DSGroupDivider()
                DSNumberField("Size", value: $state.size, range: 1...16, width: 76, help: "Brush size in cells")
                DSChip(symbol: "circle.fill", help: "Round brush", isOn: $state.round)
            }
            DSGroupDivider()
            DSChip(symbol: "number.square", help: "Show the mask number on each tile", isOn: $state.showMasks)
            if [.paint, .rect, .fill].contains(state.tool) {
                HintLabel(keys: ["⇧"], text: "erase")
            }
        }
    }

    private var terrainMenu: some View {
        let ids = model.terrainIds
        let current = ids.contains(state.terrain) ? state.terrain : (ids.first ?? 0)
        return Menu {
            ForEach(ids, id: \.self) { id in
                Button {
                    state.terrain = id
                } label: {
                    if id == current { Label(model.terrainName(id), systemImage: "checkmark") } else { Text(model.terrainName(id)) }
                }
            }
        } label: {
            HStack(spacing: DS.Space.xs + 2) {
                TerrainSwatch(color: model.terrainColor(current), size: 12)
                Text(model.terrainName(current)).font(DS.Font.body).lineLimit(1)
                Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold)).foregroundStyle(DSColor.textTertiary)
            }
            .padding(.horizontal, DS.Space.sm)
            .frame(height: DS.Metric.control)
            .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Terrain to paint")
    }
}

/// Terrain controls inside the Tileset section for tilemap layers: the terrain list, authoring helpers, the 3x3 mask
/// editor of the selected tile and the report of tiles that are missing. The brush and its tools live in the context bar.
struct AutotilePanel: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = AutotileState.shared
    @ObservedObject private var tiles = TilemapState.shared

    var body: some View {
        let _ = model.revision
        let mode = model.terrainMode
        VStack(alignment: .leading, spacing: DS.Space.md) {
            Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
            header(mode)
            if mode < 0 {
                create()
            } else {
                modePicker(mode)
                terrainList()
                maskEditor(mode: mode, tile: tiles.selected)
                authoring()
                options()
                missing()
            }
        }
        .padding(.top, DS.Space.xs)
    }

    private func header(_ mode: Int32) -> some View {
        HStack(spacing: DS.Space.xs) {
            Text("Terrain").font(DS.Font.section).foregroundStyle(DSColor.text)
            Text("auto-tiling").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
            Spacer(minLength: 0)
            if mode >= 0 {
                DSChip("Brush", symbol: "paintbrush.pointed",
                       help: "Paint terrain on this layer; tiles are chosen automatically. Options appear in the bar above the canvas.",
                       isOn: $state.brush)
            }
        }
    }

    private func create() -> some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            DSNote("Name a terrain and pick how its tiles connect. Painting terrain then picks the right tile for you.")
            if iso { DSNote("Isometric layers connect through the diamond's four sides and four corners; the dual-grid Corners mode is not available.") }
            TextField("Name", text: $state.newName).dsTextField()
            HStack(spacing: DS.Space.sm - 2) {
                DSPopup(selection: Binding(get: { iso && state.newMode == 2 ? 1 : state.newMode }, set: { state.newMode = $0 }),
                        options: modeOptions, help: "How tiles connect", fill: true)
                Button("Create") { model.createTerrain(name: state.newName, mode: iso && state.newMode == 2 ? 1 : state.newMode) }
                    .buttonStyle(DSBarButtonStyle(prominent: true))
            }
        }
    }

    private func modePicker(_ mode: Int32) -> some View {
        DSPopup(selection: Binding(get: { mode }, set: { model.setTerrainMode($0) }),
                options: modeOptions, help: "How tiles connect", fill: true)
    }

    /// Isometric layers have no dual grid, so Corners is not offered there.
    private var iso: Bool { model.isIsoLayer }
    private var modeOptions: [(Int32, String)] {
        (0..<(iso ? 2 : 3)).map { (Int32($0), AutotileState.modeNames[$0]) }
    }

    @ViewBuilder
    private func options() -> some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            HStack(spacing: DS.Space.sm - 2) {
                DSChip("Edges as terrain", help: "Cells beyond the map edge count as this terrain",
                       isOn: Binding(get: { pixmith_autotile_edge_same(model.doc, UInt32(model.layer)) }, set: { model.setEdgeSame($0) }))
                DSChip("Masks", symbol: "number.square", help: "Show each tile's mask number on the tileset", isOn: $state.showMasks)
                Spacer(minLength: 0)
            }
            DSFormRow(label: "Variation seed", labelWidth: 96) {
                TextField("0", text: $state.seedText).dsTextField().frame(width: 64)
                    .onSubmit { if let s = UInt32(state.seedText) { model.setTerrainSeed(s) } }
                    .help("Picks between tiles that share a mask. Press Return to apply.")
                Spacer(minLength: 0)
            }
            HStack(spacing: DS.Space.sm - 2) {
                Button { model.rebuildTerrain() } label: { Text("Rebuild all").frame(maxWidth: .infinity) }
                    .buttonStyle(DSBarButtonStyle()).help("Recompute every tile from the painted terrain")
                Button { model.clearOverrides() } label: { Text("Clear manual tiles").frame(maxWidth: .infinity) }
                    .buttonStyle(DSBarButtonStyle()).help("Remove manual tiles of this frame")
            }
        }
    }

    @ViewBuilder
    private func terrainList() -> some View {
        let ids = model.terrainIds
        VStack(alignment: .leading, spacing: DS.Space.xs) {
            HStack {
                Text("Terrains").font(DS.Font.captionMedium).foregroundStyle(DSColor.textTertiary)
                Spacer(minLength: 0)
                Text("top = highest priority").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
            }
            VStack(spacing: 2) {
                ForEach(Array(ids.enumerated()), id: \.element) { index, id in
                    terrainRow(index: index, id: id, count: ids.count)
                }
            }
            .padding(.horizontal, -DS.Space.sm)
            Button { model.addTerrain() } label: {
                HStack(spacing: 4) {
                    Image(systemName: "plus").font(.system(size: 10, weight: .semibold))
                    Text("Add terrain")
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(DSBarButtonStyle())
        }
    }

    private func terrainRow(index: Int, id: UInt32, count: Int) -> some View {
        let selected = state.terrain == id
        return HStack(spacing: DS.Space.sm) {
            TerrainSwatch(color: model.terrainColor(id))
            Text(model.terrainName(id)).font(DS.Font.body).lineLimit(1)
                .foregroundStyle(selected ? DSColor.text : DSColor.textSecondary)
            Spacer(minLength: 0)
            IconButton("arrow.up", help: "Higher priority", compact: true, size: 22, iconSize: 10) { model.moveTerrain(id, to: index - 1) }
                .disabled(index == 0)
            IconButton("arrow.down", help: "Lower priority", compact: true, size: 22, iconSize: 10) { model.moveTerrain(id, to: index + 1) }
                .disabled(index == count - 1)
            IconButton("minus", help: "Remove terrain", compact: true, size: 22, iconSize: 10, destructive: true) { model.removeTerrain(id) }
                .disabled(count <= 1)
        }
        .padding(.leading, DS.Space.sm)
        .padding(.trailing, DS.Space.xs)
        .frame(height: DS.Metric.control + 4)
        .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(selected ? DSColor.selected : .clear))
        .contentShape(Rectangle())
        .onTapGesture { state.terrain = id }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func authoring() -> some View {
        VStack(alignment: .leading, spacing: DS.Space.xs + 2) {
            Text("Template").font(DS.Font.captionMedium).foregroundStyle(DSColor.textTertiary)
            HStack(spacing: DS.Space.sm - 2) {
                Button { model.generateTemplate(reduced: false) } label: { Text("Full").frame(maxWidth: .infinity) }
                    .buttonStyle(DSBarButtonStyle()).help("Add a labelled empty tile for every mask")
                Button { model.generateTemplate(reduced: true) } label: { Text("Reduced").frame(maxWidth: .infinity) }
                    .buttonStyle(DSBarButtonStyle()).help("Add only the shapes that rotations and flips cannot make")
                Button { model.assignTemplateMasks() } label: { Text("Assign").frame(maxWidth: .infinity) }
                    .buttonStyle(DSBarButtonStyle()).help("Give tiles 1… masks in template order (47 / 16 / reduced layouts)")
            }
        }
    }

    @ViewBuilder
    private func maskEditor(mode: Int32, tile: Int) -> some View {
        let layer = UInt32(model.layer), doc = model.doc
        let mask = pixmith_autotile_tile_mask(doc, layer, UInt32(tile))
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            if tile > 0 {
                HStack(spacing: DS.Space.xs) {
                    Text("Tile \(tile)").font(DS.Font.captionMedium).foregroundStyle(DSColor.text)
                    Text(mask < 0 ? "no mask" : "mask \(mask)").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                }
                HStack(alignment: .top, spacing: DS.Space.md) {
                    VStack(spacing: 3) {
                        ForEach(0..<3, id: \.self) { r in
                            HStack(spacing: 3) {
                                ForEach(0..<3, id: \.self) { c in peerButton(mode: mode, tile: tile, dir: AutotileState.ring[r * 3 + c]) }
                            }
                        }
                    }
                    VStack(alignment: .leading, spacing: DS.Space.sm - 2) {
                        HStack(spacing: DS.Space.sm - 2) {
                            Text("Weight").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                            TextField("1", text: $state.weightText).dsTextField().frame(width: 56)
                                .onSubmit { if let w = Float(state.weightText) { model.setTileWeight(tile, w) } }
                        }
                        .disabled(mask < 0)
                        let sym = pixmith_autotile_tile_sym(doc, layer, UInt32(tile))
                        HStack(spacing: 2) {
                            symToggle("X", 1, sym, tile)
                            symToggle("Y", 2, sym, tile)
                            symToggle("D", 4, sym, tile)
                        }
                        .disabled(mask < 0)
                        .help("Let this tile be reused flipped horizontally / vertically / diagonally")
                        Button("Clear data") { model.setTileMask(tile, -1) }
                            .buttonStyle(DSBarButtonStyle()).disabled(mask < 0)
                    }
                }
                .onChange(of: tile) { _ in
                    state.weightText = String(pixmith_autotile_tile_weight(doc, layer, UInt32(tile)))
                }
                if model.isIsoLayer {
                    DSNote("On diamonds N is the top-right side, E bottom-right, S bottom-left and W top-left; NE, SE, SW and NW are the right, bottom, left and top corners.")
                }
            } else {
                DSNote("Select a tile to edit its terrain edges.")
            }
        }
    }

    private func symToggle(_ title: String, _ bit: UInt32, _ sym: UInt32, _ tile: Int) -> some View {
        DSChip(title, help: "Reusable flipped \(title == "X" ? "horizontally" : title == "Y" ? "vertically" : "diagonally")",
               isOn: Binding(get: { sym & bit != 0 }, set: { model.setTileSym(tile, $0 ? sym | bit : sym & ~bit) }))
    }

    /// One cell of the 3x3 editor: the center sets the tile's own terrain, edges cycle through terrains.
    private func peerButton(mode: Int32, tile: Int, dir: Int?) -> some View {
        let layer = UInt32(model.layer), doc = model.doc
        let ids = [0] + model.terrainIds
        let usable = dir.map { AutotileState.usesDir(mode, $0) } ?? (mode != 2)
        let value: UInt32 = {
            if let d = dir { return UInt32(max(0, pixmith_autotile_tile_peer(doc, layer, UInt32(tile), UInt32(d)))) }
            return UInt32(max(0, pixmith_autotile_tile_terrain(doc, layer, UInt32(tile))))
        }()
        return Button {
            let next = ids[((ids.firstIndex(of: value) ?? 0) + 1) % ids.count]
            if let d = dir { model.setTilePeer(tile, dir: d, terrain: next) } else { model.setTileTerrain(tile, next) }
        } label: {
            RoundedRectangle(cornerRadius: 4)
                .fill(!usable ? Color.clear : (value == 0 ? DSColor.field : model.terrainColor(value)))
                .frame(width: 22, height: 22)
                .overlay(RoundedRectangle(cornerRadius: 4)
                    .stroke(usable ? DSColor.separator : DSColor.separatorSoft, lineWidth: 1))
                .overlay {
                    if dir == nil && usable { Circle().fill(Color.white.opacity(0.85)).frame(width: 4, height: 4) }
                }
        }
        .buttonStyle(.plain)
        .disabled(!usable)
        .help(dir == nil ? "Terrain of this tile. Click to cycle." : "Terrain on this side. Click to cycle.")
    }

    @ViewBuilder
    private func missing() -> some View {
        let lines = model.missingReport()
        if !lines.isEmpty {
            VStack(alignment: .leading, spacing: DS.Space.xs + 2) {
                HStack(spacing: 5) {
                    Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 10)).foregroundStyle(DSColor.destructive)
                    Text("Missing tiles").font(DS.Font.captionMedium).foregroundStyle(DSColor.text)
                    Text("red on canvas").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(lines.enumerated()), id: \.offset) {
                            Text($1).font(DS.Font.mono).foregroundStyle(DSColor.textSecondary).textSelection(.enabled)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(DS.Space.sm)
                }
                .frame(maxHeight: 84)
                .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.recessed))
            }
        }
    }
}
