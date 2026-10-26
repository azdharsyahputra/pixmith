import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

// WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC (core/src/formats/raster, pixmith_formats.h).
//
// Import: File > Open, drag and drop and File > Import > Image… take any of these files as a new, unsaved sprite
// (the core detects the format from the content). An icon with several sizes asks which one to open.
// Export: the formats are entries of the File > Export… sheet (format numbers 5 to 11 of `PixmithExport.format`), each
// with its own options, saved per document next to the sheet's other settings.

enum ImageFormats {
    /// Extensions that Open and drag and drop treat as an import.
    static let importExtensions: Set<String> = [
        "webp", "jpg", "jpeg", "jpe", "jfif", "bmp", "dib", "tga", "icb", "vda", "vst", "ico", "cur", "pcx", "flc", "fli",
    ]

    static func isImage(_ url: URL) -> Bool { importExtensions.contains(url.pathExtension.lowercased()) }

    static var openTypes: [UTType] { importExtensions.sorted().compactMap { UTType(filenameExtension: $0) } }

    struct ExportFormat: Identifiable {
        let id: Int
        let title: String
        let ext: String
    }

    /// The export formats this adds to the sheet: the core's format number, the name in the popup, the file extension.
    static let exportFormats: [ExportFormat] = [
        ExportFormat(id: 5, title: "WebP", ext: "webp"), ExportFormat(id: 6, title: "JPEG", ext: "jpg"),
        ExportFormat(id: 7, title: "BMP", ext: "bmp"), ExportFormat(id: 8, title: "TGA", ext: "tga"),
        ExportFormat(id: 9, title: "ICO (Windows icon)", ext: "ico"), ExportFormat(id: 10, title: "PCX", ext: "pcx"),
        ExportFormat(id: 11, title: "FLI/FLC animation", ext: "flc"),
    ]

    static let iconSizes = [16, 24, 32, 48, 64, 128, 256]
}

// MARK: - Import

extension SpriteModel {
    /// The sizes (larger side of each image, ascending) of an icon file, nil for anything else.
    static func iconSizes(_ url: URL) -> [Int]? {
        guard let p = url.path.withCString({ pixmith_image_info($0) }) else { return nil }
        defer { pixmith_string_free(p) }
        guard let info = try? JSONSerialization.jsonObject(with: Data(String(cString: p).utf8)) as? [String: Any],
              let sizes = info["sizes"] as? [Int] else { return nil }
        return sizes
    }

    /// Asks which size of an icon to open. Returns nil when cancelled.
    func askIconSize(_ sizes: [Int], file: String) -> Int? {
        let alert = NSAlert()
        alert.messageText = "Which size of the icon do you want to open?"
        alert.informativeText = "“\(file)” holds \(sizes.count) sizes of the same icon. The one you choose opens as a new sprite."
        let popup = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 240, height: 26))
        for s in sizes.reversed() {
            popup.addItem(withTitle: "\(s) × \(s)" + (s == sizes.last ? "  (largest)" : ""))
            popup.lastItem?.tag = s
        }
        alert.accessoryView = popup
        alert.addButton(withTitle: "Open")
        alert.addButton(withTitle: "Cancel")
        guard Self.alertRunner(alert) == .alertFirstButtonReturn else { return nil }
        return popup.selectedItem?.tag
    }

    /// Reads an image file into a new document handle (null on failure; see `lastError`). `iconSize` picks one size of an icon.
    func readImage(_ url: URL, iconSize: Int? = nil) -> OpaquePointer? {
        if let iconSize {
            return url.path.withCString { p in "{\"ico_size\": \(iconSize)}".withCString { pixmith_import_image_ex(p, $0) } }
        }
        return url.path.withCString { pixmith_import_image($0) }
    }

    /// Opens a WebP, JPEG, BMP, TGA, ICO, PCX or FLI/FLC file as a new sprite.
    func importImage(_ url: URL) {
        var size: Int?
        if let sizes = Self.iconSizes(url), sizes.count > 1 {
            guard let chosen = askIconSize(sizes, file: url.lastPathComponent) else { return }
            size = chosen
        }
        adopt(readImage(url, iconSize: size), source: url)
        RecentFiles.shared.add(url)
    }

    /// File > Import > Image… (any format the core reads, PNG and GIF included).
    func importImageAsSprite() {
        guard !MainWindow.hasSheet else { return }
        let panel = NSOpenPanel()
        panel.allowedContentTypes = ImageFormats.openTypes + [.png, .gif]
        panel.message = "Choose an image or animation to open as a new sprite."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        importImage(url)
    }

    /// File > Export > <format>…: the Export sheet with that format chosen.
    func showExportOptions(format: Int) {
        stop()
        presentSheet { dismiss in ExportOptionsSheet(model: self, dismiss: dismiss, initialFormat: format) }
    }
}

// MARK: - Export options

/// The per-format settings of the Export sheet for the formats of this file.
struct ImageExportOptions: Codable, Equatable {
    /// JPEG quality, and WebP quality when lossy.
    var quality = 90
    var lossless = true
    var animated = true
    /// "#rrggbb": what transparent pixels turn into in formats without alpha.
    var background = "#ffffff"
    /// BMP, TGA, PCX: 0 chooses, otherwise 8, 24 or 32 bits.
    var bits = 0
    var rle = true
    /// ICO sizes; empty means the sprite's own size.
    var icoSizes: [Int] = []

    static func key(for url: URL?) -> String { "imageExport:" + (url?.path ?? "untitled") }

    static func load(for url: URL?) -> ImageExportOptions {
        guard let data = UserDefaults.standard.data(forKey: key(for: url)),
              let o = try? JSONDecoder().decode(ImageExportOptions.self, from: data) else { return ImageExportOptions() }
        return o
    }

    func save(for url: URL?) {
        if let data = try? JSONEncoder().encode(self) { UserDefaults.standard.set(data, forKey: Self.key(for: url)) }
    }

    /// The options object of `pixmith_export_run_ex`.
    var json: String {
        var d: [String: Any] = [
            "quality": quality, "lossless": lossless, "animated": animated, "background": background, "bits": bits,
            "rle": rle,
        ]
        if !icoSizes.isEmpty { d["ico_sizes"] = icoSizes.sorted() }
        guard let data = try? JSONSerialization.data(withJSONObject: d), let text = String(data: data, encoding: .utf8) else { return "{}" }
        return text
    }

    var backgroundColor: Color {
        get { Color(hexString: background) }
        set { background = newValue.hexString }
    }
}

extension Color {
    init(hexString: String) {
        let h = hexString.trimmingCharacters(in: CharacterSet(charactersIn: "# "))
        let v = UInt32(h, radix: 16) ?? 0xFFFFFF
        self.init(.sRGB, red: Double((v >> 16) & 255) / 255, green: Double((v >> 8) & 255) / 255,
                  blue: Double(v & 255) / 255, opacity: 1)
    }

    var hexString: String {
        let c = NSColor(self).usingColorSpace(.sRGB) ?? .white
        let r = Int((c.redComponent * 255).rounded()), g = Int((c.greenComponent * 255).rounded())
        let b = Int((c.blueComponent * 255).rounded())
        return String(format: "#%02x%02x%02x", r, g, b)
    }
}

extension SpriteModel {
    /// Writes the current frame (or the animation, for WebP and FLC) with the same settings the Export sheet would use.
    /// Used by the QA scenarios and by anything that wants a quick image export without the sheet.
    @discardableResult
    func exportImage(to url: URL, scale: Int = 1, options: ImageExportOptions = ImageExportOptions(), allFrames: Bool = false) -> Bool {
        let ok = url.path.withCString { path in
            options.json.withCString { json in
                pixmith_export_image(doc, allFrames ? UInt32.max : UInt32(frame), UInt32(scale), path, json)
            }
        }
        if !ok { errorMessage = lastError }
        return ok
    }
}

/// The rows of the Export sheet that belong to formats 5 to 11.
struct ImageFormatOptions: View {
    let format: Int
    @Binding var o: ImageExportOptions
    let labelWidth: CGFloat

    private func check(_ title: String, _ on: Binding<Bool>) -> some View {
        HStack(spacing: DS.Space.sm) {
            Color.clear.frame(width: labelWidth, height: 1)
            Toggle(title, isOn: on).toggleStyle(.checkbox).font(DS.Font.body)
            Spacer(minLength: 0)
        }
    }

    private var background: some View {
        DSFormRow(label: "Background", labelWidth: labelWidth) {
            RoundedRectangle(cornerRadius: DS.Radius.field).fill(o.backgroundColor)
                .frame(width: 36, height: DS.Metric.control)
                .overlay(RoundedRectangle(cornerRadius: DS.Radius.field).stroke(Color.white.opacity(0.12), lineWidth: 1))
                .overlay(ColorPicker("", selection: $o.backgroundColor, supportsOpacity: false).labelsHidden().opacity(0.02))
                .help("Transparent pixels are flattened onto this color")
            Text(o.background.uppercased()).font(DS.Font.caption).foregroundStyle(DSColor.textSecondary)
            Spacer(minLength: 0)
        }
    }

    private func sizeBinding(_ size: Int) -> Binding<Bool> {
        Binding(get: { o.icoSizes.contains(size) }, set: { on in
            o.icoSizes.removeAll { $0 == size }
            if on { o.icoSizes.append(size) }
            o.icoSizes.sort()
        })
    }

    var body: some View {
        if format >= 5 {
            Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
        }
        switch format {
        case 5:
            DSFormRow(label: "Compression", labelWidth: labelWidth) {
                DSSegmented(selection: $o.lossless, options: [.init(true, title: "Lossless"), .init(false, title: "Lossy")])
                if !o.lossless {
                    DSNumberField("Quality", value: $o.quality, range: 1...100, width: 112, help: "Smaller files at lower quality")
                }
                Spacer(minLength: 0)
            }
            check("Animated (all frames)", $o.animated)
            DSNote(o.lossless ? "Lossless keeps every pixel exactly, transparency included." : "Lossy makes smaller files and blurs sharp edges; use it for large images, not for pixel art.")
        case 6:
            DSFormRow(label: "Quality", labelWidth: labelWidth) {
                DSNumberField("Quality", value: $o.quality, range: 1...100, width: 112, help: "1 smallest file, 100 best image")
                Spacer(minLength: 0)
            }
            background
            DSNote("JPEG has no transparency. Colors keep full detail (no chroma subsampling), which suits pixel art.")
        case 7, 8, 10:
            DSFormRow(label: "Depth", labelWidth: labelWidth) {
                DSPopup(selection: $o.bits, options: format == 10
                        ? [(0, "Automatic"), (8, "256 colors"), (24, "24 bit")]
                        : [(0, "Automatic"), (8, "256 colors"), (24, "24 bit"), (32, "32 bit with alpha")],
                        help: "Automatic: alpha 32 bit, up to 256 colors 8 bit, otherwise 24 bit", fill: true)
            }
            if format == 8 { check("Run-length compression", $o.rle) }
            background
            DSNote(format == 10 ? "PCX has no transparency." : "Transparency needs 32 bit; the other depths flatten onto the background.")
        case 9:
            DSFormRow(label: "Sizes", labelWidth: labelWidth) {
                ForEach(ImageFormats.iconSizes, id: \.self) { size in
                    DSChip("\(size)", help: "\(size) × \(size) pixels", isOn: sizeBinding(size))
                }
                Spacer(minLength: 0)
            }
            DSNote(o.icoSizes.isEmpty
                   ? "No size chosen: the icon has the sprite's own size (up to 256 pixels)."
                   : "The sprite is scaled by a whole number to fit each square, and centered.")
        case 11:
            background
            DSNote("FLI/FLC has 256 colors and no transparency; the animation keeps every frame and its duration.")
        default:
            EmptyView()
        }
    }
}

/// File > Export > …: one entry per new format, opening the sheet with it chosen.
struct ExportImageMenu: View {
    let model: SpriteModel

    var body: some View {
        Menu("Export As") {
            ForEach(ImageFormats.exportFormats) { f in
                Button("\(f.title)…") { model.showExportOptions(format: f.id) }
            }
        }
    }
}
