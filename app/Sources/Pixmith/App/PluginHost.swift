import Foundation
import PixmithCore

/// Loads the scripts that enabled extensions contribute as plugins (docs/LUA_DIALOG.md, "Plugins"): once per script
/// when the extension is enabled, stopped (its `exit` runs) when it is disabled, uninstalled or updated.
final class PluginHost: ObservableObject {
    static let shared = PluginHost()
    private init() {}

    /// Script key (path and modification time) to its session.
    private(set) var loaded: [String: UInt64] = [:]

    private struct Entry {
        let key: String
        let name: String
        let path: String
        let dir: String
        let prefs: String
    }

    private static func wanted() -> [Entry] {
        guard let p = pixmith_ext_scripts() else { return [] }
        defer { pixmith_string_free(p) }
        guard let data = String(cString: p).data(using: .utf8),
              let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return [] }
        return array.compactMap { j in
            guard let ext = j["extension"] as? String, let name = j["name"] as? String, let path = j["path"] as? String,
                  let dir = j["dir"] as? String, let prefs = j["prefs"] as? String else { return nil }
            let modified = (try? FileManager.default.attributesOfItem(atPath: path)[.modificationDate] as? Date)?
                .timeIntervalSince1970 ?? 0
            return Entry(key: "\(path)@\(modified)", name: "\(ext)/\(name)", path: path, dir: dir, prefs: prefs)
        }
    }

    /// Makes the loaded plugins match the enabled extensions' scripts.
    func syncExtensions() {
        let entries = Self.wanted()
        let keys = Set(entries.map(\.key))
        for (key, session) in loaded where !keys.contains(key) {
            ScriptSessions.shared.stop(session)
            loaded[key] = nil
        }
        for e in entries where loaded[e.key] == nil {
            guard let source = try? String(contentsOfFile: e.path, encoding: .utf8) else { continue }
            try? FileManager.default.createDirectory(atPath: (e.prefs as NSString).deletingLastPathComponent,
                                                     withIntermediateDirectories: true)
            let id = ScriptSessions.shared.loadPlugin(source: source, name: e.name, dir: e.dir, prefsPath: e.prefs)
            if id != 0 { loaded[e.key] = id }
        }
    }
}
