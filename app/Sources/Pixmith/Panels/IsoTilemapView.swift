import AppKit
import PixmithCore
import SwiftUI

// Isometric tilemap layers (core/src/document/iso.rs, isoext.rs): a layer whose tileset puts its tiles on 2:1 diamond
// cells (Godot DIAMOND_DOWN). Cell (i, j) has its box at (x + (i - j) * w / 2, y + (i + j) * h / 2), where (x, y) is
// the layer's grid offset. The layer's tile grid is addressed by cell, so the canvas code here converts pixels to cells
// with the core and never does the diamond math on its own, except to draw.

/// Pointer state of the iso overlay, shared with the canvas view.
enum IsoHover {
    /// The cell under the pointer on the active isometric layer, nil when the pointer is off the canvas.
    static var cell: (i: Int, j: Int)?
    /// The cell the last stamp of the running stroke went to (a drag stamps each cell once).
    static var lastStamp: (i: Int, j: Int)?
}

// MARK: - Model

extension SpriteModel {
    /// The current layer is an isometric tilemap layer.
    var isIsoLayer: Bool { isTilemapLayer && pixmith_layer_is_iso(doc, UInt32(layer)) }

    /// Diamond cell size of an isometric tileset.
    func isoCellSize(of ts: Int) -> (w: Int, h: Int)? {
        var out: [UInt32] = [0, 0]
        guard pixmith_tileset_iso_cell(doc, UInt32(ts), &out) else { return nil }
        return (Int(out[0]), Int(out[1]))
    }

    /// Canvas pixel of cell (0, 0)'s box on an isometric layer.
    func isoOffset(of layer: Int) -> (x: Int, y: Int)? {
        var out: [Int32] = [0, 0]
        guard pixmith_layer_grid_offset(doc, UInt32(layer), &out) else { return nil }
        return (Int(out[0]), Int(out[1]))
    }

    /// The grid of the current layer when it is an isometric tilemap layer (cell size from the tileset, offset from the layer).
    var isoLayerGrid: IsoGrid? {
        guard isIsoLayer, let ts = tilesetIndex, let c = isoCellSize(of: ts), let o = isoOffset(of: layer) else { return nil }
        return IsoGrid(w: c.w, h: c.h, x: o.x, y: o.y)
    }

    /// The cell of the current isometric layer that contains a canvas pixel.
    func isoCell(at p: (Int, Int)) -> (i: Int, j: Int)? {
        var out: [Int32] = [0, 0]
        guard pixmith_layer_cell_at(doc, UInt32(layer), Int32(clamping: p.0), Int32(clamping: p.1), &out) else { return nil }
        return (Int(out[0]), Int(out[1]))
    }

    func newIsoTilemapLayer(tileWidth tw: Int, tileHeight th: Int, cellWidth cw: Int, cellHeight ch: Int, offsetX: Int, offsetY: Int) {
        stop()
        let i = pixmith_add_iso_tilemap_layer(doc, UInt32(layer), UInt32(tw), UInt32(th), UInt32(cw), UInt32(ch),
                                              Int32(clamping: offsetX), Int32(clamping: offsetY))
        guard pixmith_layer_is_iso(doc, i) else {
            errorMessage = "Could not create an isometric tilemap layer. \(lastError)"
            return
        }
        layer = Int(i)
        labelLastStep("New Isometric Tilemap Layer")
        TilemapState.shared.selected = 1
        TilemapState.shared.flipD = false
        revision &+= 1
    }

    /// Moves the grid of the current isometric layer (one undo step).
    func setIsoOffset(x: Int, y: Int) {
        guard isIsoLayer, isoOffset(of: layer).map({ $0 != (x, y) }) ?? false else { return }
        if pixmith_layer_set_grid_offset(doc, UInt32(layer), Int32(clamping: x), Int32(clamping: y)) {
            labelLastStep("Layer Offset")
            revision &+= 1
        } else {
            errorMessage = lastError
        }
    }

    // MARK: Tile pivots

    func tilePivot(_ ts: Int, _ tile: Int) -> (x: Int, y: Int)? {
        var out: [Int32] = [0, 0]
        guard pixmith_tile_pivot(doc, UInt32(ts), UInt32(tile), &out) else { return nil }
        return (Int(out[0]), Int(out[1]))
    }

    func tileTextureOrigin(_ ts: Int, _ tile: Int) -> (x: Int, y: Int)? {
        var out: [Int32] = [0, 0]
        guard pixmith_tile_texture_origin(doc, UInt32(ts), UInt32(tile), &out) else { return nil }
        return (Int(out[0]), Int(out[1]))
    }

    func isPivotSet(_ ts: Int, _ tile: Int) -> Bool { pixmith_tile_pivot_is_set(doc, UInt32(ts), UInt32(tile)) }

    func setPivot(_ ts: Int, _ tile: Int, x: Int, y: Int) {
        if pixmith_tile_set_pivot(doc, UInt32(ts), UInt32(tile), Int32(clamping: x), Int32(clamping: y)) {
            labelLastStep("Tile Pivot")
            revision &+= 1
        }
    }

    func clearPivot(_ ts: Int, _ tile: Int) {
        if pixmith_tile_clear_pivot(doc, UInt32(ts), UInt32(tile)) {
            labelLastStep("Clear Tile Pivot")
            revision &+= 1
        }
    }

    // MARK: Canvas

    /// Stamp mode on an isometric layer: the cell under the cursor gets the tile (Shift erases, Option picks what you see).
    func handleIsoTileMouse(_ e: NSEvent, at p: (Int, Int), began: Bool) -> Bool {
        let state = TilemapState.shared
        if e.modifierFlags.contains(.option) {
            var hit: [Int32] = [0, 0, 0]
            if began, pixmith_tile_hit_at(doc, UInt32(layer), UInt32(frame), Int32(clamping: p.0), Int32(clamping: p.1), &hit) {
                let v = UInt32(bitPattern: hit[2])
                if v & 0x1FFF_FFFF != 0 { state.select(value: v) }
                state.flipD = false
            }
            return true
        }
        guard let c = isoCell(at: p) else { return true }
        if began {
            beginStroke("Stamp Tile")
            IsoHover.lastStamp = nil
        }
        if let l = IsoHover.lastStamp, l == c { return true }
        IsoHover.lastStamp = c
        // The diagonal flip does not exist for isometric tiles.
        let value = e.modifierFlags.contains(.shift) ? 0 : state.value & ~TilemapState.flipD
        if pixmith_tile_stamp_cell(doc, UInt32(layer), UInt32(frame), Int32(clamping: c.i), Int32(clamping: c.j), value) {
            revision &+= 1
        }
        return true
    }

    /// Called when the pointer moves over the canvas; redraws the highlight when the cell under it changes.
    func isoPointerMoved(_ p: (Int, Int)?, in view: NSView) {
        let new = p.flatMap { isIsoLayer ? isoCell(at: $0) : nil }
        let old = IsoHover.cell
        if old?.i == new?.i, old?.j == new?.j { return }
        IsoHover.cell = new
        view.needsDisplay = true
    }

    /// Cells the next click would touch with the terrain brush, from the pointer cell.
    private func isoPreviewCells(hover: (i: Int, j: Int)) -> [(i: Int, j: Int)] {
        let st = AutotileState.shared
        guard st.brush, terrainMode >= 0 else { return [hover] }
        switch st.tool {
        case .paint:
            return Self.isoBrushCells(hover.i, hover.j, size: st.size, round: st.round)
        case .rect:
            guard let a = st.rectStart.flatMap({ isoCell(at: $0) }), let b = st.rectEnd.flatMap({ isoCell(at: $0) }) else { return [hover] }
            let (i0, i1, j0, j1) = (min(a.i, b.i), max(a.i, b.i), min(a.j, b.j), max(a.j, b.j))
            guard (i1 - i0 + 1) * (j1 - j0 + 1) <= 4096 else { return [hover] }
            return (j0...j1).flatMap { j in (i0...i1).map { (i: $0, j: j) } }
        default:
            return [hover]
        }
    }

    /// The core's brush footprint in cell space (a square block of cells, which is a rhombus on screen).
    static func isoBrushCells(_ i: Int, _ j: Int, size: Int, round: Bool) -> [(i: Int, j: Int)] {
        let n = min(max(size, 1), 64)
        let lo = -(n - 1) / 2
        var out: [(i: Int, j: Int)] = []
        for dy in 0..<n {
            for dx in 0..<n {
                if round, n > 2 {
                    let (qx, qy) = (2 * dx - (n - 1), 2 * dy - (n - 1))
                    if qx * qx + qy * qy > n * n { continue }
                }
                out.append((i + lo + dx, j + lo + dy))
            }
        }
        return out
    }

    /// Draws the isometric layer's overlays: the diamond under the cursor (or the cells the terrain brush would paint)
    /// and the diamonds of cells that use a fallback tile.
    func drawIsoOverlay(_ ctx: CGContext, dirty: CGRect) {
        guard let g = isoLayerGrid else { return }
        if terrainMode >= 0 { drawIsoMissing(ctx, grid: g, dirty: dirty) }
        guard let hover = IsoHover.cell else { return }
        let st = AutotileState.shared
        let base = st.brush && terrainMode >= 0 ? NSColor(terrainColor(st.terrain)) : NSColor.controlAccentColor
        g.draw(ctx, cells: isoPreviewCells(hover: hover), zoom: zoom, dirty: dirty,
               fill: base.withAlphaComponent(0.32).cgColor, stroke: base.withAlphaComponent(0.95).cgColor)
    }

    private func drawIsoMissing(_ ctx: CGContext, grid g: IsoGrid, dirty: CGRect) {
        let n = pixmith_autotile_missing_cells(doc, UInt32(layer), UInt32(frame), nil, 0)
        guard n > 0 else { return }
        var buf = [Int32](repeating: 0, count: n * 2)
        _ = pixmith_autotile_missing_cells(doc, UInt32(layer), UInt32(frame), &buf, buf.count)
        let z = zoom
        ctx.saveGState()
        ctx.setStrokeColor(NSColor.systemRed.cgColor)
        ctx.setLineWidth(2)
        for k in 0..<n {
            let r = CGRect(x: CGFloat(buf[k * 2]) * z, y: CGFloat(buf[k * 2 + 1]) * z, width: CGFloat(g.w) * z, height: CGFloat(g.h) * z)
            guard r.intersects(dirty) else { continue }
            IsoGrid.addDiamond(ctx, r.insetBy(dx: 2, dy: 1))
            ctx.move(to: CGPoint(x: r.midX - r.width * 0.18, y: r.midY - r.height * 0.18))
            ctx.addLine(to: CGPoint(x: r.midX + r.width * 0.18, y: r.midY + r.height * 0.18))
            ctx.move(to: CGPoint(x: r.midX + r.width * 0.18, y: r.midY - r.height * 0.18))
            ctx.addLine(to: CGPoint(x: r.midX - r.width * 0.18, y: r.midY + r.height * 0.18))
            ctx.strokePath()
        }
        ctx.restoreGState()
    }

    /// The tile grid of the selected isometric layer, as orange diamonds (the View > Isometric Grid already shows it).
    func drawIsoTileGrid(_ ctx: CGContext, size: CGSize, dirty: CGRect) {
        guard !ViewSettings.shared.isometricGrid, let g = isoLayerGrid, CGFloat(g.w) * zoom >= 8 else { return }
        g.stroke(ctx, in: CGRect(origin: .zero, size: size), dirty: dirty, zoom: zoom,
                 color: NSColor.systemOrange.withAlphaComponent(0.7).cgColor)
    }
}

// MARK: - IsoGrid drawing

extension IsoGrid {
    private static var templates: [String: [(dy: Int, l: Int, r: Int)]] = [:]

    /// The pixel rows of a cell relative to its box (cells are translations of each other).
    func templateRows() -> [(dy: Int, l: Int, r: Int)] {
        let key = "\(w)x\(h)"
        if let t = Self.templates[key] { return t }
        let g = IsoGrid(w: w, h: h, x: 0, y: 0)
        let rows = g.rows((0, 0)).map { (dy: $0.y, l: $0.l, r: $0.r) }
        Self.templates[key] = rows
        return rows
    }

    /// Adds the diamond inscribed in `r` as a closed subpath.
    static func addDiamond(_ ctx: CGContext, _ r: CGRect) {
        ctx.move(to: CGPoint(x: r.midX, y: r.minY))
        ctx.addLine(to: CGPoint(x: r.maxX, y: r.midY))
        ctx.addLine(to: CGPoint(x: r.midX, y: r.maxY))
        ctx.addLine(to: CGPoint(x: r.minX, y: r.midY))
        ctx.closePath()
    }

    /// Fills the pixels of each cell and outlines its diamond. `zoom` is view points per sprite pixel.
    func draw(_ ctx: CGContext, cells: [(i: Int, j: Int)], zoom z: CGFloat, dirty: CGRect, fill: CGColor, stroke: CGColor) {
        let rows = templateRows()
        let fillPath = CGMutablePath()
        let outline = CGMutablePath()
        var count = 0
        for c in cells {
            let b = box(c)
            let r = CGRect(x: CGFloat(b.x) * z, y: CGFloat(b.y) * z, width: CGFloat(w) * z, height: CGFloat(h) * z)
            guard r.intersects(dirty) else { continue }
            count += 1
            if count > 2048 { break }
            for r in rows {
                fillPath.addRect(CGRect(x: CGFloat(b.x + r.l) * z, y: CGFloat(b.y + r.dy) * z,
                                        width: CGFloat(r.r - r.l + 1) * z, height: z))
            }
            outline.move(to: CGPoint(x: r.midX, y: r.minY))
            outline.addLine(to: CGPoint(x: r.maxX, y: r.midY))
            outline.addLine(to: CGPoint(x: r.midX, y: r.maxY))
            outline.addLine(to: CGPoint(x: r.minX, y: r.midY))
            outline.closeSubpath()
        }
        ctx.saveGState()
        ctx.setFillColor(fill)
        ctx.addPath(fillPath)
        ctx.fillPath()
        ctx.setStrokeColor(stroke)
        ctx.setLineWidth(1.5)
        ctx.setLineJoin(.round)
        ctx.addPath(outline)
        ctx.strokePath()
        ctx.restoreGState()
    }

    /// The grid Select > Isometric Tile and the View > Isometric Grid use: the grid of the current isometric tilemap
    /// layer when there is one, else the one from View Options.
    static func active(for m: SpriteModel) -> IsoGrid { m.isoLayerGrid ?? .current }
}

// MARK: - Inspector

/// Offset and cell size of the selected isometric layer (the elevation levels of a map).
struct IsoLayerPanel: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        let _ = model.revision
        if let g = model.isoLayerGrid {
            VStack(alignment: .leading, spacing: DS.Space.sm) {
                HStack(spacing: DS.Space.xs) {
                    Text("Isometric layer").font(DS.Font.section).foregroundStyle(DSColor.text)
                    Text("\(g.w)×\(g.h) · Diamond Down").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                    Spacer(minLength: 0)
                }
                DSFormRow(label: "Offset", labelWidth: 48) {
                    DSNumberField("X", value: Binding(get: { model.isoOffset(of: model.layer)?.x ?? 0 },
                                                      set: { model.setIsoOffset(x: $0, y: model.isoOffset(of: model.layer)?.y ?? 0) }),
                                  range: -100_000...100_000, unit: "px", width: 96,
                                  help: "Canvas x of cell (0, 0)'s box", deferScrub: true)
                    DSNumberField("Y", value: Binding(get: { model.isoOffset(of: model.layer)?.y ?? 0 },
                                                      set: { model.setIsoOffset(x: model.isoOffset(of: model.layer)?.x ?? 0, y: $0) }),
                                  range: -100_000...100_000, unit: "px", width: 96,
                                  help: "Canvas y of cell (0, 0)'s box", deferScrub: true)
                }
                HStack(spacing: DS.Space.sm - 2) {
                    Button { model.setIsoOffset(x: g.x, y: g.y - g.h / 2) } label: {
                        Label("Raise ½ cell", systemImage: "arrow.up").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(DSBarButtonStyle()).help("Move the grid up by half a cell height (\(g.h / 2) px)")
                    Button { model.setIsoOffset(x: g.x, y: g.y + g.h / 2) } label: {
                        Label("Lower ½ cell", systemImage: "arrow.down").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(DSBarButtonStyle()).help("Move the grid down by half a cell height (\(g.h / 2) px)")
                }
                DSNote("Canvas pixel of cell (0, 0)'s box. Give each elevation level its own layer and offset; a level half a cell higher is \(g.h / 2) px up.")
            }
        }
    }
}

/// Pivot of the selected tile of an isometric tileset: the point of the image that sits on the diamond's center.
struct IsoPivotPanel: View {
    @ObservedObject var model: SpriteModel
    let ts: Int
    let tile: Int
    @State private var drag: (x: Int, y: Int)?

    var body: some View {
        let _ = model.revision
        let (tw, th) = model.tileSize(of: ts)
        if tile > 0, tile < model.tileCount(of: ts), tw > 0, th > 0,
           let stored = model.tilePivot(ts, tile), let cell = model.isoCellSize(of: ts) {
            let pivot = drag ?? stored
            let isSet = model.isPivotSet(ts, tile)
            let z = max(1, min(6, Int(200 / CGFloat(max(tw, th)))))
            VStack(alignment: .leading, spacing: DS.Space.sm) {
                HStack(spacing: DS.Space.xs) {
                    Text("Tile \(tile) pivot").font(DS.Font.captionMedium).foregroundStyle(DSColor.text)
                    Text(isSet ? "custom" : "default").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                    Spacer(minLength: 0)
                    Button("Clear") { model.clearPivot(ts, tile) }
                        .buttonStyle(DSBarButtonStyle()).disabled(!isSet)
                        .help("Back to the default: centered across, bottom-aligned on the diamond")
                }
                HStack(spacing: DS.Space.sm - 2) {
                    DSNumberField("X", value: Binding(get: { pivot.x }, set: { model.setPivot(ts, tile, x: $0, y: pivot.y) }),
                                  range: -2048...2048, unit: "px", width: 96, help: "Pivot x from the image's left edge", deferScrub: true)
                    DSNumberField("Y", value: Binding(get: { pivot.y }, set: { model.setPivot(ts, tile, x: pivot.x, y: $0) }),
                                  range: -2048...2048, unit: "px", width: 96, help: "Pivot y from the image's top edge", deferScrub: true)
                }
                preview(tw: tw, th: th, z: z, cell: cell, pivot: pivot)
                if let o = model.tileTextureOrigin(ts, tile) {
                    DSNote("Godot texture_origin (\(o.x), \(o.y)). Click the picture to set the pivot; the diamond is one cell.")
                }
            }
        }
    }

    private func preview(tw: Int, th: Int, z: Int, cell: (w: Int, h: Int), pivot: (x: Int, y: Int)) -> some View {
        let zf = CGFloat(z)
        return ZStack(alignment: .topLeading) {
            DSChecker(cell: 4)
            if let img = model.tileImage(tileset: ts, tile: tile) {
                Image(decorative: img, scale: 1).interpolation(.none).resizable()
            }
            Canvas { gc, _ in
                let c = CGPoint(x: CGFloat(pivot.x) * zf, y: CGFloat(pivot.y) * zf)
                let hw = CGFloat(cell.w) * zf / 2, hh = CGFloat(cell.h) * zf / 2
                var d = Path()
                d.move(to: CGPoint(x: c.x, y: c.y - hh))
                d.addLine(to: CGPoint(x: c.x + hw, y: c.y))
                d.addLine(to: CGPoint(x: c.x, y: c.y + hh))
                d.addLine(to: CGPoint(x: c.x - hw, y: c.y))
                d.closeSubpath()
                gc.stroke(d, with: .color(.orange.opacity(0.9)), lineWidth: 1)
                var cross = Path()
                cross.move(to: CGPoint(x: c.x - 5, y: c.y)); cross.addLine(to: CGPoint(x: c.x + 5, y: c.y))
                cross.move(to: CGPoint(x: c.x, y: c.y - 5)); cross.addLine(to: CGPoint(x: c.x, y: c.y + 5))
                gc.stroke(cross, with: .color(.white), lineWidth: 3)
                gc.stroke(cross, with: .color(.red), lineWidth: 1.5)
            }
            .allowsHitTesting(false)
        }
        .frame(width: CGFloat(tw * z), height: CGFloat(th * z))
        .clipShape(RoundedRectangle(cornerRadius: 3))
        .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.white.opacity(0.12), lineWidth: 1))
        .contentShape(Rectangle())
        .gesture(DragGesture(minimumDistance: 0)
            .onChanged { drag = point($0.location, z: z, tw: tw, th: th) }
            .onEnded { g in
                let p = point(g.location, z: z, tw: tw, th: th)
                drag = nil
                model.setPivot(ts, tile, x: p.x, y: p.y)
            })
        .help("Click to set the pivot (the point that sits on the diamond's center)")
    }

    private func point(_ loc: CGPoint, z: Int, tw: Int, th: Int) -> (x: Int, y: Int) {
        (min(max(Int((loc.x / CGFloat(z)).rounded()), 0), tw), min(max(Int((loc.y / CGFloat(z)).rounded()), 0), th))
    }
}
