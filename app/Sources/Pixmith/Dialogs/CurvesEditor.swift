import AppKit
import PixmithCore
import SwiftUI

struct CurvePoint: Equatable {
    var x: Int
    var y: Int
}

/// The five editable curves of the Color Curves filter: RGB (master), R, G, B and A.
struct CurveSet: Equatable {
    static let names = ["RGB", "R", "G", "B", "A"]
    static let identity = [CurvePoint(x: 0, y: 0), CurvePoint(x: 255, y: 255)]

    var channels: [[CurvePoint]] = Array(repeating: CurveSet.identity, count: 5)

    /// Parameter block for `PIXMITH_FILTER_CURVES`: per curve a point count, then x, y pairs.
    func encoded() -> [Int32] {
        channels.flatMap { pts in [Int32(pts.count)] + pts.flatMap { [Int32($0.x), Int32($0.y)] } }
    }
}

/// Lookup table of one curve, computed by the core so the preview matches the filter exactly.
private func lut(_ pts: [CurvePoint]) -> [UInt8] {
    var out = [UInt8](repeating: 0, count: 256)
    let flat = pts.flatMap { [Int32($0.x), Int32($0.y)] }
    _ = pixmith_curve_lut(flat, pts.count, &out)
    return out
}

/// Curve editor: click empty space to add a point, drag a point to move it, Option-click a point to remove it.
struct CurvesEditor: View {
    @Binding var curves: CurveSet
    @State private var channel = 0
    @State private var dragging: Int?
    @State private var selected: Int?

    private static let size: CGFloat = 256
    private static let colors: [Color] = [.primary, .red, .green, .blue, .gray]
    private let grab: CGFloat = 10

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            DSSegmented(selection: $channel, options: (0..<5).map { .init($0, title: CurveSet.names[$0]) })
                .onChange(of: channel) { _ in selected = nil }

            Canvas { ctx, size in
                let pts = curves.channels[channel]
                // Quarter grid and the identity diagonal.
                var grid = Path()
                for i in 1..<4 {
                    let v = CGFloat(i) * size.width / 4
                    grid.move(to: CGPoint(x: v, y: 0)); grid.addLine(to: CGPoint(x: v, y: size.height))
                    grid.move(to: CGPoint(x: 0, y: v)); grid.addLine(to: CGPoint(x: size.width, y: v))
                }
                ctx.stroke(grid, with: .color(.secondary.opacity(0.25)), lineWidth: 1)
                var diag = Path()
                diag.move(to: CGPoint(x: 0, y: size.height)); diag.addLine(to: CGPoint(x: size.width, y: 0))
                ctx.stroke(diag, with: .color(.secondary.opacity(0.4)), style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
                // The curve.
                let table = lut(pts)
                var line = Path()
                for x in 0..<256 {
                    let p = CGPoint(x: CGFloat(x), y: size.height - 1 - CGFloat(table[x]))
                    x == 0 ? line.move(to: p) : line.addLine(to: p)
                }
                ctx.stroke(line, with: .color(Self.colors[channel]), lineWidth: 2)
                for (i, p) in pts.enumerated() {
                    let c = CGPoint(x: CGFloat(p.x), y: size.height - 1 - CGFloat(p.y))
                    let r: CGFloat = i == selected ? 5 : 4
                    let dot = Path(ellipseIn: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r))
                    ctx.fill(dot, with: .color(i == selected ? .accentColor : Color(nsColor: .windowBackgroundColor)))
                    ctx.stroke(dot, with: .color(Self.colors[channel]), lineWidth: 1.5)
                }
            }
            .frame(width: Self.size, height: Self.size)
            .background(DSColor.field)
            .clipShape(RoundedRectangle(cornerRadius: DS.Radius.control))
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { drag(to: $0) }
                .onEnded { _ in dragging = nil })

            HStack {
                Button("Remove Point") {
                    if let s = selected, curves.channels[channel].count > 2 {
                        curves.channels[channel].remove(at: s)
                        selected = nil
                    }
                }
                .disabled(selected == nil || curves.channels[channel].count <= 2)
                Spacer()
                Button("Reset Channel") { curves.channels[channel] = CurveSet.identity; selected = nil }
                    .disabled(curves.channels[channel] == CurveSet.identity)
                Button("Reset All") { curves = CurveSet(); selected = nil }
                    .disabled(curves == CurveSet())
            }
            .buttonStyle(DSBarButtonStyle())
        }
    }

    /// Image-space position (y up) of a pointer location in the canvas.
    private func curvePosition(_ p: CGPoint) -> CurvePoint {
        CurvePoint(x: Int(p.x.rounded()).clamped(to: 0...255), y: Int((Self.size - 1 - p.y).rounded()).clamped(to: 0...255))
    }

    private func drag(to value: DragGesture.Value) {
        var pts = curves.channels[channel]
        if dragging == nil {
            let start = value.startLocation
            let hit = pts.indices.min { distance(pts[$0], start) < distance(pts[$1], start) }
            if let hit, distance(pts[hit], start) <= grab {
                if NSEvent.modifierFlags.contains(.option) {
                    if pts.count > 2 { pts.remove(at: hit); curves.channels[channel] = pts; selected = nil }
                    dragging = -1
                    return
                }
                dragging = hit
            } else {
                let p = curvePosition(start)
                guard !pts.contains(where: { $0.x == p.x }) else { dragging = -1; return }
                let at = pts.firstIndex { $0.x > p.x } ?? pts.count
                pts.insert(p, at: at)
                dragging = at
            }
            selected = dragging
        }
        guard let i = dragging, i >= 0, i < pts.count else { return }
        var p = curvePosition(value.location)
        let lo = i > 0 ? pts[i - 1].x + 1 : 0
        let hi = i < pts.count - 1 ? pts[i + 1].x - 1 : 255
        p.x = p.x.clamped(to: lo...max(lo, hi))
        pts[i] = p
        curves.channels[channel] = pts
    }

    private func distance(_ p: CurvePoint, _ loc: CGPoint) -> CGFloat {
        hypot(CGFloat(p.x) - loc.x, (Self.size - 1 - CGFloat(p.y)) - loc.y)
    }
}
