import AppKit
import PixmithCore

/// What the custom grid draws: square cells, or the diamond cells of an isometric (2:1) tile map.
enum GridType: String, CaseIterable, Identifiable {
    case rectangular, isometric

    var id: String { rawValue }
    var title: String { self == .rectangular ? "Rectangular" : "Isometric" }
}

/// Isometric grid geometry, in sprite pixels. One cell is a diamond that fits a `w` x `h` box whose top-left is
/// (`x` + (i - j) * w / 2, `y` + (i + j) * h / 2) for the cell (i, j); `x`, `y` is therefore where the box of cell (0, 0)
/// starts. The diamond's corners are (cx, cy -/+ h/2) and (cx -/+ w/2, cy) around the box center (cx, cy).
///
/// Every pixel belongs to exactly one cell: the one whose diamond contains the pixel center (a center exactly on an
/// edge goes to the cell below/right of it). A cell holds w * h / 2 pixels; for 32x16 its rows are 2, 6, ... 30, 30,
/// ... 6, 2 pixels wide, so the outline steps 2 pixels across per 1 down.
struct IsoGrid: Equatable {
    var w = 32, h = 16, x = 0, y = 0

    /// Tile sizes are even and at least 4.
    static func size(_ v: Int) -> Int { max(4, v - (v & 1)) }

    init(w: Int = 32, h: Int = 16, x: Int = 0, y: Int = 0) {
        (self.w, self.h, self.x, self.y) = (Self.size(w), Self.size(h), x, y)
    }

    static var current: IsoGrid {
        let s = ViewSettings.shared
        return IsoGrid(w: s.isoGridW, h: s.isoGridH, x: s.isoGridX, y: s.isoGridY)
    }

    private static func floorDiv(_ a: Int, _ b: Int) -> Int {
        let q = a / b
        return (a % b != 0 && (a < 0) != (b < 0)) ? q - 1 : q
    }

    /// The cell (i, j) that owns pixel (px, py). Exact integer math on doubled coordinates.
    func cell(atX px: Int, y py: Int) -> (i: Int, j: Int) {
        let ax = 2 * (px - x) + 1 - w, ay = 2 * (py - y) + 1 - h
        let den = 2 * w * h
        return (Self.floorDiv(ax * h + ay * w + w * h, den), Self.floorDiv(ay * w - ax * h + w * h, den))
    }

    /// Top-left of the cell's w x h box.
    func box(_ c: (i: Int, j: Int)) -> (x: Int, y: Int) {
        (x + (c.i - c.j) * (w / 2), y + (c.i + c.j) * (h / 2))
    }

    /// The pixels of a cell as rows `(y, left, right)`, top to bottom (not clipped to any canvas).
    func rows(_ c: (i: Int, j: Int)) -> [(y: Int, l: Int, r: Int)] {
        let b = box(c)
        var out: [(y: Int, l: Int, r: Int)] = []
        for py in b.y..<b.y + h {
            var l = Int.max, r = Int.min
            for px in b.x..<b.x + w where cell(atX: px, y: py) == c {
                l = min(l, px)
                r = max(r, px)
            }
            if l <= r { out.append((py, l, r)) }
        }
        return out
    }

    /// A closed path (x, y pairs) whose polygon fill in the core (pixel-center sampled, outline pixels included) is
    /// exactly `rows`: every edge runs inside the rows, so the outline adds no pixel outside them.
    static func path(of rows: [(y: Int, l: Int, r: Int)]) -> [Int32] {
        var pts: [Int32] = []
        func add(_ x: Int, _ y: Int) { pts += [Int32(truncatingIfNeeded: x), Int32(truncatingIfNeeded: y)] }
        // Down the right side: along the row to the narrower neighbor's edge, then straight down into it.
        for k in 0..<rows.count {
            add(rows[k].r, rows[k].y)
            if k + 1 < rows.count {
                let m = min(rows[k].r, rows[k + 1].r)
                add(m, rows[k].y)
                add(m, rows[k + 1].y)
            }
        }
        // And back up the left side.
        for k in stride(from: rows.count - 1, through: 0, by: -1) {
            add(rows[k].l, rows[k].y)
            if k > 0 {
                let m = max(rows[k].l, rows[k - 1].l)
                add(m, rows[k].y)
                add(m, rows[k - 1].y)
            }
        }
        return pts
    }

    // MARK: Drawing

    /// Fills the grid lines as pixels (2 across per 1 down for a 2:1 tile) over the part of `area` (the sprite, in
    /// view points) that touches `dirty`. One path, so crossings do not darken.
    func stroke(_ ctx: CGContext, in area: CGRect, dirty: CGRect, zoom z: CGFloat, color: CGColor) {
        let vis = area.intersection(dirty.insetBy(dx: -1, dy: -1))
        guard !vis.isNull, !vis.isEmpty, z > 0 else { return }
        let px0 = Int((vis.minX / z).rounded(.down)), px1 = Int((vis.maxX / z).rounded(.up))
        let py0 = Int((vis.minY / z).rounded(.down)), py1 = Int((vis.maxY / z).rounded(.up))
        let s = Double(w) / Double(h)
        let half = max(s, 1) / 2
        let path = CGMutablePath()
        for py in py0..<py1 {
            let dy = Double(py) + 0.5 - Double(y)
            for dir in [-1.0, 1.0] {
                // The line n of this family crosses the middle of the pixel row at xc = x + w * (n + 1/2) + shift.
                let shift = dir * s * dy
                let lo = Int(((Double(px0) - 1 - half - Double(x) - shift) / Double(w) - 0.5).rounded(.up))
                let hi = Int(((Double(px1) + 1 + half - Double(x) - shift) / Double(w) - 0.5).rounded(.down))
                guard lo <= hi else { continue }
                for n in lo...hi {
                    let xc = Double(x) + Double(w) * (Double(n) + 0.5) + shift
                    let k0 = Int((xc - half - 0.5).rounded(.up)), k1 = Int((xc + half - 0.5).rounded(.up)) - 1
                    guard k1 >= k0 else { continue }
                    path.addRect(CGRect(x: CGFloat(k0) * z, y: CGFloat(py) * z, width: CGFloat(k1 - k0 + 1) * z, height: z))
                }
            }
        }
        ctx.saveGState()
        ctx.clip(to: area)
        ctx.setFillColor(color)
        ctx.addPath(path)
        ctx.fillPath()
        ctx.restoreGState()
    }

    /// The pixel the last canvas click landed on (Select > Isometric Tile uses it when the cursor is not over the canvas).
    static var lastClick: (x: Int, y: Int)?
}

extension SpriteModel {
    /// Select > Isometric Tile: the diamond cell under the cursor (when it is over the canvas), else under the last
    /// canvas click, as an exact mask (replaces the selection). Returns false when there is no such pixel.
    @discardableResult
    func selectIsometricTile() -> Bool {
        guard let p = StatusInfo.shared.cursor ?? IsoGrid.lastClick else { return false }
        return selectIsometricTile(at: (p.x, p.y))
    }

    @discardableResult
    func selectIsometricTile(at p: (Int, Int)) -> Bool {
        guard p.0 >= 0, p.1 >= 0, p.0 < width, p.1 < height else { return false }
        stop()
        let grid = IsoGrid.active(for: self)
        let pts = IsoGrid.path(of: grid.rows(grid.cell(atX: p.0, y: p.1)))
        pixmith_sel_begin(doc, 0)
        pixmith_sel_poly(doc, pts, pts.count / 2)
        pixmith_sel_end(doc)
        labelLastStep("Select Isometric Tile")
        changed()
        return hasSelection
    }
}
