import PixmithCore
import SwiftUI

enum ColorMode: UInt32, CaseIterable, Identifiable {
    case rgb = 0, grayscale, indexed

    var id: UInt32 { rawValue }

    var title: String {
        switch self {
        case .rgb: "RGB Color"
        case .grayscale: "Grayscale"
        case .indexed: "Indexed"
        }
    }
}

extension SpriteModel {
    var colorMode: ColorMode { ColorMode(rawValue: pixmith_cm_get_mode(doc)) ?? .rgb }
    var transparentIndex: Int { Int(pixmith_cm_transparent_index(doc)) }

    /// Index of the palette entry equal to the drawing color, if any.
    var selectedPaletteIndex: Int? {
        let i = pixmith_cm_palette_index_of(doc, color)
        return i >= 0 ? Int(i) : nil
    }

    /// Constrains the drawing color to the color mode; called whenever the color changes.
    func snapDrawColor() {
        let snapped = pixmith_cm_snap_color(doc, color)
        if snapped != color { color = snapped }
    }

    /// Converts the sprite (one undo step). `dither` is a Bayer matrix size, 0 for none.
    func convertColorMode(to mode: ColorMode, dither: Int = 0) {
        stop()
        guard mode != colorMode || mode == .indexed else { return }
        guard pixmith_cm_convert(doc, mode.rawValue, UInt32(dither)) else {
            errorMessage = String(cString: pixmith_last_error())
            return
        }
        labelLastStep("Convert Color Mode")
        snapDrawColor()
        changed()
    }

    /// Snaps all pixels to the palette (Indexed) or to gray (Grayscale); one undo step.
    func snapPixelsToPalette() {
        stop()
        if pixmith_cm_conform_all(doc) {
            changed()
        }
    }

    /// Replaces the palette with the sprite's colors; `maxColors` 0 keeps them exact.
    func paletteFromSprite(maxColors: Int) {
        stop()
        guard pixmith_cm_palette_from_sprite(doc, UInt32(maxColors)) > 0 else {
            errorMessage = String(cString: pixmith_last_error())
            return
        }
        snapDrawColor()
        changed()
    }
}

/// Entries of Sprite > Color Mode and Sprite > Palette from Sprite.
struct ColorModeMenu: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Menu("Color Mode") {
            ForEach(ColorMode.allCases) { mode in
                Toggle(mode.title, isOn: Binding(get: { model.colorMode == mode },
                                                 set: { _ in model.convertColorMode(to: mode) }))
            }
            Divider()
            Button("Indexed with Ordered Dithering") { model.convertColorMode(to: .indexed, dither: 4) }
            Divider()
            Button("Snap Pixels to Palette") { model.snapPixelsToPalette() }
                .disabled(model.colorMode == .rgb)
        }
        PalettePresetMenu(model: model)
        Menu("Palette from Sprite") {
            Button("Exact Colors") { model.paletteFromSprite(maxColors: 0) }
            Divider()
            ForEach([8, 16, 32, 64, 128, 256], id: \.self) { n in
                Button("Quantize to \(n) Colors") { model.paletteFromSprite(maxColors: n) }
            }
        }
    }
}
