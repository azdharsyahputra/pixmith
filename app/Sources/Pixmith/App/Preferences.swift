import AppKit
import PixmithCore
import SwiftUI

/// User preferences persisted in UserDefaults. Colors are 0xRRGGBBAA.
enum Prefs {
    enum Key {
        static let newWidth = "newSpriteWidth"
        static let newHeight = "newSpriteHeight"
        static let checkerLight = "checkerLight"
        static let checkerDark = "checkerDark"
        static let gridColor = "gridColor"
        static let defaultZoom = "defaultZoom"
        static let appearance = "appearance"
        static let canvasBgOn = "canvasBackgroundOn"
        static let canvasBg = "canvasBackground"
        static let recentProjects = "recentProjects"
        /// Folder the New Project dialog starts in (the last one used).
        static let projectsLocation = "projectsLocation"
        static let projectSort = "projectSort"
        static let canvasSort = "canvasSort"
        static let homeTab = "homeTab"
        /// Allow AI agents to control Pixmith (Live mode, docs/LIVE_PROTOCOL.md); on by default.
        static let liveEnabled = "liveEnabled"
    }

    enum Default {
        static let size = 32
        /// Checkerboard defaults follow the appearance: a quiet dark pair in Dark, a light pair in Light.
        static var checkerLight: Int { isDark ? 0x3A3A3DFF : 0xF3F3F5FF }
        static var checkerDark: Int { isDark ? 0x2F2F32FF : 0xDADADFFF }
        private static var isDark: Bool {
            NSApplication.shared.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        }
        static let gridColor = 0x0000001F
        static let zoom = 16
        static let canvasBg = 0x3C3C3CFF
    }

    private static func int(_ key: String, _ fallback: Int) -> Int {
        UserDefaults.standard.object(forKey: key) as? Int ?? fallback
    }

    static var liveEnabled: Bool { UserDefaults.standard.object(forKey: Key.liveEnabled) as? Bool ?? true }

    static var newWidth: Int { int(Key.newWidth, Default.size).clamped(1, 4096) }
    static var newHeight: Int { int(Key.newHeight, Default.size).clamped(1, 4096) }
    static var checkerLight: UInt32 { UInt32(truncatingIfNeeded: int(Key.checkerLight, Default.checkerLight)) }
    static var checkerDark: UInt32 { UInt32(truncatingIfNeeded: int(Key.checkerDark, Default.checkerDark)) }
    static var gridColor: CGColor { UInt32(truncatingIfNeeded: int(Key.gridColor, Default.gridColor)).cgColor }

    /// 0 follows the system, 1 light, 2 dark.
    static func applyAppearance() {
        switch int(Key.appearance, 0) {
        case 1: NSApp.appearance = NSAppearance(named: .aqua)
        case 2: NSApp.appearance = NSAppearance(named: .darkAqua)
        default: NSApp.appearance = nil
        }
    }

    /// The area around the sprite: the system's color unless a custom one is chosen.
    static var canvasBackground: NSColor {
        guard UserDefaults.standard.bool(forKey: Key.canvasBgOn) else { return DSColor.workspaceNS }
        return NSColor(cgColor: UInt32(truncatingIfNeeded: int(Key.canvasBg, Default.canvasBg)).cgColor) ?? DSColor.workspaceNS
    }

    /// The preferred zoom, reduced so that large sprites still fit on screen.
    static func initialZoom(width: Int, height: Int) -> CGFloat {
        let fit = (512 / CGFloat(max(width, height, 1))).rounded(.down)
        return max(1, min(CGFloat(int(Key.defaultZoom, Default.zoom).clamped(1, 64)), fit))
    }

    private static var checkerCache: (UInt32, UInt32, CGImage)?

    /// A 2x2 tile of the two checkerboard colors.
    static var checkerImage: CGImage {
        let (light, dark) = (checkerLight, checkerDark)
        if let c = checkerCache, c.0 == light, c.1 == dark { return c.2 }
        func rgb(_ c: UInt32) -> [UInt8] { [UInt8(c >> 24 & 0xFF), UInt8(c >> 16 & 0xFF), UInt8(c >> 8 & 0xFF), 255] }
        let px = rgb(light) + rgb(dark) + rgb(dark) + rgb(light)
        let image = CGImage(
            width: 2, height: 2, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: 8,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
            provider: CGDataProvider(data: Data(px) as CFData)!, decode: nil, shouldInterpolate: false,
            intent: .defaultIntent)!
        checkerCache = (light, dark, image)
        return image
    }
}

private extension Int {
    func clamped(_ lo: Int, _ hi: Int) -> Int { Swift.min(Swift.max(self, lo), hi) }
}

struct PreferencesView: View {
    @ObservedObject private var selection = ExtensionPrefs.shared
    @ObservedObject private var themes = ThemeStore.shared

    var body: some View {
        TabView(selection: $selection.tab) {
            GeneralPrefsView().tabItem { Label("General", systemImage: "gearshape") }.tag(PrefsTabID.general)
            AppearancePrefsView().tabItem { Label("Appearance", systemImage: "paintbrush") }.tag(PrefsTabID.appearance)
            KeyboardPrefsView().tabItem { Label("Keyboard", systemImage: "keyboard") }.tag(PrefsTabID.keyboard)
            ExtensionsPrefsView().tabItem { Label("Extensions", systemImage: "puzzlepiece.extension") }.tag(PrefsTabID.extensions)
        }
        .frame(width: 520, height: 560)
        .tint(themes.selected == nil ? nil : DSColor.accent)
    }
}

struct AppearancePrefsView: View {
    @AppStorage(Prefs.Key.appearance) private var appearance = 0
    @AppStorage(Prefs.Key.canvasBgOn) private var customBackground = false
    @AppStorage(Prefs.Key.canvasBg) private var canvasBg = Prefs.Default.canvasBg
    @ObservedObject private var themes = ThemeStore.shared
    @AppStorage(ThemeStore.defaultsKey) private var colorTheme = ""

    var body: some View {
        Form {
            Section("Theme") {
                Picker("Appearance", selection: $appearance) {
                    Text("System").tag(0)
                    Text("Light").tag(1)
                    Text("Dark").tag(2)
                }
                .pickerStyle(.segmented)
                .onChange(of: appearance) { _ in Prefs.applyAppearance() }
                Picker("Color theme", selection: Binding(get: { themes.selected?.id ?? "" },
                                                        set: { colorTheme = $0; themes.select($0) })) {
                    Text("Default").tag("")
                    ForEach(themes.themes) { Text($0.name).tag($0.id) }
                }
                Text(themes.themes.isEmpty ? "Themes come from extensions (Extensions tab)." : "Themes come from extensions.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Canvas") {
                Toggle("Custom canvas background", isOn: $customBackground)
                ColorPicker("Background color", selection: Binding(
                    get: { Color(cgColor: UInt32(truncatingIfNeeded: canvasBg).cgColor) },
                    set: { canvasBg = Int(UInt32($0.cgColor ?? .black)) }), supportsOpacity: false)
                    .disabled(!customBackground)
                Text("The area around the sprite. Checkerboard and grid colors are under General.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }
}

struct GeneralPrefsView: View {
    @AppStorage(Prefs.Key.newWidth) private var newWidth = Prefs.Default.size
    @AppStorage(Prefs.Key.newHeight) private var newHeight = Prefs.Default.size
    @AppStorage(Prefs.Key.checkerLight) private var checkerLight = Prefs.Default.checkerLight
    @AppStorage(Prefs.Key.checkerDark) private var checkerDark = Prefs.Default.checkerDark
    @AppStorage(Prefs.Key.gridColor) private var gridColor = Prefs.Default.gridColor
    @AppStorage(Prefs.Key.defaultZoom) private var defaultZoom = Prefs.Default.zoom
    @AppStorage(Prefs.Key.liveEnabled) private var liveEnabled = true

    private static let zooms = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64]

    private func color(_ value: Binding<Int>) -> Binding<Color> {
        Binding(get: { Color(cgColor: UInt32(truncatingIfNeeded: value.wrappedValue).cgColor) },
                set: { value.wrappedValue = Int(UInt32($0.cgColor ?? .black)) })
    }

    var body: some View {
        Form {
            Section("New sprite") {
                TextField("Width", value: $newWidth, format: .number)
                TextField("Height", value: $newHeight, format: .number)
            }
            Section("Canvas") {
                ColorPicker("Checkerboard light", selection: color($checkerLight), supportsOpacity: false)
                ColorPicker("Checkerboard dark", selection: color($checkerDark), supportsOpacity: false)
                ColorPicker("Grid", selection: color($gridColor), supportsOpacity: true)
                Picker("Default zoom", selection: $defaultZoom) {
                    ForEach(Self.zooms + (Self.zooms.contains(defaultZoom) ? [] : [defaultZoom]), id: \.self) { Text("\($0)x").tag($0) }
                }
                Text("Large sprites open at a smaller zoom so they fit the window.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Live mode") {
                Toggle("Allow AI agents to control Pixmith (Live mode)", isOn: $liveEnabled)
                    .onChange(of: liveEnabled) { _ in LiveServer.shared.applyPreference() }
                Text("Lets tools on this computer, such as the Pixmith MCP server, edit the open documents. Each edit is an undo step.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Scripts: file access") {
                HStack {
                    Text("Scripts ask before they read, write or run anything.")
                    Spacer()
                    Button("Forget Allowed Scripts") { pixmith_script_reset_grants() }
                }
                Text("Clears every “Always Allow for This Script” answer; those scripts ask again.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Restore Defaults") {
                    newWidth = Prefs.Default.size
                    newHeight = Prefs.Default.size
                    checkerLight = Prefs.Default.checkerLight
                    checkerDark = Prefs.Default.checkerDark
                    gridColor = Prefs.Default.gridColor
                    defaultZoom = Prefs.Default.zoom
                }
            }
        }
        .formStyle(.grouped)
    }
}
