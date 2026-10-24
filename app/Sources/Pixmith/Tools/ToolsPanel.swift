import SwiftUI

extension Tool {
    /// Toolbar glyph. All tools use the same symbol weight so the column reads as one set.
    var dsIcon: String {
        switch self {
        case .marquee: "rectangle.dashed"
        case .lasso: "lasso"
        case .polygon: "pentagon"
        case .wand: "wand.and.stars"
        case .move: "arrow.up.and.down.and.arrow.left.and.right"
        case .pencil: "pencil"
        case .eraser: "eraser"
        case .spray: "circle.dotted"
        case .fill: "drop.fill"
        case .gradient: "rectangle.lefthalf.filled"
        case .line: "line.diagonal"
        case .rectangle: "rectangle"
        case .ellipse: "circle"
        case .curve: "point.topleft.down.curvedto.point.bottomright.up"
        case .shapePolygon: "hexagon"
        case .contour: "scribble"
        case .text: "textformat"
        case .picker: "eyedropper"
        case .slice: "crop"
        case .blur: "drop.halffull"
        case .jumble: "shuffle"
        }
    }

    /// Name shown in tooltips and the options bar.
    var dsTitle: String {
        switch self {
        case .marquee: "Rectangular Marquee"
        case .lasso: "Lasso"
        case .polygon: "Polygonal Lasso"
        case .wand: "Magic Wand"
        case .move: "Move"
        case .pencil: "Pencil"
        case .eraser: "Eraser"
        case .spray: "Spray"
        case .fill: "Paint Bucket"
        case .gradient: "Gradient"
        case .line: "Line"
        case .rectangle: "Rectangle"
        case .ellipse: "Ellipse"
        case .curve: "Curve"
        case .shapePolygon: "Polygon"
        case .contour: "Contour"
        case .text: "Text"
        case .picker: "Eyedropper"
        case .slice: "Slice"
        case .blur: "Blur"
        case .jumble: "Jumble"
        }
    }

    /// Toolbar groups, in order: select, paint, shapes, other.
    static let groups: [[Tool]] = [
        [.marquee, .lasso, .polygon, .wand, .move],
        [.pencil, .eraser, .spray, .blur, .jumble, .fill, .gradient],
        [.line, .rectangle, .ellipse, .curve, .shapePolygon, .contour],
        [.text, .picker, .slice],
    ]
}

/// Slim vertical rail: icon-only tools grouped by function, then the color chips. On short windows the buttons shrink
/// to 24pt (still a comfortable target) so every tool stays visible without scrolling.
struct ToolsPanel: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        VStack(spacing: 0) {
            ViewThatFits(in: .vertical) {
                tools(size: DS.Metric.toolButton, gap: 2)
                tools(size: DS.Metric.control, gap: 0)
            }
            .frame(maxHeight: .infinity, alignment: .top)
            ColorChips(model: model).padding(.bottom, DS.Space.sm)
        }
        .frame(width: DS.Metric.toolRail)
        .background(DSColor.panel)
        .overlay(alignment: .trailing) { Rectangle().fill(DSColor.separatorSoft).frame(width: 1) }
    }

    private func tools(size: CGFloat, gap: CGFloat) -> some View {
        VStack(spacing: gap) {
            ForEach(Tool.groups.indices, id: \.self) { g in
                if g > 0 { DSRailDivider() }
                ForEach(Tool.groups[g], id: \.self) { tool in
                    ToolButton(symbol: tool.dsIcon, title: tool.dsTitle, shortcut: tool.key, active: model.tool == tool,
                               size: size) { model.tool = tool }
                }
            }
        }
        .padding(.top, 6)
    }
}
