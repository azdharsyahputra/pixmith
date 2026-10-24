import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

// Color profile, cel z-index and reference layers: model calls, Sprite Properties, and the menu entries that open them.
// The data is kept in the .aseprite / .pxmt file; see core/src/document/spritedata.rs.

extension SpriteModel {
    // MARK: Color profile

    /// 0 none, 1 sRGB, 2 embedded ICC.
    var colorProfileKind: Int { Int(pixmith_color_profile_kind(doc)) }

    /// "None", "sRGB" or "Embedded ICC (name)".
    var colorProfileLabel: String {
        guard let p = pixmith_color_profile_label(doc) else { return "None" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    /// Assigns sRGB (1) or removes the profile (0). Pixels are not converted.
    func setColorProfile(_ kind: Int) {
        if pixmith_color_profile_set(doc, UInt32(kind)) {
            labelLastStep(kind == 0 ? "Remove Color Profile" : "Assign sRGB Profile")
            changed()
        }
    }

    // MARK: Cel z-index

    /// Z-index of the current layer's cel at the current frame, or nil if the cel is empty.
    var celZIndex: Int? {
        var z: Int32 = 0
        return pixmith_cel_z_index(doc, UInt32(layer), UInt32(frame), &z) ? Int(z) : nil
    }

    func setCelZIndex(_ v: Int) {
        let z = Int32(clamping: v)
        if pixmith_cel_set_z_index(doc, UInt32(layer), UInt32(frame), z) {
            labelLastStep("Cel Z-Index")
            changed()
        }
    }

    // MARK: Reference layers

    func isReference(_ i: Int) -> Bool { pixmith_layer_is_reference(doc, UInt32(i)) }
    func isLocked(_ i: Int) -> Bool { pixmith_layer_is_locked(doc, UInt32(i)) }

    /// Turns a layer into a locked reference layer, or back into a normal one.
    func setReference(_ i: Int, _ on: Bool) {
        if pixmith_layer_set_reference(doc, UInt32(i), on) {
            labelLastStep(on ? "Reference Layer" : "Normal Layer")
            changed()
        }
    }

    func setLocked(_ i: Int, _ locked: Bool) {
        if pixmith_layer_set_locked(doc, UInt32(i), locked) {
            labelLastStep(locked ? "Lock Layer" : "Unlock Layer")
            changed()
        }
    }

    /// How opaque reference layers are drawn in the editor, in percent (0 hides them). Not saved, not an edit.
    var referenceDisplay: Int {
        get { Int(pixmith_reference_display(doc)) }
        set {
            pixmith_set_reference_display(doc, UInt32(clamping: newValue))
            revision &+= 1
        }
    }

    /// Layer > New Reference Layer from File…: any image the system can read, scaled to fit the canvas.
    func newReferenceLayerFromFile() {
        guard !MainWindow.hasSheet else { return }
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.message = "Choose an image for a reference layer. It is scaled to fit the canvas and never exported."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        addReferenceLayer(from: url)
    }

    func addReferenceLayer(from url: URL) {
        stop()
        // The core reads PNG; convert whatever the system can decode.
        var path = url.path
        var temp: URL?
        if url.pathExtension.lowercased() != "png" {
            guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
                  let img = CGImageSourceCreateImageAtIndex(src, 0, nil)
            else {
                errorMessage = "Could not read \(url.lastPathComponent) as an image."
                return
            }
            let t = FileManager.default.temporaryDirectory
                .appendingPathComponent("pixmith-ref-\(UUID().uuidString)")
                .appendingPathComponent(url.deletingPathExtension().lastPathComponent + ".png")
            try? FileManager.default.createDirectory(at: t.deletingLastPathComponent(), withIntermediateDirectories: true)
            guard let dest = CGImageDestinationCreateWithURL(t as CFURL, UTType.png.identifier as CFString, 1, nil) else { return }
            CGImageDestinationAddImage(dest, img, nil)
            guard CGImageDestinationFinalize(dest) else {
                errorMessage = "Could not convert \(url.lastPathComponent)."
                return
            }
            path = t.path
            temp = t
        }
        defer { temp.map { try? FileManager.default.removeItem(at: $0.deletingLastPathComponent()) } }
        let index = path.withCString { pixmith_layer_new_reference(doc, UInt32(layer), $0) }
        if index < 0 {
            errorMessage = lastError
            return
        }
        labelLastStep("New Reference Layer")
        layer = Int(index)
        changed()
    }
}

// MARK: - Sprite Properties

/// Sprite > Sprite Properties…: size, color mode and the color profile.
struct SpritePropertiesSheet: View {
    @ObservedObject var model: SpriteModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let _ = model.revision
        DSDialog(title: "Sprite Properties", subtitle: nil, primary: "Done", showsCancel: false, width: 400,
                 onPrimary: { dismiss() }, onCancel: { dismiss() }) {
            DSFormRow(label: "Size") { Text("\(model.width) × \(model.height) px").font(DS.Font.body) }
            DSFormRow(label: "Frames") { Text("\(model.frameCount)").font(DS.Font.body) }
            DSFormRow(label: "Color mode") { Text(model.colorMode.title).font(DS.Font.body) }
            DSDivider()
            DSFormRow(label: "Color profile") {
                Text(model.colorProfileLabel).font(DS.Font.body).lineLimit(2)
                    .accessibilityIdentifier("colorProfileLabel")
            }
            HStack(spacing: DS.Space.sm) {
                Button("Assign sRGB") { model.setColorProfile(1) }
                    .buttonStyle(DSSecondaryButtonStyle())
                    .disabled(model.colorProfileKind == 1)
                Button("Remove Profile") { model.setColorProfile(0) }
                    .buttonStyle(DSSecondaryButtonStyle())
                    .disabled(model.colorProfileKind == 0)
            }
            DSNote("The profile is stored in the file and used by other programs. Changing it does not convert any pixels.")
        }
    }
}

// MARK: - Menu entries

/// Sprite > Sprite Properties…
struct SpriteDataSpriteMenuItems: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Button("Sprite Properties…") { model.toolSheet = .sprite }
    }
}

/// Layer > New Reference Layer from File…, and the reference layer commands of the current layer.
struct ReferenceLayerMenuItems: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Button("New Reference Layer from File…") { model.newReferenceLayerFromFile() }
        Toggle("Reference Layer", isOn: Binding(get: { model.isReference(model.layer) },
                                                set: { model.setReference(model.layer, $0) }))
            .disabled(model.isGroup(model.layer))
    }
}

/// View > Reference Layers: whether and how opaque the editor draws them.
struct ReferenceLayerViewMenu: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Picker("Reference Layers", selection: Binding(get: { model.referenceDisplay }, set: { model.referenceDisplay = $0 })) {
            Text("Hidden").tag(0)
            Text("Opaque").tag(100)
            Text("Semi-transparent (50%)").tag(50)
            Text("Faint (25%)").tag(25)
        }
    }
}
