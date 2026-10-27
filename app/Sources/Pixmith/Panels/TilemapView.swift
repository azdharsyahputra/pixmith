import AppKit
import Combine
import PixmithCore
import SwiftUI

/// UI state for tilemap editing that is independent of the document.
/// The boxes of the tile copies outlined under the pointer, to redraw only when they change.
enum TileCopiesHover {
    static var boxes: [CGRect] = []
    /// The pointer in canvas view points, where the label chip sits next to; nil until the pointer has moved.
    static var pointer: CGPoint?
    /// The area the label chip covered when last drawn, to redraw it as the pointer moves.
    static var chip: CGRect = .zero
}

/// The cell box (canvas pixels) that shows the Tiles-mode ghost, to redraw only when it changes.
enum TileGhostHover {
    static var cell: CGRect?
}

final class TilemapState: ObservableObject {
    static let shared = TilemapState()

    enum Sheet: Identifiable {
        case newLayer, convert, rename
        var id: Int {
            switch self {
            case .newLayer: 0
            case .convert: 1
            case .rename: 2
            }
        }
    }

    static let flipX: UInt32 = 0x8000_0000
    static let flipY: UInt32 = 0x4000_0000
    static let flipD: UInt32 = 0x2000_0000

    /// While on, canvas clicks place the selected tile instead of drawing pixels.
    @Published var stamp = false
    /// Auto Tiles (`tiles.auto`): painting a cell whose tile other cells show too gives it its own copy; off, painting
    /// edits the tile everywhere it is used.
    @Published var autoTiles = UserDefaults.standard.object(forKey: "tiles.auto") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(autoTiles, forKey: "tiles.auto")
            pixmith_set_auto_tiles(autoTiles)
        }
    }
    @Published var selected = 1
    @Published var flipX = false
    @Published var flipY = false
    @Published var flipD = false
    @Published var sheet: Sheet?
    /// Shows the "You're placing tiles" hint chip over the canvas for a few seconds.
    @Published var stampHint = false
    /// Stamps in a row that changed nothing because the cell already held the tile.
    private(set) var noopStamps = 0
    private var hintGeneration = 0
    private var bag = Set<AnyCancellable>()

    private init() {
        pixmith_set_auto_tiles(autoTiles)
        // Choosing any drawing tool leaves stamp mode, including the one that is already selected (@Published sends
        // on every assignment).
        SpriteModel.shared.$tool.dropFirst().sink { [weak self] _ in self?.stamp = false }.store(in: &bag)
        // So does choosing a color: the user is about to draw pixels.
        SpriteModel.shared.$color.dropFirst().sink { [weak self] _ in self?.stamp = false }.store(in: &bag)
        $stamp.dropFirst().sink { [weak self] on in
            self?.noopStamps = 0
            if !on { self?.stampHint = false }
        }.store(in: &bag)
        // The canvas draws the ghost, the copies outline and its label from this state.
        objectWillChange.sink { SpriteModel.shared.objectWillChange.send() }.store(in: &bag)
    }

    /// A stamp of a click changed a cell.
    func stampChanged() { noopStamps = 0 }

    /// A click stamped a tile the cell already held. The second one in a row shows the hint for a few seconds.
    func stampDidNothing() {
        noopStamps += 1
        guard noopStamps >= 2 else { return }
        noopStamps = 0
        stampHint = true
        hintGeneration += 1
        let g = hintGeneration
        DispatchQueue.main.asyncAfter(deadline: .now() + 4) { [weak self] in
            if let self, self.hintGeneration == g { self.stampHint = false }
        }
    }

    /// The selected tile index with the current flip flags.
    var value: UInt32 {
        UInt32(selected) | (flipX ? Self.flipX : 0) | (flipY ? Self.flipY : 0) | (flipD ? Self.flipD : 0)
    }

    func select(value v: UInt32) {
        selected = Int(v & 0x1FFF_FFFF)
        flipX = v & Self.flipX != 0
        flipY = v & Self.flipY != 0
        flipD = v & Self.flipD != 0
    }
}

extension SpriteModel {
    var isTilemapLayer: Bool { pixmith_layer_is_tilemap(doc, UInt32(layer)) }

    /// Tileset index of the current layer, if it is a tilemap layer.
    var tilesetIndex: Int? {
        let i = pixmith_layer_tileset(doc, UInt32(layer))
        return i >= 0 ? Int(i) : nil
    }

    func tileSize(of ts: Int) -> (w: Int, h: Int) {
        (Int(pixmith_tileset_tile_width(doc, UInt32(ts))), Int(pixmith_tileset_tile_height(doc, UInt32(ts))))
    }

    func tileCount(of ts: Int) -> Int { Int(pixmith_tileset_tile_count(doc, UInt32(ts))) }
    func tileBase(of ts: Int) -> Int { Int(pixmith_tileset_base_index(doc, UInt32(ts))) }

    func tilesetName(_ ts: Int) -> String {
        guard let p = pixmith_tileset_name(doc, UInt32(ts)) else { return "" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    /// The tile's picture, shown with flip flags (the core's order: diagonal, then X, then Y; diagonal only for square tiles).
    func tileImage(tileset ts: Int, tile: Int, flags: UInt32 = 0) -> CGImage? {
        let (w, h) = tileSize(of: ts)
        guard w > 0, h > 0 else { return nil }
        var buf = [UInt8](repeating: 0, count: w * h * 4)
        guard pixmith_tileset_tile_pixels(doc, UInt32(ts), UInt32(tile), &buf, buf.count) else { return nil }
        if flags & (TilemapState.flipX | TilemapState.flipY | TilemapState.flipD) != 0 {
            let src = buf
            for cy in 0..<h {
                for cx in 0..<w {
                    var (a, b) = (cx, cy)
                    if flags & TilemapState.flipY != 0 { b = h - 1 - b }
                    if flags & TilemapState.flipX != 0 { a = w - 1 - a }
                    if flags & TilemapState.flipD != 0, w == h { swap(&a, &b) }
                    for k in 0..<4 { buf[(cy * w + cx) * 4 + k] = src[(b * w + a) * 4 + k] }
                }
            }
        }
        guard let provider = CGDataProvider(data: Data(buf) as CFData) else { return nil }
        return CGImage(
            width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }

    func newTilemapLayer(tileWidth w: Int, tileHeight h: Int) {
        stop()
        layer = Int(pixmith_add_tilemap_layer(doc, UInt32(layer), UInt32(w), UInt32(h)))
        labelLastStep("New Tilemap Layer")
        TilemapState.shared.selected = 1
        revision &+= 1
    }

    func convertToTilemap(tileWidth w: Int, tileHeight h: Int, flipAware: Bool = true) {
        guard pixmith_convert_to_tilemap_ex(doc, UInt32(layer), UInt32(w), UInt32(h), flipAware) else {
            errorMessage = "Only normal layers can be converted to tilemaps."
            return
        }
        labelLastStep("Convert to Tilemap")
        TilemapState.shared.selected = 1
        revision &+= 1
    }

    func convertToNormal() {
        if pixmith_convert_to_normal(doc, UInt32(layer)) { labelLastStep("Convert to Layer"); TilemapState.shared.stamp = false; revision &+= 1 }
    }

    func addTile() {
        guard let ts = tilesetIndex else { return }
        let i = pixmith_tileset_add_tile(doc, UInt32(ts))
        if i > 0 { labelLastStep("Add Tile"); TilemapState.shared.selected = Int(i) }
        revision &+= 1
    }

    var tilesetCount: Int { Int(pixmith_tileset_count(doc)) }

    /// Other tilesets with the same tile size as the current layer's, which it could switch to.
    var compatibleTilesets: [Int] {
        guard let cur = tilesetIndex else { return [] }
        let size = tileSize(of: cur), cell = isoCellSize(of: cur)
        return (0..<tilesetCount).filter {
            let c = isoCellSize(of: $0)
            return $0 != cur && tileSize(of: $0) == size && c?.w == cell?.w && c?.h == cell?.h
        }
    }

    func assignTileset(_ ts: Int) {
        if pixmith_layer_assign_tileset(doc, UInt32(layer), UInt32(ts)) {
            labelLastStep("Change Tileset")
            TilemapState.shared.selected = min(TilemapState.shared.selected, max(1, tileCount(of: tilesetIndex ?? 0) - 1))
            revision &+= 1
        }
    }

    func renameTileset(_ ts: Int, to name: String) {
        if name.withCString({ pixmith_tileset_rename(doc, UInt32(ts), $0) }) { labelLastStep("Rename Tileset"); revision &+= 1 }
    }

    func duplicateTile(_ tile: Int) {
        guard let ts = tilesetIndex else { return }
        let n = pixmith_tileset_duplicate_tile(doc, UInt32(ts), UInt32(tile))
        if n > 0 { labelLastStep("Duplicate Tile"); TilemapState.shared.selected = Int(n) }
        revision &+= 1
    }

    func deleteTile(_ tile: Int) {
        guard let ts = tilesetIndex, pixmith_tileset_delete_tile(doc, UInt32(ts), UInt32(tile)) else { return }
        labelLastStep("Delete Tile")
        let state = TilemapState.shared
        if state.selected >= tileCount(of: ts) { state.selected = max(1, tileCount(of: ts) - 1) }
        revision &+= 1
    }

    /// Handles a canvas mouse event in stamp mode. Returns true if the event was consumed.
    /// Plain click places the selected tile, Shift erases, Option picks the tile under the cursor.
    func handleTileMouse(_ e: NSEvent, at p: (Int, Int), began: Bool) -> Bool {
        let state = TilemapState.shared
        // Stamp mode belongs to the painting tools; selection, move, slice, picker and shape tools work as usual.
        guard state.stamp, isTilemapLayer, tool == .pencil || tool == .eraser else { return false }
        if isIsoLayer { return handleIsoTileMouse(e, at: p, began: began) }
        if e.modifierFlags.contains(.option) {
            let v = pixmith_tile_at(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1))
            if v & 0x1FFF_FFFF != 0 { state.select(value: v) }
            return true
        }
        if began { beginStroke("Stamp Tile") }
        let value = e.modifierFlags.contains(.shift) ? 0 : state.value
        if pixmith_tile_stamp(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1), value) {
            revision &+= 1
            state.stampChanged()
        } else if began {
            state.stampDidNothing()
        }
        return true
    }

    /// Tiles mode: the tile stamp is on for a tool it works with, and the terrain brush is not in the way.
    var tilesMode: Bool {
        TilemapState.shared.stamp && isTilemapLayer && (tool == .pencil || tool == .eraser)
            && !(AutotileState.shared.brush && terrainMode >= 0)
    }

    /// Switches the tool options bar's Pixels | Tiles control. Tiles needs the pencil or eraser; the terrain brush is off in both.
    func setTileMode(_ tiles: Bool) {
        guard isTilemapLayer else { return }
        if tiles, tool != .pencil, tool != .eraser { tool = .pencil }  // sends on $tool, which turns the stamp off
        AutotileState.shared.brush = false
        TilemapState.shared.stamp = tiles
    }

    /// The Pixels / Tiles shortcut.
    func toggleTileMode() {
        guard isTilemapLayer else { return }
        setTileMode(!TilemapState.shared.stamp)
    }

    /// Boxes (canvas pixels) of the cells showing the same tile as the cell under `p`, when there are several:
    /// painting that tile with Auto Tiles off changes all of them.
    func tileCopies(at p: (Int, Int)?) -> [CGRect] {
        // Only while drawing pixels: Tiles mode shows the tile ghost instead, the terrain brush its own cells.
        guard let p, isTilemapLayer, !TilemapState.shared.stamp, !(AutotileState.shared.brush && terrainMode >= 0) else { return [] }
        var buf = [Int32](repeating: 0, count: 4 * 4096)
        let n = pixmith_tile_copies_at(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1), &buf, buf.count)
        guard n >= 2 else { return [] }
        return (0..<min(n, 4096)).map { k in
            CGRect(x: Int(buf[k * 4]), y: Int(buf[k * 4 + 1]), width: Int(buf[k * 4 + 2]), height: Int(buf[k * 4 + 3]))
        }
    }

    /// Redraws the canvas when the pointer moves onto a cell whose copies differ from the outlined ones.
    /// `pointer` is the pointer in the view's points; the label chip follows it while the outline is up.
    func tileCopiesPointerMoved(_ p: (Int, Int)?, at pointer: CGPoint? = nil, in view: NSView) {
        let boxes = tileCopies(at: p)
        TileCopiesHover.pointer = pointer
        if boxes == TileCopiesHover.boxes {
            guard !boxes.isEmpty else { return }
            // Same outline: only the chip moves.
            view.setNeedsDisplay(TileCopiesHover.chip.insetBy(dx: -2, dy: -2))
            if let chip = tileCopiesChip(boxes, at: p) { view.setNeedsDisplay(chip.rect.insetBy(dx: -2, dy: -2)) }
            return
        }
        TileCopiesHover.boxes = boxes
        view.needsDisplay = true
    }

    /// The chip next to the pointer for the outlined copies: tile N, how many cells, and what painting would change.
    private func tileCopiesChip(_ boxes: [CGRect], at p: (Int, Int)?) -> (rect: CGRect, lines: [NSAttributedString])? {
        guard let p, !boxes.isEmpty else { return nil }
        var tile = pixmith_tile_at(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1))
        var hit: [Int32] = [0, 0, 0]
        if isIsoLayer {
            tile = pixmith_tile_hit_at(doc, UInt32(layer), UInt32(frame), Int32(clamping: p.0), Int32(clamping: p.1), &hit)
                ? UInt32(bitPattern: hit[2]) : 0
        }
        let n = boxes.count
        let first = NSAttributedString(string: "Tile \(tile & 0x1FFF_FFFF) · \(n) cells",
                                       attributes: [.font: NSFont.systemFont(ofSize: 11, weight: .medium), .foregroundColor: DSColor.textNS])
        let second = NSAttributedString(string: TilemapState.shared.autoTiles ? "Painting changes this cell only" : "Painting changes all \(n)",
                                        attributes: [.font: NSFont.systemFont(ofSize: 11), .foregroundColor: DSColor.text2NS])
        let (w1, w2) = (first.size(), second.size())
        let size = CGSize(width: ceil(max(w1.width, w2.width)) + 2 * DS.Space.sm, height: ceil(w1.height + w2.height) + DS.Space.sm)
        let at = TileCopiesHover.pointer ?? CGPoint(x: (CGFloat(p.0) + 0.5) * zoom, y: (CGFloat(p.1) + 0.5) * zoom)
        let area = CGSize(width: CGFloat(width) * zoom, height: CGFloat(height) * zoom)
        // 16 pt right and down of the pointer; kept inside the canvas view by flipping to the other side.
        var o = CGPoint(x: at.x + 16, y: at.y + 16)
        if o.x + size.width > area.width { o.x = at.x - 16 - size.width }
        if o.y + size.height > area.height { o.y = at.y - 16 - size.height }
        return (CGRect(origin: CGPoint(x: max(0, o.x), y: max(0, o.y)), size: size), [first, second])
    }

    /// Outlines the copies of the tile under the pointer (squares, or diamonds on isometric layers).
    func drawTileCopies(_ ctx: CGContext) {
        let boxes = tileCopies(at: StatusInfo.shared.cursor.map { ($0.x, $0.y) })
        TileCopiesHover.boxes = boxes
        guard !boxes.isEmpty else {
            TileCopiesHover.chip = .zero
            return
        }
        ctx.saveGState()
        ctx.setFillColor(DSColor.accentNS.withAlphaComponent(0.14).cgColor)
        ctx.setStrokeColor(DSColor.accentNS.cgColor)
        ctx.setLineWidth(1.5)
        for b in boxes {
            let r = CGRect(x: b.minX * zoom, y: b.minY * zoom, width: b.width * zoom, height: b.height * zoom)
            if isIsoLayer { IsoGrid.addDiamond(ctx, r) } else { ctx.addRect(r.insetBy(dx: 0.75, dy: 0.75)) }
        }
        ctx.drawPath(using: .fillStroke)
        ctx.restoreGState()
        drawTileCopiesChip(ctx, boxes)
    }

    private func drawTileCopiesChip(_ ctx: CGContext, _ boxes: [CGRect]) {
        let p = StatusInfo.shared.cursor.map { ($0.x, $0.y) }
        guard let chip = tileCopiesChip(boxes, at: p) else { return }
        TileCopiesHover.chip = chip.rect
        ctx.saveGState()
        ctx.setFillColor(DSColor.overlayNS.cgColor)
        ctx.addPath(CGPath(roundedRect: chip.rect, cornerWidth: DS.Radius.field, cornerHeight: DS.Radius.field, transform: nil))
        ctx.fillPath()
        ctx.restoreGState()
        var y = chip.rect.minY + DS.Space.sm / 2
        for line in chip.lines {
            line.draw(at: CGPoint(x: chip.rect.minX + DS.Space.sm, y: y))
            y += line.size().height
        }
    }

    // MARK: Tiles-mode ghost

    /// The cell box (canvas pixels) under the pointer while Tiles mode is on: the tile box for square tilemaps, the
    /// diamond's box for isometric ones.
    func tileGhostCell(at p: (Int, Int)?) -> CGRect? {
        guard let p, tilesMode, let ts = tilesetIndex else { return nil }
        if isIsoLayer {
            guard let g = isoLayerGrid, let c = isoCell(at: p) else { return nil }
            let b = g.box(c)
            return CGRect(x: b.x, y: b.y, width: g.w, height: g.h)
        }
        let (tw, th) = tileSize(of: ts)
        var o: [Int32] = [0, 0]
        guard tw > 0, th > 0, pixmith_tile_grid_origin(doc, UInt32(layer), UInt32(frame), &o) else { return nil }
        func snap(_ v: Int, _ origin: Int32, _ step: Int) -> Int {
            let d = v - Int(origin)
            return Int(origin) + (d >= 0 ? d / step : -((-d + step - 1) / step)) * step
        }
        return CGRect(x: snap(p.0, o[0], tw), y: snap(p.1, o[1], th), width: tw, height: th)
    }

    /// Redraws the canvas when the pointer moves to another cell (or out) while Tiles mode is on.
    func tileGhostPointerMoved(_ p: (Int, Int)?, in view: NSView) {
        let new = tileGhostCell(at: p)
        if new == TileGhostHover.cell { return }
        TileGhostHover.cell = new
        view.needsDisplay = true
    }

    /// The selected tile at 65% opacity in the cell under the pointer, with a 1px accent outline of the cell.
    /// Isometric layers put the image where the core does (its pivot on the diamond's center); the diamond itself is
    /// the hover highlight of `drawIsoOverlay`.
    func drawTileGhost(_ ctx: CGContext) {
        let cell = tileGhostCell(at: StatusInfo.shared.cursor.map { ($0.x, $0.y) })
        TileGhostHover.cell = cell
        guard let cell, let ts = tilesetIndex else { return }
        let z = zoom
        let iso = isIsoLayer
        let st = TilemapState.shared
        // The diagonal flip does not exist for isometric tiles.
        let value = iso ? st.value & ~TilemapState.flipD : st.value
        let tile = Int(value & 0x1FFF_FFFF)
        if tile > 0, tile < tileCount(of: ts), let img = tileImage(tileset: ts, tile: tile, flags: value & 0xE000_0000) {
            let (tw, th) = tileSize(of: ts)
            var origin = cell.origin
            if iso, var pv = tilePivot(ts, tile) {
                if value & TilemapState.flipX != 0 { pv.x = tw - pv.x }
                if value & TilemapState.flipY != 0 { pv.y = th - pv.y }
                origin = CGPoint(x: cell.midX - CGFloat(pv.x), y: cell.midY - CGFloat(pv.y))
            }
            CanvasOverlays.drawImage(ctx, img, in: CGRect(x: origin.x * z, y: origin.y * z, width: CGFloat(tw) * z, height: CGFloat(th) * z),
                                     alpha: 0.65)
        }
        if !iso {
            ctx.saveGState()
            ctx.setStrokeColor(DSColor.accentNS.cgColor)
            ctx.setLineWidth(1)
            ctx.stroke(CGRect(x: cell.minX * z, y: cell.minY * z, width: cell.width * z, height: cell.height * z).insetBy(dx: 0.5, dy: 0.5))
            ctx.restoreGState()
        }
    }

    /// Draws tile boundaries over the canvas while a tilemap layer is selected.
    func drawTileGrid(_ ctx: CGContext, size: CGSize, scale: CGFloat, dirty: CGRect? = nil) {
        guard let ts = tilesetIndex else { return }
        if isIsoLayer {
            drawIsoTileGrid(ctx, size: size, dirty: dirty ?? CGRect(origin: .zero, size: size))
            return
        }
        let (tw, th) = tileSize(of: ts)
        let sx = CGFloat(tw) * zoom, sy = CGFloat(th) * zoom
        guard tw > 0, th > 0, sx >= 4, sy >= 4 else { return }
        ctx.saveGState()
        ctx.setStrokeColor(NSColor.systemOrange.withAlphaComponent(0.7).cgColor)
        ctx.setLineWidth(1 / scale)
        var origin: [Int32] = [0, 0]
        _ = pixmith_tile_grid_origin(doc, UInt32(layer), UInt32(frame), &origin)
        var x = CGFloat(origin[0]) * zoom
        while x <= size.width + 0.5 {
            ctx.move(to: CGPoint(x: x, y: 0))
            ctx.addLine(to: CGPoint(x: x, y: size.height))
            x += sx
        }
        var y = CGFloat(origin[1]) * zoom
        while y <= size.height + 0.5 {
            ctx.move(to: CGPoint(x: 0, y: y))
            ctx.addLine(to: CGPoint(x: size.width, y: y))
            y += sy
        }
        ctx.strokePath()
        ctx.restoreGState()
    }
}

/// Menu with the tilemap actions; used by the Tileset section and the layers menu.
struct TilemapMenuItems: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = TilemapState.shared

    var body: some View {
        Button("New Tilemap Layer…") { state.sheet = .newLayer }
        Button("Convert Layer to Tilemap…") { state.sheet = .convert }
            .disabled(model.isTilemapLayer || model.isGroup(model.layer))
        Button("Convert Tilemap to Normal Layer") { model.convertToNormal() }
            .disabled(!model.isTilemapLayer)
        if model.isTilemapLayer {
            Divider()
            Button("Rename Tileset…") { state.sheet = .rename }
            Menu("Use Tileset") {
                ForEach(model.compatibleTilesets, id: \.self) { i in
                    Button(model.tilesetName(i)) { model.assignTileset(i) }
                }
            }
            .disabled(model.compatibleTilesets.isEmpty)
        }
    }
}

extension SpriteModel {
    /// The Tileset section only shows while a tilemap layer is selected.
    var showsTilesetSection: Bool { tilesetIndex != nil }
}

/// Presents the tilemap sheets. Attached to a view that is always on screen, so the sheets also open from the
/// layers menu while the Tileset section is hidden.
private struct TilemapSheets: ViewModifier {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = TilemapState.shared

    func body(content: Content) -> some View {
        content.sheet(item: $state.sheet) { sheet in
            if sheet == .rename {
                TilesetRenameSheet(model: model)
            } else {
                TileSizeSheet(model: model, convert: sheet == .convert)
            }
        }
    }
}

extension View {
    func tilemapSheets(model: SpriteModel) -> some View { modifier(TilemapSheets(model: model)) }
}

/// Inspector section for the tileset of the selected tilemap layer.
struct TilemapPanel: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = TilemapState.shared

    var body: some View {
        if let ts = model.tilesetIndex {
            let (tw, th) = model.tileSize(of: ts)
            let iso = model.isoCellSize(of: ts)
            InspectorSection("Tileset", id: "tileset",
                             subtitle: (iso.map { "iso \($0.w)×\($0.h) · " } ?? "") + "\(tw)×\(th) · \(model.tileCount(of: ts)) tiles") {
                IconButton("plus", help: "Add empty tile", compact: true) { model.addTile() }
                DSMenuButton(help: "Tilemap layers") { TilemapMenuItems(model: model) }
            } content: {
                VStack(alignment: .leading, spacing: DS.Space.md) {
                    if iso != nil {
                        IsoLayerPanel(model: model)
                        Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
                    }
                    tilesetBody(ts, tw: tw, th: th)
                    if iso != nil { IsoPivotPanel(model: model, ts: ts, tile: state.selected) }
                    AutotilePanel(model: model)
                }
            }
        }
    }

    @ViewBuilder
    private func tilesetBody(_ ts: Int, tw: Int, th: Int) -> some View {
        let count = model.tileCount(of: ts)
        let cell = max(24, min(44, CGFloat(max(tw, th)) * 3))
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: cell), spacing: 3)], spacing: 3) {
                    ForEach(0..<count, id: \.self) { i in
                        tileCell(ts, i, size: cell, base: model.tileBase(of: ts))
                    }
                }
                .padding(2)
            }
            .frame(maxHeight: 132)
            HStack(spacing: DS.Space.xs) {
                DSChip("Stamp", symbol: "square.on.square.dashed",
                       help: "Place the selected tile on click. Shift-click erases, Option-click picks.", isOn: $state.stamp)
                Spacer()
                DSChip("X", help: "Flip horizontally", isOn: $state.flipX)
                DSChip("Y", help: "Flip vertically", isOn: $state.flipY)
                DSChip("D", help: "Flip diagonally (square tiles)", isOn: $state.flipD)
                    .disabled(model.isoCellSize(of: ts) != nil)
            }
        }
    }

    private func tileCell(_ ts: Int, _ i: Int, size: CGFloat, base: Int) -> some View {
        let selected = i == state.selected
        return ZStack {
            DSChecker(cell: 4)
            if let img = model.tileImage(tileset: ts, tile: i) {
                Image(decorative: img, scale: 1)
                    .interpolation(.none)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: 3))
        .overlay(alignment: .bottomTrailing) {
            Text(i == 0 ? "∅" : "\(base + i - 1)").font(.system(size: 8, weight: .medium).monospacedDigit()).padding(.horizontal, 2)
                .background(Color.black.opacity(0.55)).foregroundColor(.white)
                .clipShape(RoundedRectangle(cornerRadius: 2)).padding(1)
        }
        .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.white.opacity(0.07), lineWidth: 1))
        .overlay {
            if selected { RoundedRectangle(cornerRadius: 4).stroke(DSColor.accent, lineWidth: 2).padding(-1.5) }
        }
        .overlay(alignment: .topLeading) { AutotileBadge(model: model, tile: i) }
        .contentShape(Rectangle())
        .onTapGesture {
            state.selected = i
            model.setTileMode(true)
        }
        .contextMenu {
            Button("Duplicate Tile") { model.duplicateTile(i) }
            Button("Delete Tile") { model.deleteTile(i) }.disabled(i == 0)
        }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

struct TileSizeSheet: View {
    @ObservedObject var model: SpriteModel
    let convert: Bool
    @Environment(\.dismiss) private var dismiss
    @State private var width = 16
    @State private var height = 16
    @State private var flipAware = true
    @State private var isometric = false
    @State private var cellW = 96
    @State private var cellH = 48
    @State private var offsetX = 0
    @State private var offsetY = 0
    /// Whether the tile image size still follows the diamond size (tall tiles unlink it).
    @State private var linked = true

    private var iso: Bool { isometric && !convert }

    var body: some View {
        DSDialog(title: convert ? "Convert to Tilemap" : "New Tilemap Layer", primary: convert ? "Convert" : "Create",
                 width: 360, onPrimary: {
                     if iso {
                         let cw = IsoGrid.size(min(max(cellW, 4), 256)), ch = IsoGrid.size(min(max(cellH, 4), 256))
                         model.newIsoTilemapLayer(tileWidth: min(max(width, 1), 512), tileHeight: min(max(height, 1), 512),
                                                  cellWidth: cw, cellHeight: ch, offsetX: offsetX, offsetY: offsetY)
                     } else {
                         let w = min(max(width, 1), 256), h = min(max(height, 1), 256)
                         if convert { model.convertToTilemap(tileWidth: w, tileHeight: h, flipAware: flipAware) }
                         else { model.newTilemapLayer(tileWidth: w, tileHeight: h) }
                     }
                     dismiss()
                 }, onCancel: { dismiss() }) {
            if !convert {
                DSFormRow(label: "Grid") {
                    DSSegmented(selection: $isometric, options: [
                        .init(false, title: "Square", help: "Square cells"),
                        .init(true, title: "Isometric", help: "2:1 diamond cells, as in Godot's isometric TileMap"),
                    ])
                }
            }
            if iso {
                DSFormRow(label: "Layout") {
                    DSPopup(selection: .constant(0), options: [(0, "Diamond Down")], help: "Godot tile layout", fill: true)
                }
                DSFormRow(label: "Cell size") {
                    DSNumberField("W", value: Binding(get: { cellW }, set: { setCell(w: $0, h: cellH) }), range: 4...256, unit: "px", width: 92,
                                  help: "Width of the diamond")
                    DSNumberField("H", value: Binding(get: { cellH }, set: { setCell(w: cellW, h: $0) }), range: 4...256, unit: "px", width: 92,
                                  help: "Height of the diamond (half the width for a 2:1 tile)")
                }
                DSFormRow(label: "Tile image") {
                    DSNumberField("W", value: Binding(get: { width }, set: { width = $0; linked = false }), range: 1...512, unit: "px", width: 92,
                                  help: "Width of a tile picture")
                    DSNumberField("H", value: Binding(get: { height }, set: { height = $0; linked = false }), range: 1...512, unit: "px", width: 92,
                                  help: "Height of a tile picture. Taller than the diamond for cliffs and trees.")
                }
                DSFormRow(label: "Grid offset") {
                    DSNumberField("X", value: $offsetX, range: -100_000...100_000, unit: "px", width: 92,
                                  help: "Canvas x of cell (0, 0)'s box")
                    DSNumberField("Y", value: $offsetY, range: -100_000...100_000, unit: "px", width: 92,
                                  help: "Canvas y of cell (0, 0)'s box")
                }
                DSNote("Cell size and offset start from View Options > Custom Grid > Isometric. Tiles taller than the diamond get their pivot at the bottom; edit it in the Tileset section.")
            } else {
                DSFormRow(label: "Tile width") { DSNumberField("W", value: $width, range: 1...256, unit: "px", width: 120) }
                DSFormRow(label: "Tile height") { DSNumberField("H", value: $height, range: 1...256, unit: "px", width: 120) }
            }
            if convert {
                Toggle("Share flipped copies of tiles", isOn: $flipAware).toggleStyle(.checkbox)
            }
        }
        .onChange(of: isometric) { on in
            guard on else { return }
            let s = ViewSettings.shared
            // The iso grid's size, offset and a flat tile as defaults.
            cellW = s.isoGridW
            cellH = s.isoGridH
            offsetX = s.isoGridX
            offsetY = s.isoGridY
            width = cellW
            height = cellH
            linked = true
        }
    }

    private func setCell(w: Int, h: Int) {
        let follows = linked && width == cellW && height == cellH
        cellW = w
        cellH = h
        if follows { width = w; height = h }
    }
}

struct TilesetRenameSheet: View {
    @ObservedObject var model: SpriteModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""

    var body: some View {
        DSDialog(title: "Rename Tileset", primary: "Rename", width: 300, onPrimary: {
            if let ts = model.tilesetIndex { model.renameTileset(ts, to: name) }
            dismiss()
        }, onCancel: { dismiss() }) {
            TextField("Name", text: $name).dsTextField()
        }
        .onAppear { name = model.tilesetIndex.map { model.tilesetName($0) } ?? "" }
    }
}

struct TilemapCommands: Commands {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = TilemapState.shared
    @ObservedObject private var sheet = SheetState.shared

    var body: some Commands {
        CommandMenu("Tilemap") {
            Group {
            Button("New Tilemap Layer…") { state.sheet = .newLayer }
            Button("Convert Layer to Tilemap…") { state.sheet = .convert }
                .disabled(model.isTilemapLayer || model.isGroup(model.layer))
            Button("Convert Tilemap to Normal Layer") { model.convertToNormal() }
                .disabled(!model.isTilemapLayer)
            Divider()
            Toggle("Tile Stamp Mode", isOn: $state.stamp).disabled(!model.isTilemapLayer)
            }
            .disabled(sheet.isOpen)
        }
    }
}
