import AppKit
import PixmithCore
import SwiftUI

/// File > Export to Godot… / Re-export to Godot (SpriteFrames .tres + atlas PNG, Godot 4 only).
struct GodotExportCommands: Commands {
    @ObservedObject var model: SpriteModel

    var body: some Commands {
        CommandGroup(after: .importExport) {
            Button("Export to Godot…") { GodotExportSheet.present(model: model) }
            Button("Re-export to Godot") { GodotExportSheet.reexport(model: model) }
                .keyboardShortcut("g", modifiers: [.command, .shift])
        }
    }
}

enum GodotExportDefaults {
    static let rootKey = "godotExport.projectRoot"
    static let warnedKey = "godotExport.filterWarningShown"
}

/// Mirrors the JSON understood by core/src/godot/settings.rs.
struct GodotSettings: Equatable {
    var root = ""
    var tres = ""
    var padding = 1
    var extrude = true
    var trim = false
    var maxTexture = 4096
    var scale = 1
    var grid = false
    var perAnimationPNG = false
    var nameTemplate = "{name}"
    /// "visible", "each" or "selected"
    var layerMode = "visible"
    var selectedLayers: [String] = []
    var tagsInclude: [String] = []
    var tagsExclude: [String] = []
    var tscn = false
    var overwriteTscn = false

    init() {}

    init?(json: String) {
        guard let d = json.data(using: .utf8), let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any]
        else { return nil }
        root = o["root"] as? String ?? ""
        tres = o["tres"] as? String ?? ""
        padding = o["padding"] as? Int ?? padding
        extrude = o["extrude"] as? Bool ?? extrude
        trim = o["trim"] as? Bool ?? trim
        maxTexture = o["max_texture"] as? Int ?? maxTexture
        scale = o["scale"] as? Int ?? scale
        grid = (o["packing"] as? String) == "grid"
        perAnimationPNG = o["per_animation_png"] as? Bool ?? false
        nameTemplate = o["name_template"] as? String ?? nameTemplate
        if let names = o["layers"] as? [String] {
            layerMode = "selected"
            selectedLayers = names
        } else {
            layerMode = (o["layers"] as? String) == "each" ? "each" : "visible"
        }
        tagsInclude = o["tags_include"] as? [String] ?? []
        tagsExclude = o["tags_exclude"] as? [String] ?? []
        tscn = o["tscn"] as? Bool ?? false
        overwriteTscn = o["overwrite_tscn"] as? Bool ?? false
    }

    var json: String {
        var o: [String: Any] = [
            "v": 1, "root": root, "tres": tres, "padding": padding, "extrude": extrude, "trim": trim,
            "max_texture": maxTexture, "scale": scale, "packing": grid ? "grid" : "tight",
            "per_animation_png": perAnimationPNG, "name_template": nameTemplate,
            "tags_include": tagsInclude, "tags_exclude": tagsExclude, "tscn": tscn, "overwrite_tscn": overwriteTscn,
        ]
        o["layers"] = layerMode == "selected" ? selectedLayers : layerMode
        let d = (try? JSONSerialization.data(withJSONObject: o, options: [.sortedKeys])) ?? Data("{}".utf8)
        return String(decoding: d, as: UTF8.self)
    }
}

enum GodotCore {
    struct Result {
        var ok: Bool
        var warnings: [String] = []
        var error = ""
        var tres = ""
        var files: [String] = []
    }

    static func take(_ p: UnsafeMutablePointer<CChar>?) -> String {
        guard let p else { return "" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    static func lines(_ s: String) -> [String] { s.split(separator: "\n").map(String.init) }

    static func export(doc: OpaquePointer, json: String) -> Result {
        var w: UnsafeMutablePointer<CChar>?
        let ok = json.withCString { pixmith_export_godot(doc, $0, &w) }
        if ok { return Result(ok: true, warnings: lines(take(w))) }
        return Result(ok: false, error: String(cString: pixmith_last_error()))
    }

    static func preview(doc: OpaquePointer, json: String, png: String) -> Result {
        var t: UnsafeMutablePointer<CChar>?, f: UnsafeMutablePointer<CChar>?, w: UnsafeMutablePointer<CChar>?
        let ok = json.withCString { j in png.withCString { p in pixmith_godot_preview(doc, j, p, &t, &f, &w) } }
        if ok { return Result(ok: true, warnings: lines(take(w)), tres: take(t), files: lines(take(f))) }
        return Result(ok: false, error: String(cString: pixmith_last_error()))
    }

    static func savedSettings(doc: OpaquePointer) -> GodotSettings? {
        guard let p = pixmith_godot_settings_get(doc) else { return nil }
        return GodotSettings(json: take(p))
    }

    /// Stores the settings in the document; marks it modified only when they changed.
    @MainActor static func save(_ s: GodotSettings, model: SpriteModel) {
        let before = pixmith_godot_settings_get(model.doc).map { take($0) }
        guard s.json.withCString({ pixmith_godot_settings_set(model.doc, $0) }) else { return }
        let after = pixmith_godot_settings_get(model.doc).map { take($0) }
        if before != after { model.changed() }
    }
}

@MainActor
final class GodotExportState: ObservableObject {
    @Published var s = GodotSettings()
    @Published var message: String?
    @Published var isError = false
    @Published var done = false
    @Published var previewImage: NSImage?
    @Published var previewText = ""
    @Published var previewFiles: [String] = []
    @Published var previewWarnings: [String] = []
    @Published var previewError: String?
    @Published var showText = false
    let model: SpriteModel
    var close: () -> Void = {}
    let previewPath = NSTemporaryDirectory() + "pixmith-godot-preview-\(ProcessInfo.processInfo.processIdentifier).png"

    init(model: SpriteModel) {
        self.model = model
        if let saved = GodotCore.savedSettings(doc: model.doc) {
            s = saved
        } else {
            s.root = UserDefaults.standard.string(forKey: GodotExportDefaults.rootKey) ?? ""
            if !s.root.isEmpty { s.tres = (s.root as NSString).appendingPathComponent("art/\(spriteName).tres") }
        }
    }

    var spriteName: String { model.fileURL?.deletingPathExtension().lastPathComponent ?? "sprite" }

    var leafLayers: [String] {
        (0..<model.layerCount).filter { !model.isGroup($0) }.map { model.layerName($0) }
    }

    var tagNames: [String] { (0..<model.tagCount).compactMap { model.tag($0)?.name } }

    func chooseRoot() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.message = "Choose the Godot project folder (the one containing project.godot)"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        s.root = url.path
        UserDefaults.standard.set(s.root, forKey: GodotExportDefaults.rootKey)
        s.tres = url.appendingPathComponent("art/\(spriteName).tres").path
    }

    func chooseOutput() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(spriteName).tres"
        if !s.root.isEmpty { panel.directoryURL = URL(fileURLWithPath: s.root) }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        s.tres = url.pathExtension == "tres" ? url.path : url.appendingPathExtension("tres").path
    }

    func refreshPreview() {
        try? FileManager.default.removeItem(atPath: previewPath)
        guard !s.root.isEmpty, !s.tres.isEmpty else {
            previewError = "Choose the project folder and the output file to see a preview."
            previewImage = nil
            previewText = ""
            return
        }
        let r = GodotCore.preview(doc: model.doc, json: s.json, png: previewPath)
        if r.ok {
            previewError = nil
            previewText = r.tres
            previewFiles = r.files
            previewWarnings = r.warnings
            previewImage = NSImage(contentsOfFile: previewPath)
        } else {
            previewError = r.error
            previewImage = nil
            previewText = ""
            previewFiles = []
        }
    }

    func export() {
        let r = GodotCore.export(doc: model.doc, json: s.json)
        if r.ok {
            GodotCore.save(s, model: model)
            UserDefaults.standard.set(s.root, forKey: GodotExportDefaults.rootKey)
            let shown = UserDefaults.standard.bool(forKey: GodotExportDefaults.warnedKey)
            UserDefaults.standard.set(true, forKey: GodotExportDefaults.warnedKey)
            let notes = r.warnings.filter { !(shown && $0.contains("Nearest")) }
            message = (["Exported."] + notes).joined(separator: "\n\n")
            isError = false
            done = true
        } else {
            message = r.error
            isError = true
        }
    }
}

/// A small notice row: a symbol and wrapped text in a quiet or warning tone.
struct DSNotice: View {
    enum Tone { case info, warning, error }
    let text: String
    var tone = Tone.info

    private var color: Color {
        switch tone {
        case .info: return DSColor.textSecondary
        case .warning: return Color.orange
        case .error: return DSColor.destructive
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: DS.Space.xs + 2) {
            if tone != .info {
                Image(systemName: tone == .error ? "xmark.octagon.fill" : "exclamationmark.triangle.fill")
                    .font(.system(size: 11)).foregroundStyle(color).padding(.top, 1)
            }
            Text(text).font(DS.Font.caption).foregroundStyle(tone == .info ? DSColor.textSecondary : DSColor.text)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
    }
}

/// A titled block of an export form: a quiet heading, then rows, with a hairline above all but the first.
struct DSFormGroup<Content: View>: View {
    let title: String
    var first = false
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            if !first { Rectangle().fill(DSColor.separatorSoft).frame(height: 1).padding(.bottom, DS.Space.xs) }
            Text(title).font(DS.Font.section).foregroundStyle(DSColor.text)
            content
        }
    }
}

struct GodotExportView: View {
    @ObservedObject var state: GodotExportState
    private let sizes = [2048, 4096, 8192, 16384]
    private let labelWidth: CGFloat = 96

    private func included(_ tag: String) -> Binding<Bool> {
        Binding(
            get: { !state.s.tagsExclude.contains(tag) && (state.s.tagsInclude.isEmpty || state.s.tagsInclude.contains(tag)) },
            set: { on in
                // Stored as an exclude list so that new tags are exported by default.
                state.s.tagsInclude = []
                state.s.tagsExclude.removeAll { $0 == tag }
                if !on { state.s.tagsExclude.append(tag) }
            }
        )
    }

    private func layerSelected(_ name: String) -> Binding<Bool> {
        Binding(
            get: { state.s.selectedLayers.contains(name) },
            set: { on in
                state.s.selectedLayers.removeAll { $0 == name }
                if on { state.s.selectedLayers.append(name) }
            }
        )
    }

    var body: some View {
        DSDialog(title: "Export to Godot 4",
                 subtitle: "Writes a SpriteFrames .tres (format 3) and an atlas PNG. Godot 4 only: Godot 3 projects use another format and are not supported.",
                 primary: "Export", primaryEnabled: !state.s.root.isEmpty && !state.s.tres.isEmpty,
                 cancelTitle: state.done ? "Close" : "Cancel", width: 1000,
                 onPrimary: { state.export() }, onCancel: { state.close() }) {
            HStack(alignment: .top, spacing: DS.Space.lg) {
                ScrollView { form.padding(.trailing, DS.Space.md) }.frame(width: 450)
                DSDivider(axis: .vertical)
                preview
            }
            .frame(height: 548)
            if !UserDefaults.standard.bool(forKey: GodotExportDefaults.warnedKey) {
                DSNotice(text: "Pixel art looks blurry in Godot by default. Set Project Settings > Rendering > Textures > Default Texture Filter = Nearest.",
                         tone: .warning)
            }
            if let m = state.message {
                DSNotice(text: m, tone: state.isError ? .error : .info).textSelection(.enabled)
            }
        }
        .task(id: state.s) {
            try? await Task.sleep(nanoseconds: 150_000_000)
            if !Task.isCancelled { state.refreshPreview() }
        }
    }

    private func pathRow(_ label: String, _ prompt: String, _ text: Binding<String>, choose: (() -> Void)? = nil) -> some View {
        DSFormRow(label: label, labelWidth: labelWidth) {
            TextField(prompt, text: text).dsTextField()
            if let choose {
                Button("Choose…", action: choose).buttonStyle(DSBarButtonStyle())
            }
        }
    }

    private func indented<C: View>(@ViewBuilder _ content: () -> C) -> some View {
        HStack(alignment: .top, spacing: DS.Space.sm) {
            Color.clear.frame(width: labelWidth, height: 1)
            VStack(alignment: .leading, spacing: DS.Space.sm - 2) { content() }
            Spacer(minLength: 0)
        }
    }

    private func check(_ title: String, _ on: Binding<Bool>) -> some View {
        Toggle(title, isOn: on).toggleStyle(.checkbox).font(DS.Font.body)
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: DS.Space.md) {
            DSFormGroup(title: "Destination", first: true) {
                pathRow("Project folder", "Folder with project.godot", $state.s.root) { state.chooseRoot() }
                pathRow("Output file", "SpriteFrames .tres inside the project", $state.s.tres) { state.chooseOutput() }
                pathRow("File name", "{name} {layer}", $state.s.nameTemplate)
            }
            DSFormGroup(title: "Content") {
                DSFormRow(label: "Layers", labelWidth: labelWidth) {
                    DSPopup(selection: $state.s.layerMode, options: [
                        ("visible", "Visible layers"), ("each", "Each layer as its own file"), ("selected", "Selected layers"),
                    ], help: "Which layers to export", fill: true)
                }
                if state.s.layerMode == "selected" {
                    indented { ForEach(state.leafLayers, id: \.self) { check($0, layerSelected($0)) } }
                }
                if !state.tagNames.isEmpty {
                    DSFormRow(label: "Animations", labelWidth: labelWidth) {
                        Text("One per tag").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                        Spacer(minLength: 0)
                    }
                    indented { ForEach(state.tagNames, id: \.self) { check($0, included($0)) } }
                } else {
                    indented { DSNote("No tags: one \"default\" animation with all frames.") }
                }
            }
            DSFormGroup(title: "Atlas") {
                DSFormRow(label: "Packing", labelWidth: labelWidth) {
                    DSSegmented(selection: $state.s.grid, options: [
                        .init(false, title: "Tight (deduplicated)"), .init(true, title: "Grid"),
                    ])
                    Spacer(minLength: 0)
                }
                DSFormRow(label: "Spacing", labelWidth: labelWidth) {
                    DSNumberField("Padding", value: $state.s.padding, range: 0...16, unit: "px", width: 128)
                    DSNumberField("Scale", value: $state.s.scale, range: 1...8, unit: "×", width: 96)
                    Spacer(minLength: 0)
                }
                DSFormRow(label: "Max texture", labelWidth: labelWidth) {
                    DSPopup(selection: $state.s.maxTexture, options: sizes.map { ($0, "\($0) px") }, help: "Largest atlas page")
                    Spacer(minLength: 0)
                }
                if state.s.maxTexture > 4096 {
                    indented { DSNotice(text: "Above 4096px some mobile GPUs cannot load the texture.", tone: .warning) }
                }
                indented {
                    check("Extrude edges 1px (stops texture bleeding)", $state.s.extrude)
                    check("Trim transparent borders (AtlasTexture margin)", $state.s.trim)
                    check("Separate PNG per animation", $state.s.perAnimationPNG)
                }
            }
            DSFormGroup(title: "Scene") {
                indented {
                    check("Also write a .tscn with an AnimatedSprite2D", $state.s.tscn)
                        .help("The scene uses the nearest texture filter")
                    check("Replace the .tscn if it exists", $state.s.overwriteTscn).disabled(!state.s.tscn)
                }
            }
        }
    }

    private var preview: some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            DSSegmented(selection: $state.showText, options: [.init(false, title: "Atlas"), .init(true, title: ".tres")])
            Group {
                if state.showText {
                    ScrollView([.vertical, .horizontal]) {
                        Text(state.previewText.isEmpty ? " " : state.previewText).font(DS.Font.mono).foregroundStyle(DSColor.textSecondary)
                            .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(DS.Space.sm + 2)
                    }
                    .background(RoundedRectangle(cornerRadius: DS.Radius.panel).fill(DSColor.recessed))
                } else if let img = state.previewImage {
                    ZStack {
                        DSChecker(cell: 8)
                        Image(nsImage: img).interpolation(.none).resizable().scaledToFit().padding(DS.Space.sm)
                    }
                    .clipShape(RoundedRectangle(cornerRadius: DS.Radius.panel))
                    .overlay(RoundedRectangle(cornerRadius: DS.Radius.panel).stroke(DSColor.separator, lineWidth: 1))
                } else {
                    VStack(spacing: DS.Space.sm) {
                        Image(systemName: "photo.on.rectangle.angled").font(.system(size: 24)).foregroundStyle(DSColor.textTertiary)
                        Text(state.previewError ?? "The atlas appears here.")
                            .font(DS.Font.body).foregroundStyle(DSColor.textSecondary).multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(DS.Space.lg)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(RoundedRectangle(cornerRadius: DS.Radius.panel).fill(DSColor.recessed))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            if let e = state.previewError, state.previewImage != nil || state.showText {
                DSNotice(text: e, tone: .error)
            }
            if !state.showText, let img = state.previewImage {
                Text("\(Int(img.size.width)) × \(Int(img.size.height)) px · first page").font(DS.Font.number)
                    .foregroundStyle(DSColor.textTertiary)
            }
            if !state.previewFiles.isEmpty {
                Text("Files: " + state.previewFiles.map { ($0 as NSString).lastPathComponent }.joined(separator: ", "))
                    .font(DS.Font.caption).foregroundStyle(DSColor.textTertiary).lineLimit(2)
            }
            ForEach(state.previewWarnings.filter { !$0.contains("Nearest") }, id: \.self) {
                DSNotice(text: $0, tone: .warning)
            }
        }
        .frame(maxWidth: .infinity)
    }
}

enum GodotExportSheet {
    @MainActor static func present(model: SpriteModel) {
        let state = GodotExportState(model: model)
        let host = NSHostingController(rootView: GodotExportView(state: state))
        let window = NSWindow(contentViewController: host)
        window.styleMask = [.titled]
        state.close = {
            if let parent = window.sheetParent { parent.endSheet(window) } else { window.close() }
        }
        if let parent = NSApp.keyWindow {
            parent.beginSheet(window)
        } else {
            window.makeKeyAndOrderFront(nil)
        }
    }

    /// Repeats the last export saved in the document (Cmd-Shift-G); opens the sheet when there is none.
    @MainActor static func reexport(model: SpriteModel) {
        guard let s = GodotCore.savedSettings(doc: model.doc), !s.root.isEmpty, !s.tres.isEmpty else {
            return present(model: model)
        }
        let r = GodotCore.export(doc: model.doc, json: s.json)
        if !r.ok {
            model.errorMessage = r.error
            return
        }
        let shown = UserDefaults.standard.bool(forKey: GodotExportDefaults.warnedKey)
        UserDefaults.standard.set(true, forKey: GodotExportDefaults.warnedKey)
        let notes = r.warnings.filter { !$0.contains("Nearest") || !shown }
        if notes.isEmpty {
            NSSound(named: "Pop")?.play()
            return
        }
        let alert = NSAlert()
        alert.messageText = "Exported to Godot"
        alert.informativeText = notes.joined(separator: "\n\n")
        alert.runModal()
    }
}
