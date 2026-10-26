import AppKit
import SwiftUI

/// Floating panel with onion skin, frame difference, tiling, grid and reference image options (View > View Options…).
final class ViewOptionsController {
    static let shared = ViewOptionsController()
    private var panel: NSPanel?

    func show() {
        if panel == nil {
            let p = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 380, height: 560),
                            styleMask: [.titled, .closable, .utilityWindow],
                            backing: .buffered, defer: false)
            p.title = "View Options"
            p.isFloatingPanel = true
            p.hidesOnDeactivate = false
            p.isReleasedWhenClosed = false
            p.contentView = NSHostingView(rootView: ViewOptionsView(model: SpriteModel.shared))
            p.center()
            panel = p
        }
        panel?.makeKeyAndOrderFront(nil)
    }
}

struct ViewOptionsView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var s = ViewSettings.shared

    init(model: SpriteModel) { self.model = model }

    var body: some View {
        Form {
            Section("Onion Skin") {
                Toggle("Enabled", isOn: $model.onionSkin)
                Stepper("Previous frames: \(s.onionPast)", value: $s.onionPast, in: 0...3)
                Stepper("Next frames: \(s.onionFuture)", value: $s.onionFuture, in: 0...3)
                slider("Opacity", $s.onionOpacity, 0.05...0.8)
                slider("Falloff", $s.onionFalloff, 0.1...1)
                Toggle("Tint past red, future blue", isOn: $s.onionTint)
            }
            Section("Frame Difference") {
                Toggle("Enabled", isOn: $model.frameDiff)
                slider("Opacity", $s.diffOpacity, 0.1...1)
                Toggle("Active layer only", isOn: $s.diffActiveLayer)
                Toggle("Compare first frame with last", isOn: $s.diffWrap)
            }
            Section("Tiled Mode") {
                Picker("Mode", selection: $s.tiled) {
                    ForEach(TiledMode.allCases) { Text($0.title).tag($0) }
                }
            }
            Section("Pixel Grid") {
                Toggle("Show grid", isOn: $model.showGrid)
                ColorPicker("Color", selection: $s.gridColor, supportsOpacity: false)
                slider("Opacity", $s.gridOpacity, 0.02...1)
                Stepper("Show at zoom ≥ \(s.gridMinZoom)x", value: $s.gridMinZoom, in: 1...64)
            }
            Section("Custom Grid") {
                Toggle("Show custom grid", isOn: $s.customGrid)
                Picker("Type", selection: $s.customGridType) {
                    ForEach(GridType.allCases) { Text($0.title).tag($0) }
                }
                if s.customGridType == .isometric {
                    HStack {
                        TextField("Tile W", value: $s.isoGridW, format: .number)
                        TextField("Tile H", value: $s.isoGridH, format: .number)
                    }
                    HStack {
                        TextField("X offset", value: $s.isoGridX, format: .number)
                        TextField("Y offset", value: $s.isoGridY, format: .number)
                    }
                    Text("Diamond cells of an isometric tile (even sizes, at least 4; 32x16 is 2:1). Select > Isometric Tile selects the cell under the cursor.")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    HStack {
                        TextField("W", value: $s.customGridW, format: .number)
                        TextField("H", value: $s.customGridH, format: .number)
                    }
                    HStack {
                        TextField("X offset", value: $s.customGridX, format: .number)
                        TextField("Y offset", value: $s.customGridY, format: .number)
                    }
                }
                ColorPicker("Color", selection: $s.customGridColor, supportsOpacity: false)
                slider("Opacity", $s.customGridOpacity, 0.05...1)
            }
            Section("Reference Image") {
                HStack {
                    Text(s.referenceImage == nil ? "None" : s.referenceName).lineLimit(1).truncationMode(.middle)
                    Spacer()
                    Button("Load…") { s.chooseReference() }
                    Button("Clear") { s.clearReference() }.disabled(s.referenceImage == nil)
                }
                Toggle("Show", isOn: $s.showReference)
                slider("Opacity", $s.referenceOpacity, 0.05...1)
                Picker("Placement", selection: $s.referencePlacement) {
                    ForEach(ReferencePlacement.allCases) { Text($0.title).tag($0) }
                }
                HStack {
                    TextField("X", value: $s.referenceX, format: .number)
                    TextField("Y", value: $s.referenceY, format: .number)
                }
                slider("Scale %", $s.referenceScale, 10...800)
            }
        }
        .formStyle(.grouped)
        .frame(width: 380)
    }

    private func slider(_ title: String, _ value: Binding<Double>, _ range: ClosedRange<Double>) -> some View {
        HStack {
            Text(title)
            Slider(value: value, in: range)
        }
    }
}
