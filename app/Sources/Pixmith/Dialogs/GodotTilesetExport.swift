import AppKit
import PixmithCore
import SwiftUI

/// Export settings, remembered per document (keyed by the document's file path).
struct GodotExportSettings: Codable, Equatable {
    var projectRoot = ""
    var outputPath = ""
    var tileset = 0
    var mode = 0
    var collision = 0
    var margin = 0
    var separation = 0
    var extrude = 0
    var maxTexture = 4096
    var writeScene = false
    var terrainName = ""
    var color: [Double] = [0.3, 0.7, 0.3, 1]

    static func key(for url: URL?) -> String? {
        url.map { "godotExport." + $0.resolvingSymlinksInPath().path }
    }

    static func load(for url: URL?) -> GodotExportSettings? {
        guard let key = key(for: url), let data = UserDefaults.standard.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(GodotExportSettings.self, from: data)
    }

    func save(for url: URL?) {
        guard let key = Self.key(for: url), let data = try? JSONEncoder().encode(self) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }

    /// Calls `body` with a C options struct that is valid for the duration of the call.
    func withOptions<T>(_ body: (UnsafePointer<PixmithGodotOptions>) -> T) -> T {
        terrainName.withCString { name in
            var o = PixmithGodotOptions()
            o.mode = UInt32(mode)
            o.collision = UInt32(collision)
            o.margin = UInt32(max(margin, 0))
            o.separation = UInt32(max(separation, 0))
            o.extrude = UInt32(max(extrude, 0))
            o.max_texture = UInt32(max(maxTexture, 0))
            o.write_scene = writeScene
            let c = (color + [0, 0, 0, 1]).prefix(4).map { Float($0) }
            o.terrain_color = (c[0], c[1], c[2], c[3])
            o.terrain_name = name
            return withUnsafePointer(to: &o) { body($0) }
        }
    }
}

extension SpriteModel {
    /// Opens the export sheet.
    func exportGodotTileset() {
        guard !MainWindow.hasSheet else { return }
        let count = Int(pixmith_tileset_count(doc))
        guard count > 0 else {
            errorMessage = "This sprite has no tileset. Add a tilemap layer first."
            return
        }
        var settings = GodotExportSettings.load(for: fileURL) ?? GodotExportSettings()
        if GodotExportSettings.load(for: fileURL) == nil { settings.tileset = tilesetIndex ?? 0 }
        settings.tileset = min(settings.tileset, count - 1)
        let names = (0..<count).map { "\($0): \(tilesetName($0))" }
        var sheet: NSWindow?
        let close = { if let w = sheet { MainWindow.window?.endSheet(w) } }
        let view = GodotTilesetExportView(model: self, settings: settings, tilesetNames: names, close: close)
        let host = NSHostingController(rootView: view)
        let w = NSWindow(contentViewController: host)
        w.styleMask = [.titled]
        sheet = w
        MainWindow.window?.beginSheet(w)
    }

    /// Repeats the last export of this document with its remembered settings.
    func reexportGodotTileset() {
        guard !MainWindow.hasSheet, let s = GodotExportSettings.load(for: fileURL), !s.outputPath.isEmpty else { return }
        if let message = runGodotExport(s) { errorMessage = message }
    }

    var hasGodotExportSettings: Bool {
        GodotExportSettings.load(for: fileURL).map { !$0.outputPath.isEmpty } ?? false
    }

    /// Returns an error or warning text, or nil on a clean export.
    func runGodotExport(_ s: GodotExportSettings) -> String? {
        let ok = s.projectRoot.withCString { r in
            s.outputPath.withCString { o in
                s.withOptions { pixmith_export_godot_tileset_ex(doc, UInt32(s.tileset), r, o, $0) }
            }
        }
        guard ok else { return lastError }
        s.save(for: fileURL)
        guard let w = pixmith_godot_tileset_warning() else { return nil }
        defer { pixmith_string_free(w) }
        let text = String(cString: w)
        return text.isEmpty ? nil : "Exported with warnings:\n" + text
    }

    func godotPreview(_ s: GodotExportSettings) -> String {
        let p: UnsafeMutablePointer<CChar>? = s.projectRoot.withCString { r in
            s.outputPath.withCString { o in
                s.withOptions { pixmith_godot_tileset_preview(doc, UInt32(s.tileset), r, o, $0) }
            }
        }
        guard let p else { return lastError }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }
}

struct GodotTilesetExportView: View {
    let model: SpriteModel
    let tilesetNames: [String]
    let close: () -> Void
    @State private var s: GodotExportSettings
    @State private var color: Color
    @State private var preview = ""
    @State private var result: String?

    init(model: SpriteModel, settings: GodotExportSettings, tilesetNames: [String], close: @escaping () -> Void) {
        self.model = model
        self.tilesetNames = tilesetNames
        self.close = close
        _s = State(initialValue: settings)
        let c = settings.color + [0, 0, 0, 1]
        _color = State(initialValue: Color(.sRGB, red: c[0], green: c[1], blue: c[2], opacity: c[3]))
    }

    private let labelWidth: CGFloat = 96

    var body: some View {
        DSDialog(title: "Export Tileset to Godot 4",
                 subtitle: "Writes a TileSet .tres and its atlas PNG. Godot 4 only: set Default Texture Filter to Nearest to keep pixels sharp.",
                 primary: "Export", primaryEnabled: ready, width: 580,
                 onPrimary: { export() }, onCancel: { close() }) {
            DSFormRow(label: "Tileset", labelWidth: labelWidth) {
                DSPopup(selection: $s.tileset, options: (0..<tilesetNames.count).map { ($0, tilesetNames[$0]) },
                        help: "Tileset to export", fill: true)
            }
            shapeRows
            DSFormRow(label: "Project folder", labelWidth: labelWidth) {
                TextField("Folder with project.godot", text: $s.projectRoot).dsTextField()
                Button("Choose…") { chooseRoot() }.buttonStyle(DSBarButtonStyle())
            }
            DSFormRow(label: "TileSet file", labelWidth: labelWidth) {
                TextField(".tres inside the project", text: $s.outputPath).dsTextField()
                Button("Choose…") { chooseOutput() }.buttonStyle(DSBarButtonStyle())
            }
            Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
            DSFormRow(label: "Terrain mode", labelWidth: labelWidth) {
                DSPopup(selection: $s.mode, options: [
                    (0, "Match Corners and Sides (blob)"), (1, "Match Corners"), (2, "Match Sides"),
                ], help: "How Godot matches terrain tiles", fill: true)
            }
            DSFormRow(label: "Terrain name", labelWidth: labelWidth) {
                TextField("Default: tileset name", text: $s.terrainName).dsTextField()
                RoundedRectangle(cornerRadius: DS.Radius.field).fill(color)
                    .frame(width: 36, height: DS.Metric.control)
                    .overlay(RoundedRectangle(cornerRadius: DS.Radius.field).stroke(Color.white.opacity(0.12), lineWidth: 1))
                    .overlay(ColorPicker("", selection: $color).labelsHidden().opacity(0.02))
                    .help("Terrain color in Godot's editor")
            }
            DSFormRow(label: "Collision", labelWidth: labelWidth) {
                DSPopup(selection: $s.collision, options: [
                    (0, "None"), (1, model.isoCellSize(of: s.tileset) != nil ? "Full diamond" : "Full tile square"),
                    (2, "Traced from opaque pixels"),
                ], help: "Physics shape for each tile", fill: true)
            }
            Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
            DSFormRow(label: "Atlas", labelWidth: labelWidth) {
                DSNumberField("Margin", value: $s.margin, range: 0...64, unit: "px", width: 116, help: "Space around the atlas")
                DSNumberField("Gap", value: $s.separation, range: 0...64, unit: "px", width: 100, help: "Separation between tiles")
                DSNumberField("Extrude", value: $s.extrude, range: 0...16, unit: "px", width: 120, help: "Repeat edge pixels to stop bleeding")
            }
            DSFormRow(label: "Max texture", labelWidth: labelWidth) {
                DSPopup(selection: $s.maxTexture, options: [1024, 2048, 4096, 8192, 16384].map { ($0, "\($0) px") },
                        help: "Largest atlas page")
                Spacer(minLength: 0)
            }
            HStack(spacing: DS.Space.sm) {
                Color.clear.frame(width: labelWidth, height: 1)
                Toggle("Also write a scene (.tscn) with the tilemap layers painted", isOn: $s.writeScene)
                    .toggleStyle(.checkbox).font(DS.Font.body)
                Spacer(minLength: 0)
            }
            VStack(alignment: .leading, spacing: DS.Space.xs + 2) {
                HStack {
                    Text("Preview").font(DS.Font.section).foregroundStyle(DSColor.text)
                    Spacer(minLength: 0)
                    Button("Preview") {
                        sync()
                        preview = ready ? model.godotPreview(s) : "Choose the project folder and the .tres file first."
                        result = nil
                    }
                    .buttonStyle(DSBarButtonStyle())
                    .help("Show what will be written")
                }
                ScrollView {
                    Text(result ?? (preview.isEmpty ? "Press Preview to see what will be written." : preview))
                        .font(DS.Font.mono)
                        .foregroundStyle(result != nil || !preview.isEmpty ? DSColor.textSecondary : DSColor.textTertiary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(DS.Space.sm + 2)
                }
                .frame(height: 96)
                .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.recessed))
            }
        }
    }

    /// Tile shape of the chosen tileset: the core writes TILE_SHAPE_ISOMETRIC / DIAMOND_DOWN for an isometric one.
    @ViewBuilder
    private var shapeRows: some View {
        let ts = s.tileset
        if ts < tilesetNames.count {
            let (tw, th) = model.tileSize(of: ts)
            if let cell = model.isoCellSize(of: ts) {
                DSFormRow(label: "Tile shape", labelWidth: labelWidth) {
                    DSPopup(selection: .constant(0), options: [(0, "Isometric")], help: "Written as TILE_SHAPE_ISOMETRIC", fill: true)
                    DSPopup(selection: .constant(0), options: [(0, "Diamond Down")], help: "Written as TILE_LAYOUT_DIAMOND_DOWN", fill: true)
                }
                .disabled(true)
                HStack(spacing: DS.Space.sm) {
                    Color.clear.frame(width: labelWidth, height: 1)
                    DSNote("Tile size \(cell.w)×\(cell.h) (the diamond), pictures \(tw)×\(th). Each tile's pivot becomes its texture_origin, and layers become y-sorted TileMapLayer nodes at their grid offset.")
                    Spacer(minLength: 0)
                }
            } else {
                DSFormRow(label: "Tile shape", labelWidth: labelWidth) {
                    Text("Square · \(tw)×\(th)").font(DS.Font.body).foregroundStyle(DSColor.textSecondary)
                    Spacer(minLength: 0)
                }
            }
        }
    }

    private var ready: Bool { !s.projectRoot.isEmpty && !s.outputPath.isEmpty }

    private func sync() {
        let ns = NSColor(color).usingColorSpace(.sRGB) ?? .green
        s.color = [ns.redComponent, ns.greenComponent, ns.blueComponent, ns.alphaComponent].map { Double($0) }
    }

    private func export() {
        sync()
        if let message = model.runGodotExport(s) {
            result = message
            if !message.hasPrefix("Exported with warnings") { return }
        }
        close()
    }

    private func chooseRoot() {
        let open = NSOpenPanel()
        open.canChooseDirectories = true
        open.canChooseFiles = false
        open.message = "Choose the Godot project folder (contains project.godot)"
        if open.runModal() == .OK, let url = open.url { s.projectRoot = url.path }
    }

    private func chooseOutput() {
        let save = NSSavePanel()
        if !s.projectRoot.isEmpty { save.directoryURL = URL(fileURLWithPath: s.projectRoot) }
        save.allowedContentTypes = [.init(filenameExtension: "tres") ?? .data]
        let name = s.tileset < tilesetNames.count ? model.tilesetName(s.tileset) : "tiles"
        save.nameFieldStringValue = "\(name).tres"
        save.message = "Save inside the Godot project. The atlas PNG is written next to it."
        if save.runModal() == .OK, let url = save.url { s.outputPath = url.path }
    }
}

struct GodotTilesetCommands: Commands {
    @ObservedObject var model: SpriteModel

    var body: some Commands {
        CommandGroup(after: .saveItem) {
            Button("Export Tileset to Godot…") { model.exportGodotTileset() }
            Button("Re-export Tileset to Godot") { model.reexportGodotTileset() }
                .disabled(!model.hasGodotExportSettings)
        }
    }
}
