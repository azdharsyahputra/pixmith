import SwiftUI

/// The context bar: the active tool's options, plus the free transform strip while a transform is active.
struct ContextBar: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        VStack(spacing: 0) {
            DSToolbar { ToolOptionsBar(model: model) }
            TransformBar(model: model)
        }
    }
}

/// Full-width row at the top of the window.
///
/// Tabs on the left (`TabBar` in Documents.swift), zoom and Export on the right.
struct TitleRow: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        DSToolbar {
            TabBar(model: model)
            Spacer(minLength: DS.Space.sm)
            ZoomControl(model: model)
            ExportButton(model: model)
        }
    }
}

/// The current document as one tab-like chip with an unsaved-changes dot.
/// Zoom out, a menu with the current zoom and presets, zoom in.
struct ZoomControl: View {
    @ObservedObject var model: SpriteModel
    private static let presets: [CGFloat] = [1, 2, 4, 8, 12, 16, 24, 32, 48, 64]

    var body: some View {
        HStack(spacing: DS.Space.xxs) {
            IconButton("minus", help: "Zoom out", shortcut: "⌘-") { model.zoom = max(1, model.zoom / 2) }
            Menu {
                ForEach(Self.presets, id: \.self) { z in
                    Button("\(Int(z) * 100)%") { model.zoom = z }
                }
                Divider()
                Button("Zoom to Fit") { ViewSettings.shared.zoomToFit(model) }
                Button("Actual Size") { ViewSettings.shared.actualSize(model) }
            } label: {
                HStack(spacing: 4) {
                    Text("\(String(format: "%.5g", Double(model.zoom) * 100))%")
                        .font(DS.Font.numberLarge)
                        .foregroundStyle(DSColor.text)
                    Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(DSColor.textTertiary)
                }
                .frame(width: 70, height: DS.Metric.control)
                .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
                .contentShape(Rectangle())
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Zoom. Pinch or Command-scroll also zooms.")
            IconButton("plus", help: "Zoom in", shortcut: "⌘=") { model.zoom = min(64, model.zoom * 2) }
        }
    }
}

/// Export menu in the primary-button style. Every export the File menu offers is here too.
struct ExportButton: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Menu {
            Button("Export…") { model.showExportOptions() }
            Divider()
            Button("PNG…") { model.exportPNG(sheet: false) }
            Button("Sprite Sheet…") { model.exportSpriteSheet() }
            Divider()
            Button("Animated GIF…") { model.exportGIF() }
            Button("Animated PNG…") { model.exportAPNG() }
            Button("PNG Sequence…") { model.exportPNGSequence() }
            Divider()
            Button("Godot SpriteFrames…") { GodotExportSheet.present(model: model) }
            Button("Re-export Godot SpriteFrames") { GodotExportSheet.reexport(model: model) }
            Button("Godot TileSet…") { model.exportGodotTileset() }
            Button("Re-export Godot TileSet") { model.reexportGodotTileset() }
                .disabled(!model.hasGodotExportSettings)
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "square.and.arrow.up").font(.system(size: 12, weight: .medium))
                Text("Export").font(DS.Font.bodyLarge.weight(.medium))
            }
            .foregroundStyle(.white)
            .padding(.horizontal, DS.Space.md)
            .frame(height: DS.Metric.iconButton)
            .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(DSColor.accent))
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Export the sprite")
    }
}
