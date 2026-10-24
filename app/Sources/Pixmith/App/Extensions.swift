import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

/// One installed extension, as `pixmith_ext_list()` reports it.
struct InstalledExtension: Identifiable, Equatable {
    var id: String { name }
    let name: String
    let displayName: String
    let version: String
    let author: String
    let description: String
    var enabled: Bool
    let dir: String
    /// Why the extension cannot contribute (damaged package.json, a missing file...). It can still be uninstalled.
    let error: String?
    let scripts: [(name: String, path: String)]
    let palettes: [String]
    let themes: [String]

    static func == (a: Self, b: Self) -> Bool {
        a.name == b.name && a.version == b.version && a.enabled == b.enabled && a.error == b.error
    }

    init?(json j: [String: Any]) {
        guard let name = j["name"] as? String else { return nil }
        self.name = name
        displayName = j["displayName"] as? String ?? name
        version = j["version"] as? String ?? ""
        author = j["author"] as? String ?? ""
        description = j["description"] as? String ?? ""
        enabled = j["enabled"] as? Bool ?? true
        dir = j["dir"] as? String ?? ""
        error = j["error"] as? String
        scripts = (j["scripts"] as? [[String: Any]] ?? []).compactMap {
            guard let n = $0["name"] as? String, let p = $0["path"] as? String else { return nil }
            return (n, p)
        }
        palettes = (j["palettes"] as? [[String: Any]] ?? []).compactMap { $0["name"] as? String }
        themes = (j["themes"] as? [[String: Any]] ?? []).compactMap { $0["name"] as? String }
    }

    /// "2 scripts, 1 palette, 1 theme".
    var summary: String {
        func plural(_ n: Int, _ s: String) -> String? { n == 0 ? nil : "\(n) \(s)\(n == 1 ? "" : "s")" }
        return [plural(scripts.count, "script"), plural(palettes.count, "palette"), plural(themes.count, "theme")]
            .compactMap { $0 }.joined(separator: ", ")
    }
}

/// The installed extensions and the actions of Preferences > Extensions. Packages are validated and unpacked by the
/// core (`pixmith_ext_*`, docs/EXTENSIONS.md); this is the window around it.
final class ExtensionLibrary: ObservableObject {
    static let shared = ExtensionLibrary()
    static let packageExtension = "pixmith-extension"

    @Published private(set) var items: [InstalledExtension] = []
    /// A line under the list: the result of the last action (text, isError).
    @Published var message: (text: String, isError: Bool)?
    /// Bumped whenever the set of extensions may have changed; menus and plugin hosts watch it.
    @Published private(set) var generation = 0

    private init() { load() }

    /// Reads the extension folder again (Preferences opened, or files changed outside the app).
    func reload() {
        pixmith_ext_refresh()
        load()
        changed()
    }

    private func load() {
        guard let p = pixmith_ext_list() else { items = []; return }
        defer { pixmith_string_free(p) }
        let array = String(cString: p).data(using: .utf8)
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [[String: Any]] } ?? []
        items = array.compactMap(InstalledExtension.init(json:))
    }

    private func changed() {
        generation &+= 1
        ThemeStore.shared.reload()
        PluginHost.shared.syncExtensions()
    }

    /// Installs or updates a package. A rejected package leaves nothing behind and is reported in an alert.
    @discardableResult
    func install(_ url: URL) -> Bool {
        guard let p = url.path.withCString({ pixmith_ext_install($0) }) else {
            SpriteModel.shared.errorMessage = "Could not install the extension: \(String(cString: pixmith_last_error()))"
            return false
        }
        defer { pixmith_string_free(p) }
        load()
        let info = String(cString: p).data(using: .utf8)
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }.flatMap(InstalledExtension.init(json:))
        let name = info.map { "\($0.displayName) \($0.version)" } ?? url.deletingPathExtension().lastPathComponent
        message = ("\(info?.error == nil ? "Installed" : "Installed with problems"): \(name)", false)
        changed()
        ExtensionPrefs.show(.extensions)
        return true
    }

    func setEnabled(_ name: String, _ enabled: Bool) {
        let ok = name.withCString { pixmith_ext_set_enabled($0, enabled) }
        message = ok ? nil : (String(cString: pixmith_last_error()), true)
        load()
        changed()
    }

    func uninstall(_ name: String) {
        let ok = name.withCString { pixmith_ext_uninstall($0) }
        message = ok ? ("Uninstalled \(name)", false) : (String(cString: pixmith_last_error()), true)
        load()
        changed()
    }

    func openFolder() {
        guard let p = pixmith_ext_dir() else { return }
        defer { pixmith_string_free(p) }
        let url = URL(fileURLWithPath: String(cString: p), isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        NSWorkspace.shared.open(url)
    }

    /// Add Extension…
    func addExtensionPanel() {
        let panel = NSOpenPanel()
        if let type = UTType(filenameExtension: Self.packageExtension) { panel.allowedContentTypes = [type] }
        panel.message = "Choose a Pixmith extension to install"
        if panel.runModal() == .OK, let url = panel.url { install(url) }
    }

    static func isPackage(_ url: URL) -> Bool { url.pathExtension.lowercased() == packageExtension }
}

/// Which tab of the Preferences window is showing, so an installed package can bring Extensions forward.
enum PrefsTabID: Hashable { case general, appearance, keyboard, extensions }

final class ExtensionPrefs: ObservableObject {
    static let shared = ExtensionPrefs()
    @Published var tab: PrefsTabID = .general

    static func show(_ tab: PrefsTabID) {
        shared.tab = tab
        NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
    }
}

// MARK: - Preferences > Extensions

struct ExtensionsPrefsView: View {
    @ObservedObject private var library = ExtensionLibrary.shared
    @State private var confirming: String?

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: DS.Space.sm) {
                    if library.items.isEmpty { empty }
                    ForEach(library.items) { card($0) }
                }
                .padding(DS.Space.lg)
            }
            DSDivider()
            footer
        }
        .onAppear { library.reload() }
    }

    private var empty: some View {
        VStack(spacing: DS.Space.xs) {
            Text("No extensions installed").font(DS.Font.bodyLarge.weight(.medium))
            DSNote("Extensions add scripts, palettes and themes.")
            DSNote("Add a .pixmith-extension file, or drop one on the window.")
        }
        .frame(maxWidth: .infinity)
        .padding(.top, DS.Space.xl)
    }

    private func card(_ e: InstalledExtension) -> some View {
        VStack(alignment: .leading, spacing: DS.Space.xs) {
            HStack(alignment: .firstTextBaseline, spacing: DS.Space.sm) {
                Text(e.displayName).font(DS.Font.bodyLarge.weight(.semibold))
                Text(e.version).font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                Spacer(minLength: 0)
                Toggle("Enabled", isOn: Binding(get: { e.enabled }, set: { library.setEnabled(e.name, $0) }))
                    .toggleStyle(.switch).controlSize(.small)
            }
            let meta = [e.author.isEmpty ? nil : "by \(e.author)", e.summary.isEmpty ? nil : e.summary]
                .compactMap { $0 }.joined(separator: "  ·  ")
            if !meta.isEmpty { Text(meta).font(DS.Font.caption).foregroundStyle(DSColor.textSecondary) }
            if !e.description.isEmpty {
                Text(e.description).font(DS.Font.body).foregroundStyle(DSColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error = e.error {
                Text(error).font(DS.Font.caption).foregroundStyle(DSColor.destructive)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: DS.Space.sm) {
                if confirming == e.name {
                    Text("Remove \(e.displayName)?").font(DS.Font.body)
                    Spacer(minLength: 0)
                    Button("Cancel") { confirming = nil }.buttonStyle(DSSecondaryButtonStyle())
                    Button("Uninstall") { confirming = nil; library.uninstall(e.name) }
                        .buttonStyle(DSSecondaryButtonStyle(destructive: true))
                } else {
                    Spacer(minLength: 0)
                    Button("Uninstall…") { confirming = e.name }.buttonStyle(DSSecondaryButtonStyle(destructive: true))
                }
            }
            .padding(.top, DS.Space.xs)
        }
        .padding(.horizontal, DS.Space.md).padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: DS.Radius.panel).fill(DSColor.panel2))
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: DS.Space.xs) {
            HStack(spacing: DS.Space.sm) {
                Button("Add Extension…") { library.addExtensionPanel() }.buttonStyle(DSPrimaryButtonStyle())
                Button("Open Folder") { library.openFolder() }.buttonStyle(DSSecondaryButtonStyle())
                Spacer(minLength: 0)
            }
            if let m = library.message {
                Text(m.text).font(DS.Font.caption).foregroundStyle(m.isError ? DSColor.destructive : DSColor.textSecondary)
            } else {
                DSNote("Extensions can run scripts. Install only ones you trust.")
            }
        }
        .padding(DS.Space.lg)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
