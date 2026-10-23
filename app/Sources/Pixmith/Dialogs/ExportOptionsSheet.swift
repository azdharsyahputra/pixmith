import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

/// Every setting of File > Export…, saved per document path.
struct ExportSettings: Codable, Equatable {
    var format = 3
    var scale = 1
    /// -1: all frames; otherwise a tag index.
    var tag = -1
    var splitTags = false
    var applyDirection = true
    /// 0 visible, 1 selected layer, 2 each layer
    var layers = 0
    /// 0 none, 1 per frame, 2 whole animation
    var trim = 0
    var ignoreEmpty = false
    var layout = 0
    var columns = 4
    var padding = 0
    var border = 0
    var extrude = 0
    var writeJSON = true
    var outputPath = ""

    static func key(for url: URL?) -> String { "exportSettings:" + (url?.path ?? "untitled") }

    static func load(for url: URL?) -> ExportSettings {
        guard let data = UserDefaults.standard.data(forKey: key(for: url)),
              let s = try? JSONDecoder().decode(ExportSettings.self, from: data) else { return ExportSettings() }
        return s
    }

    func save(for url: URL?) {
        if let data = try? JSONEncoder().encode(self) { UserDefaults.standard.set(data, forKey: Self.key(for: url)) }
    }
}

extension SpriteModel {
    /// File > Export…: one sheet for PNG, GIF, APNG, sprite sheet and PNG sequence.
    func showExportOptions() {
        stop()
        presentSheet { dismiss in ExportOptionsSheet(model: self, dismiss: dismiss) }
    }
}

private let formatNames = ["PNG (one frame)", "Animated GIF", "Animated PNG", "Sprite sheet", "PNG sequence"]
private let formatExtensions = ["png", "gif", "png", "png", "png"]

struct ExportOptionsSheet: View {
    @ObservedObject var model: SpriteModel
    let dismiss: () -> Void
    @State private var s: ExportSettings
    @State private var jobs = 1

    init(model: SpriteModel, dismiss: @escaping () -> Void) {
        self.model = model
        self.dismiss = dismiss
        var settings = ExportSettings.load(for: model.fileURL)
        if settings.tag >= model.tagCount { settings.tag = -1 }
        if settings.outputPath.isEmpty { settings.outputPath = Self.defaultPath(model, settings.format) }
        _s = State(initialValue: settings)
    }

    private static func defaultPath(_ m: SpriteModel, _ format: Int) -> String {
        let dir = m.fileURL?.deletingLastPathComponent()
            ?? FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory())
        let base = m.fileURL?.deletingPathExtension().lastPathComponent ?? "Untitled"
        let suffix = format == 3 ? "-sheet" : ""
        return dir.appendingPathComponent("\(base)\(suffix).\(formatExtensions[format])").path
    }

    private var animated: Bool { s.format == 1 || s.format == 2 }

    private let labelWidth: CGFloat = 84

    private func check(_ title: String, _ on: Binding<Bool>) -> some View {
        HStack(spacing: DS.Space.sm) {
            Color.clear.frame(width: labelWidth, height: 1)
            Toggle(title, isOn: on).toggleStyle(.checkbox).font(DS.Font.body)
            Spacer(minLength: 0)
        }
    }

    private var frameChoices: [(value: Int, title: String)] {
        var o: [(value: Int, title: String)] = [(-1, "All frames")]
        for i in 0..<model.tagCount { o.append((i, "Tag: \(model.tag(i)?.name ?? "")")) }
        if model.tagCount > 0 { o.append((-2, "One file per tag")) }
        return o
    }

    private var trimChoices: [(value: Int, title: String)] {
        var o: [(value: Int, title: String)] = [(0, "None"), (1, animated ? "Whole animation" : "Each frame")]
        if !animated { o.append((2, "Whole animation (one rectangle)")) }
        return o
    }

    var body: some View {
        DSDialog(title: "Export", primary: "Export", primaryEnabled: jobs >= 1, width: 500,
                 onPrimary: { run() }, onCancel: { dismiss() }) {
            DSFormRow(label: "Format", labelWidth: labelWidth) {
                DSPopup(selection: $s.format, options: formatNames.indices.map { ($0, formatNames[$0]) }, help: "File format", fill: true)
                    .onChange(of: s.format) { f in
                        // Keep the folder and name, switch the extension.
                        let url = URL(fileURLWithPath: s.outputPath)
                        s.outputPath = url.deletingPathExtension().appendingPathExtension(formatExtensions[f]).path
                    }
            }
            DSFormRow(label: "Scale", labelWidth: labelWidth) {
                DSPopup(selection: $s.scale, options: [1, 2, 3, 4, 6, 8, 16].map { ($0, "\($0)×") }, help: "Size multiplier")
                Spacer(minLength: 0)
            }
            DSFormRow(label: "Frames", labelWidth: labelWidth) {
                DSPopup(selection: Binding(get: { s.splitTags ? -2 : s.tag }, set: {
                    s.splitTags = $0 == -2
                    s.tag = $0 == -2 ? -1 : $0
                }), options: frameChoices, help: "Which frames to write", fill: true)
            }
            check("Apply tag direction (reverse, ping-pong)", $s.applyDirection)
                .disabled(s.tag < 0 && !s.splitTags)
            DSFormRow(label: "Layers", labelWidth: labelWidth) {
                DSPopup(selection: $s.layers, options: [
                    (0, "Visible layers"), (1, "Selected layer only"), (2, "Each layer in its own file"),
                ], help: "Which layers to write", fill: true)
            }
            DSFormRow(label: "Trim", labelWidth: labelWidth) {
                DSPopup(selection: $s.trim, options: trimChoices, help: "Crop transparent borders", fill: true)
                    .onChange(of: animated) { if $0 && s.trim == 2 { s.trim = 1 } }
            }
            check("Skip empty frames", $s.ignoreEmpty)
            if s.format == 3 {
                Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
                DSFormRow(label: "Layout", labelWidth: labelWidth) {
                    DSSegmented(selection: $s.layout, options: [
                        .init(0, title: "Horizontal"), .init(1, title: "Vertical"), .init(2, title: "Grid"),
                    ])
                    if s.layout == 2 {
                        DSNumberField("Columns", value: $s.columns, range: 1...256, width: 108)
                    }
                    Spacer(minLength: 0)
                }
                DSFormRow(label: "Spacing", labelWidth: labelWidth) {
                    DSNumberField("Gap", value: $s.padding, range: 0...64, unit: "px", width: 112, help: "Space between frames")
                    DSNumberField("Border", value: $s.border, range: 0...64, unit: "px", width: 112)
                    DSNumberField("Extrude", value: $s.extrude, range: 0...16, unit: "px", width: 120,
                                  help: "Repeat edge pixels to stop bleeding")
                    Spacer(minLength: 0)
                }
                check("Write JSON metadata", $s.writeJSON)
            }
            Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
            DSFormRow(label: s.format == 4 ? "Base name" : "File", labelWidth: labelWidth) {
                TextField(s.format == 4 ? "Base name" : "File", text: $s.outputPath).dsTextField()
                Button("Choose…") { choose() }.buttonStyle(DSBarButtonStyle())
            }
            DSNote(summary)
        }
        .onChange(of: s) { _ in refreshJobs() }
        .onAppear { refreshJobs() }
    }

    private var summary: String {
        if jobs < 1 { return "These options cannot be exported." }
        if s.format == 4 { return jobs == 1 ? "Writes name-001.png, name-002.png, …" : "Writes \(jobs) numbered sequences." }
        return jobs == 1 ? "Writes one file." : "Writes \(jobs) files, with the layer or tag name added to each file name."
    }

    private func options() -> PixmithExport {
        var o = PixmithExport()
        o.format = UInt32(s.format)
        o.scale = UInt32(s.scale)
        o.tag = Int32(s.tag)
        o.apply_direction = s.applyDirection
        o.layers = UInt32(s.layers)
        o.layer = UInt32(model.layer)
        o.split_tags = s.splitTags
        o.trim = UInt32(s.trim)
        o.ignore_empty = s.ignoreEmpty
        o.frame = UInt32(model.frame)
        o.layout = UInt32(s.layout)
        o.columns = UInt32(s.columns)
        o.padding = UInt32(s.padding)
        o.border = UInt32(s.border)
        o.extrude = UInt32(s.extrude)
        o.write_json = s.writeJSON
        return o
    }

    private func refreshJobs() {
        var o = options()
        jobs = Int(pixmith_export_job_count(model.doc, &o))
    }

    private func choose() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = URL(fileURLWithPath: s.outputPath).lastPathComponent
        panel.directoryURL = URL(fileURLWithPath: s.outputPath).deletingLastPathComponent()
        panel.allowedContentTypes = [s.format == 1 ? .gif : .png]
        if panel.runModal() == .OK, let url = panel.url { s.outputPath = url.path }
    }

    private func run() {
        var o = options()
        // The sequence format wants a base name; the field shows a file name for convenience.
        let path = s.format == 4 ? URL(fileURLWithPath: s.outputPath).deletingPathExtension().path : s.outputPath
        let n = path.withCString { pixmith_export_run(model.doc, &o, $0) }
        s.save(for: model.fileURL)
        if n < 0 {
            model.errorMessage = String(cString: pixmith_last_error())
        } else {
            dismiss()
        }
    }
}
