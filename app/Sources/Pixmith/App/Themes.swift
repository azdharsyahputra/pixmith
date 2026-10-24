import AppKit
import PixmithCore
import SwiftUI

/// A color theme contributed by an extension: token overrides for `DSColor`, per appearance (`0xRRGGBBAA`).
/// Token names are those of core/src/extensions/theme.rs.
struct PixmithTheme: Identifiable, Equatable {
    let id: String
    let name: String
    let extensionName: String
    let dark: [String: UInt32]
    let light: [String: UInt32]

    var stylesDark: Bool { !dark.isEmpty }
    var stylesLight: Bool { !light.isEmpty }

    /// Parses one element of `pixmith_ext_themes()`.
    init?(json: [String: Any]) {
        guard let id = json["id"] as? String, let name = json["name"] as? String else { return nil }
        func tokens(_ any: Any?) -> [String: UInt32] {
            guard let dict = any as? [String: String] else { return [:] }
            return dict.compactMapValues { UInt32($0, radix: 16) }
        }
        self.id = id
        self.name = name
        extensionName = json["extension"] as? String ?? ""
        dark = tokens(json["dark"])
        light = tokens(json["light"])
    }
}

/// The color theme in effect and the themes the enabled extensions offer. `DSColor` asks `override(_:dark:)` every
/// time a color is resolved, so a change shows as soon as the views are drawn again: `revision` is what the root
/// view watches to rebuild.
final class ThemeStore: ObservableObject {
    static let shared = ThemeStore()
    static let defaultsKey = "colorTheme"

    @Published private(set) var themes: [PixmithTheme] = []
    @Published private(set) var revision = 0

    private static let lock = NSLock()
    private static var active: (dark: [String: UInt32], light: [String: UInt32]) = ([:], [:])

    /// The chosen theme's id; empty is the default look. It is kept when the theme's extension is disabled, so
    /// enabling the extension again brings the theme back.
    var selectedID: String { UserDefaults.standard.string(forKey: Self.defaultsKey) ?? "" }
    var selected: PixmithTheme? { themes.first { $0.id == selectedID } }

    private init() { reload() }

    /// The theme's value for a `DSColor` token, if the active theme sets one. Callable from any thread.
    static func override(_ token: String, dark: Bool) -> UInt32? {
        lock.lock()
        defer { lock.unlock() }
        return (dark ? active.dark : active.light)[token]
    }

    /// Reads the enabled extensions' themes again and applies the chosen one (after an install, an
    /// enable/disable or an uninstall, and at launch).
    func reload() {
        themes = Self.fetch()
        activate()
    }

    func select(_ id: String) {
        UserDefaults.standard.set(id, forKey: Self.defaultsKey)
        if let theme = themes.first(where: { $0.id == id }) {
            // A theme that styles only one appearance moves Light/Dark to it, so the choice is visible.
            let isDark = NSApp?.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            if isDark && !theme.stylesDark && theme.stylesLight { setAppearance(1) }
            if !isDark && !theme.stylesLight && theme.stylesDark { setAppearance(2) }
        }
        activate()
    }

    private func setAppearance(_ value: Int) {
        UserDefaults.standard.set(value, forKey: Prefs.Key.appearance)
        Prefs.applyAppearance()
    }

    private func activate() {
        let theme = selected
        Self.lock.lock()
        Self.active = (theme?.dark ?? [:], theme?.light ?? [:])
        Self.lock.unlock()
        revision &+= 1
        // AppKit-drawn views (the canvas area, scrollers) pick the new colors up when they are drawn again.
        for window in NSApp?.windows ?? [] {
            window.contentView?.needsDisplay = true
            window.contentView?.subviews.forEach { $0.needsDisplay = true }
        }
    }

    private static func fetch() -> [PixmithTheme] {
        guard let p = pixmith_ext_themes() else { return [] }
        defer { pixmith_string_free(p) }
        guard let data = String(cString: p).data(using: .utf8),
              let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return [] }
        return array.compactMap(PixmithTheme.init(json:))
    }
}
