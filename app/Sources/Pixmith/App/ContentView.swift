import SwiftUI

/// Main window. It shows Home at launch, a project's canvases, or the editor (`AppRoute`); sheets, the file drop,
/// the error alert and the window integration sit around all three.
struct ContentView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var route = AppRoute.shared
    @ObservedObject private var themes = ThemeStore.shared

    var body: some View {
        screen
        // A new color theme (Preferences > Appearance) rebuilds the views so every color is resolved again.
        .id(themes.revision)
        .tint(themes.selected == nil ? nil : DSColor.accent)
        .frame(minWidth: 1000, minHeight: 640)
        .background(DSColor.panel)
        .navigationTitle(route.title)
        .pixmithFileDrop(model)
        .pixmithFileHandling(model)
        .sheet(isPresented: $model.showNewSheet) { NewSpriteSheet(model: model) }
        .sheet(item: $route.sheet) { sheet in
            switch sheet {
            case .newProject: NewProjectSheet()
            case .newCanvas(let folder): NewCanvasSheet(folder: folder)
            }
        }
        .filterSheets(model: model)
        .sheet(item: Binding(get: { model.renamingLayer.map(LayerID.init) },
                             set: { model.renamingLayer = $0?.id })) { item in
            RenameSheet(model: model, layer: item.id)
        }
        .sheet(item: $model.toolSheet) { ToolSheetView(model: model, sheet: $0) }
        .textSheet(model: model)
        .alert("Error", isPresented: Binding(get: { model.errorMessage != nil },
                                             set: { if !$0 { model.errorMessage = nil } })) {
            Button("OK") {}
        } message: {
            Text(model.errorMessage ?? "")
        }
    }
}

private extension ContentView {
    @ViewBuilder var screen: some View {
        switch route.screen {
        case .home: HomeView(model: model)
        case .project(let folder): ProjectView(folder: folder).id(folder.path)
        case .editor: EditorScreen(model: model)
        }
    }
}

/// The editor: a full-width title row (project switcher, document tabs, zoom, export) above three columns: tool rail |
/// canvas column | inspector. The canvas column stacks the tool's context bar, the canvas and the timeline.
struct EditorScreen: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        VStack(spacing: 0) {
            // The title row hosts the project switcher, `TabBar(model:)` (document tabs), the zoom control and Export.
            TitleRow(model: model)
            HStack(spacing: 0) {
                ToolsPanel(model: model)
                VStack(spacing: 0) {
                    ContextBar(model: model)
                    CanvasArea(model: model)
                    TimelinePanel(model: model).id(model.activeSlotID)
                }
                SidePanel(model: model).id(model.activeSlotID)
            }
        }
    }
}

private struct LayerID: Identifiable { let id: Int }

/// The canvas with its quiet status chips (sprite size, cursor position) floating in the corner.
struct CanvasArea: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var info = StatusInfo.shared
    @ObservedObject private var settings = ViewSettings.shared

    private var position: String? {
        guard let c = info.cursor, (0..<model.width).contains(c.x), (0..<model.height).contains(c.y) else { return nil }
        return "\(c.x), \(c.y)"
    }

    var body: some View {
        CanvasScroller(model: model)
            .overlay(alignment: .bottomLeading) {
                HStack(spacing: DS.Space.xs + 2) {
                    DSOverlayChip(text: "\(model.width) × \(model.height)")
                    DSOverlayChip(symbol: "cursorarrow", text: position ?? "–")
                    if model.frameDiff, let diff = FrameDiffCache.shared.entry(model) {
                        DSOverlayChip(text: diff.chip)
                    }
                }
                .padding(.leading, DS.Space.md)
                .padding(.bottom, DS.Space.sm + 2)
                .allowsHitTesting(false)
            }
    }
}

struct NewSpriteSheet: View {
    @ObservedObject var model: SpriteModel
    @Environment(\.dismiss) private var dismiss
    @State private var width = Prefs.newWidth
    @State private var height = Prefs.newHeight

    var body: some View {
        DSDialog(title: "New Sprite", primary: "Create", width: 300, onPrimary: {
            model.newSprite(width: min(max(width, 1), 4096), height: min(max(height, 1), 4096))
            dismiss()
        }, onCancel: { dismiss() }) {
            DSFormRow(label: "Width") { DSNumberField("W", value: $width, range: 1...4096, unit: "px", width: 120) }
            DSFormRow(label: "Height") { DSNumberField("H", value: $height, range: 1...4096, unit: "px", width: 120) }
        }
    }
}

struct RenameSheet: View {
    @ObservedObject var model: SpriteModel
    let layer: Int
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""

    var body: some View {
        DSDialog(title: "Rename Layer", primary: "Rename", primaryEnabled: !name.trimmingCharacters(in: .whitespaces).isEmpty,
                 width: 300, onPrimary: { model.rename(layer, to: name); dismiss() }, onCancel: { dismiss() }) {
            TextField("Name", text: $name).dsTextField()
        }
        .onAppear { name = model.layerName(layer) }
    }
}
