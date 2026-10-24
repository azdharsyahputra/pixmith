import AppKit
import Foundation
import PixmithCore
import UniformTypeIdentifiers

// Projects: a folder of canvases (.pxmt, plus any .aseprite / .ase) with a ".pixmith" folder holding project.json
// and the project's trash. Everything the app does with projects goes through `ProjectStore`, the one place that
// calls the core's `pixmith_project_*` functions (core/pixmith_project.h).
//
// SWITCH: the core implementation is compiled in with the `PIXMITH_PROJECT_CORE` flag, which Package.swift defines
// when `projectCore` (its first line after the imports) is true. Until then `ProjectStore` uses a pure-Swift fallback
// that keeps the same on-disk layout and produces the same JSON as `pixmith_project_scan`, so the rest of the app
// (and a project made today) works unchanged with either.

// MARK: - Models

struct CanvasInfo: Identifiable, Hashable {
    var id: String { path }
    let path: String
    let name: String
    /// "pxmt" or "aseprite".
    let format: String
    let width: Int
    let height: Int
    let frames: Int
    let layers: Int
    let modified: Date
    let size: Int
    /// Set for files the core could not read; the size fields are 0.
    let error: String?

    var url: URL { URL(fileURLWithPath: path) }
    var fileName: String { url.lastPathComponent }
    var sizeText: String { error == nil ? "\(width) × \(height)" : "Unreadable" }
    /// Changes whenever the file does, so a cached thumbnail is never stale.
    var thumbnailKey: String { "\(path)|\(Int(modified.timeIntervalSince1970))|\(size)" }
}

struct ProjectInfo {
    var name: String
    var folder: URL
    var canvases: [CanvasInfo]
}

struct TrashedCanvas: Identifiable, Hashable {
    var id: String { path }
    let path: String
    let name: String
    let deleted: Date
    var url: URL { URL(fileURLWithPath: path) }
}

// MARK: - ProjectStore

enum ProjectStore {
    static let canvasExtensions: Set<String> = ["pxmt", "aseprite", "ase"]
    /// Files the project's Import accepts.
    static let importExtensions: Set<String> = ["aseprite", "ase", "png"]

    static var usesCore: Bool {
        #if PIXMITH_PROJECT_CORE
        true
        #else
        false
        #endif
    }

    static var lastError: String { String(cString: pixmith_last_error()) }

    // MARK: Queries

    static func isProject(_ folder: URL) -> Bool {
        #if PIXMITH_PROJECT_CORE
        return folder.path.withCString { pixmith_project_is_project($0) }
        #else
        return FileManager.default.fileExists(atPath: Fallback.metaURL(folder).path)
        #endif
    }

    private static var projectCache: [String: (Bool, Date)] = [:]

    /// `isProject` for views that ask on every redraw: the answer is reused for a few seconds.
    static func isProjectCached(_ folder: URL) -> Bool {
        let key = folder.path
        if let (value, at) = projectCache[key], Date().timeIntervalSince(at) < 3 { return value }
        let value = isProject(folder)
        projectCache[key] = (value, Date())
        return value
    }

    /// The project a document file belongs to: its folder, when that folder is a project.
    static func projectFolder(of file: URL?) -> URL? {
        guard let file else { return nil }
        let folder = file.deletingLastPathComponent()
        return isProjectCached(folder) ? folder : nil
    }

    static func scan(_ folder: URL) -> ProjectInfo? {
        #if PIXMITH_PROJECT_CORE
        guard let json = takeString(folder.path.withCString { pixmith_project_scan($0) }) else { return nil }
        #else
        guard let json = Fallback.scanJSON(folder) else { return nil }
        #endif
        return parseScan(json)
    }

    /// Parses the JSON of `pixmith_project_scan`:
    /// `{"name","folder","canvases":[{"path","name","format","width","height","frames","layers","modified","size"} | {…,"error"}]}`.
    static func parseScan(_ json: String) -> ProjectInfo? {
        guard let data = json.data(using: .utf8),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let folderPath = root["folder"] as? String else { return nil }
        let folder = URL(fileURLWithPath: folderPath, isDirectory: true)
        let list = root["canvases"] as? [[String: Any]] ?? []
        let canvases: [CanvasInfo] = list.compactMap { c in
            guard let path = c["path"] as? String else { return nil }
            let url = URL(fileURLWithPath: path)
            func int(_ key: String) -> Int { (c[key] as? NSNumber)?.intValue ?? 0 }
            return CanvasInfo(
                path: path,
                name: c["name"] as? String ?? url.deletingPathExtension().lastPathComponent,
                format: c["format"] as? String ?? (url.pathExtension.lowercased() == "pxmt" ? "pxmt" : "aseprite"),
                width: int("width"), height: int("height"), frames: int("frames"), layers: int("layers"),
                modified: Date(timeIntervalSince1970: (c["modified"] as? NSNumber)?.doubleValue ?? 0),
                size: int("size"), error: c["error"] as? String)
        }
        return ProjectInfo(name: root["name"] as? String ?? folder.lastPathComponent, folder: folder, canvases: canvases)
    }

    // MARK: Changes

    /// Makes `folder` a project named `name`, creating it if needed. Safe on an existing project.
    @discardableResult
    static func create(_ folder: URL, name: String) -> Bool {
        projectCache[folder.path] = nil
        #if PIXMITH_PROJECT_CORE
        return folder.path.withCString { f in name.withCString { pixmith_project_create(f, $0) } }
        #else
        return Fallback.create(folder, name: name)
        #endif
    }

    @discardableResult
    static func setName(_ folder: URL, _ name: String) -> Bool {
        #if PIXMITH_PROJECT_CORE
        return folder.path.withCString { f in name.withCString { pixmith_project_set_name(f, $0) } }
        #else
        return Fallback.update(folder) { $0["name"] = name }
        #endif
    }

    /// Saves the order of the canvases (file names, as the project browser shows them).
    @discardableResult
    static func setOrder(_ folder: URL, _ fileNames: [String]) -> Bool {
        #if PIXMITH_PROJECT_CORE
        guard let data = try? JSONSerialization.data(withJSONObject: fileNames),
              let json = String(data: data, encoding: .utf8) else { return false }
        return folder.path.withCString { f in json.withCString { pixmith_project_set_order(f, $0) } }
        #else
        return Fallback.update(folder) { $0["order"] = fileNames }
        #endif
    }

    /// A new blank .pxmt canvas, named uniquely ("Canvas", "Canvas 2", …).
    static func newCanvas(in folder: URL, name: String, width: Int, height: Int) -> URL? {
        #if PIXMITH_PROJECT_CORE
        return pathResult(folder.path.withCString { f in
            name.withCString { pixmith_project_new_canvas(f, $0, UInt32(clamping: width), UInt32(clamping: height)) }
        })
        #else
        return Fallback.newCanvas(in: folder, name: name, width: width, height: height)
        #endif
    }

    static func duplicate(_ canvas: URL) -> URL? {
        #if PIXMITH_PROJECT_CORE
        return pathResult(canvas.path.withCString { pixmith_project_duplicate_canvas($0) })
        #else
        return Fallback.duplicate(canvas)
        #endif
    }

    /// Renames the file, keeping its extension. Refuses a name that is taken.
    static func rename(_ canvas: URL, to newName: String) -> URL? {
        #if PIXMITH_PROJECT_CORE
        return pathResult(canvas.path.withCString { p in newName.withCString { pixmith_project_rename_canvas(p, $0) } })
        #else
        return Fallback.rename(canvas, to: newName)
        #endif
    }

    /// Moves the canvas to the project's trash. Returns where it is now.
    static func trash(_ canvas: URL) -> URL? {
        #if PIXMITH_PROJECT_CORE
        return pathResult(canvas.path.withCString { pixmith_project_trash_canvas($0) })
        #else
        return Fallback.trash(canvas)
        #endif
    }

    static func restore(_ trashed: URL) -> URL? {
        #if PIXMITH_PROJECT_CORE
        return pathResult(trashed.path.withCString { pixmith_project_restore_canvas($0) })
        #else
        return Fallback.restore(trashed)
        #endif
    }

    /// Copies an .aseprite / .ase / .png into the project as a new .pxmt canvas.
    static func importFile(into folder: URL, file: URL) -> URL? {
        #if PIXMITH_PROJECT_CORE
        return pathResult(folder.path.withCString { f in file.path.withCString { pixmith_project_import(f, $0) } })
        #else
        return Fallback.importFile(into: folder, file: file)
        #endif
    }

    /// The canvases in the project's trash, newest first. The layout inside the trash belongs to the core, so this
    /// looks one folder level deep.
    static func trashed(in folder: URL) -> [TrashedCanvas] {
        let dir = folder.appendingPathComponent(".pixmith/trash", isDirectory: true)
        let keys: [URLResourceKey] = [.isDirectoryKey, .addedToDirectoryDateKey, .contentModificationDateKey]
        func entries(_ url: URL) -> [URL] {
            (try? FileManager.default.contentsOfDirectory(at: url, includingPropertiesForKeys: keys,
                                                          options: [.skipsHiddenFiles])) ?? []
        }
        var found: [URL] = []
        for item in entries(dir) {
            if (try? item.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
                found += entries(item).filter { canvasExtensions.contains($0.pathExtension.lowercased()) }
            } else if canvasExtensions.contains(item.pathExtension.lowercased()) {
                found.append(item)
            }
        }
        return found.map { url in
            let values = try? url.resourceValues(forKeys: Set(keys))
            let date = values?.addedToDirectoryDate ?? values?.contentModificationDate ?? Date()
            return TrashedCanvas(path: url.path, name: url.deletingPathExtension().lastPathComponent, deleted: date)
        }
        .sorted { $0.deleted > $1.deleted }
    }

    // MARK: Files

    /// Writes `doc` as .aseprite whatever the extension says (File > Export As Aseprite…).
    static func saveAseprite(_ doc: OpaquePointer, to url: URL) -> Bool {
        #if PIXMITH_PROJECT_CORE
        return url.path.withCString { pixmith_save_aseprite(doc, $0) }
        #else
        // The current core writes .aseprite for every path.
        return url.path.withCString { pixmith_save(doc, $0) }
        #endif
    }

    /// Exports a canvas file as .aseprite without opening it in the editor.
    static func exportAseprite(_ canvas: URL, to dest: URL) -> Bool {
        guard let doc = canvas.path.withCString({ pixmith_open($0) }) else { return false }
        defer { pixmith_free(doc) }
        return saveAseprite(doc, to: dest)
    }

    /// A preview of a document file fitted inside `maxWidth` × `maxHeight`, nearest-neighbour. Safe on any thread.
    static func thumbnail(_ file: URL, maxWidth: Int, maxHeight: Int) -> CGImage? {
        #if PIXMITH_PROJECT_CORE
        var buffer = [UInt8](repeating: 0, count: maxWidth * maxHeight * 4)
        var w: UInt32 = 0, h: UInt32 = 0
        let ok = file.path.withCString { pixmith_file_thumbnail($0, UInt32(maxWidth), UInt32(maxHeight), &buffer, buffer.count, &w, &h) }
        guard ok, w > 0, h > 0, Int(w) * Int(h) * 4 <= buffer.count else { return nil }
        return cgImage(rgba: Array(buffer[0..<Int(w) * Int(h) * 4]), width: Int(w), height: Int(h))
        #else
        return Fallback.thumbnail(file, maxWidth: maxWidth, maxHeight: maxHeight)
        #endif
    }

    // MARK: Helpers

    #if PIXMITH_PROJECT_CORE
    private static func takeString(_ p: UnsafeMutablePointer<CChar>?) -> String? {
        guard let p else { return nil }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    private static func pathResult(_ p: UnsafeMutablePointer<CChar>?) -> URL? {
        takeString(p).map { URL(fileURLWithPath: $0) }
    }
    #endif

    /// Straight-alpha RGBA bytes as an image.
    static func cgImage(rgba: [UInt8], width: Int, height: Int) -> CGImage? {
        guard width > 0, height > 0, rgba.count >= width * height * 4,
              let provider = CGDataProvider(data: Data(rgba) as CFData) else { return nil }
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                       space: CGColorSpace(name: CGColorSpace.sRGB)!,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }

    /// File names that may not appear in a canvas name.
    static func cleanName(_ raw: String, fallback: String) -> String {
        var name = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        name = name.replacingOccurrences(of: "/", with: "-").replacingOccurrences(of: ":", with: "-")
        while name.hasPrefix(".") { name.removeFirst() }
        for ext in [".pxmt", ".aseprite", ".ase"] where name.lowercased().hasSuffix(ext) { name.removeLast(ext.count) }
        name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? fallback : name
    }
}

// MARK: - Fallback

#if !PIXMITH_PROJECT_CORE
/// Pure-Swift stand-in for core/src/project.rs, used until the core implements the `pixmith_project_*` functions.
/// Same layout on disk (`.pixmith/project.json`, `.pixmith/trash/`), same scan JSON; documents are written with the
/// existing `pixmith_save` (so a ".pxmt" file holds .aseprite bytes until the core writes the real format; `pixmith_open`
/// reads either).
private enum Fallback {
    static func metaURL(_ folder: URL) -> URL { folder.appendingPathComponent(".pixmith/project.json") }
    static func trashURL(_ folder: URL) -> URL { folder.appendingPathComponent(".pixmith/trash", isDirectory: true) }

    static func readMeta(_ folder: URL) -> [String: Any] {
        (try? Data(contentsOf: metaURL(folder))).flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any] ?? [:]
    }

    @discardableResult
    static func update(_ folder: URL, _ change: (inout [String: Any]) -> Void) -> Bool {
        var meta = readMeta(folder)
        change(&meta)
        do {
            try FileManager.default.createDirectory(at: metaURL(folder).deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONSerialization.data(withJSONObject: meta, options: [.prettyPrinted, .sortedKeys]).write(to: metaURL(folder), options: .atomic)
            return true
        } catch { return false }
    }

    static func create(_ folder: URL, name: String) -> Bool {
        do { try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true) } catch { return false }
        return update(folder) {
            $0["name"] = name
            if $0["created"] == nil { $0["created"] = Int(Date().timeIntervalSince1970) }
            if $0["order"] == nil { $0["order"] = [String]() }
        }
    }

    private static func canvasFiles(_ folder: URL) -> [URL] {
        let urls = (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil,
                                                                 options: [.skipsHiddenFiles])) ?? []
        return urls.filter { ProjectStore.canvasExtensions.contains($0.pathExtension.lowercased()) }
    }

    private struct Header { var w = 0, h = 0, frames = 0, layers = 0, error: String? }
    private static var headers: [String: Header] = [:]
    private static let lock = NSLock()

    private static func header(_ url: URL, key: String) -> Header {
        lock.lock()
        if let h = headers[key] { lock.unlock(); return h }
        lock.unlock()
        var header = Header()
        if let doc = url.path.withCString({ pixmith_open($0) }) {
            header = Header(w: Int(pixmith_width(doc)), h: Int(pixmith_height(doc)), frames: Int(pixmith_frame_count(doc)),
                            layers: Int(pixmith_layer_count(doc)))
            pixmith_free(doc)
        } else {
            header.error = String(cString: pixmith_last_error())
        }
        lock.lock()
        headers[key] = header
        lock.unlock()
        return header
    }

    static func scanJSON(_ folder: URL) -> String? {
        let fm = FileManager.default
        guard fm.fileExists(atPath: folder.path) else { return nil }
        let meta = readMeta(folder)
        let order = meta["order"] as? [String] ?? []
        var byName: [String: URL] = [:]
        for u in canvasFiles(folder) { byName[u.lastPathComponent] = u }
        var ordered: [URL] = order.compactMap { byName.removeValue(forKey: $0) }
        // New files follow by name ("Canvas" before "Canvas 2", whatever the extension).
        ordered += byName.values.sorted {
            let (a, b) = ($0.deletingPathExtension().lastPathComponent, $1.deletingPathExtension().lastPathComponent)
            let r = a.localizedStandardCompare(b)
            return r == .orderedSame ? $0.lastPathComponent < $1.lastPathComponent : r == .orderedAscending
        }
        let canvases: [[String: Any]] = ordered.map { url in
            let values = try? url.resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey])
            let modified = values?.contentModificationDate?.timeIntervalSince1970 ?? 0
            let size = values?.fileSize ?? 0
            var item: [String: Any] = [
                "path": url.path, "name": url.deletingPathExtension().lastPathComponent,
                "format": url.pathExtension.lowercased() == "pxmt" ? "pxmt" : "aseprite",
                "modified": Int(modified),
            ]
            let h = header(url, key: "\(url.path)|\(Int(modified))|\(size)")
            if let error = h.error {
                item["error"] = error
            } else {
                item["width"] = h.w; item["height"] = h.h; item["frames"] = h.frames; item["layers"] = h.layers; item["size"] = size
            }
            return item
        }
        let root: [String: Any] = [
            "name": meta["name"] as? String ?? folder.lastPathComponent, "folder": folder.path, "canvases": canvases,
        ]
        return (try? JSONSerialization.data(withJSONObject: root)).flatMap { String(data: $0, encoding: .utf8) }
    }

    /// "Canvas", "Canvas 2", … or "hero copy", "hero copy 2", …: the first name whose file does not exist.
    static func unique(_ base: String, ext: String, in dir: URL) -> URL {
        let fm = FileManager.default
        var url = dir.appendingPathComponent(base).appendingPathExtension(ext)
        var n = 2
        while fm.fileExists(atPath: url.path) {
            url = dir.appendingPathComponent("\(base) \(n)").appendingPathExtension(ext)
            n += 1
        }
        return url
    }

    static func newCanvas(in folder: URL, name: String, width: Int, height: Int) -> URL? {
        let doc = pixmith_new(UInt32(clamping: max(width, 1)), UInt32(clamping: max(height, 1)))
        defer { pixmith_free(doc) }
        guard let doc else { return nil }
        let url = unique(ProjectStore.cleanName(name, fallback: "Canvas"), ext: "pxmt", in: folder)
        return url.path.withCString({ pixmith_save(doc, $0) }) ? url : nil
    }

    static func duplicate(_ canvas: URL) -> URL? {
        let dir = canvas.deletingLastPathComponent()
        let base = canvas.deletingPathExtension().lastPathComponent + " copy"
        let url = unique(base, ext: canvas.pathExtension, in: dir)
        do { try FileManager.default.copyItem(at: canvas, to: url) } catch { return nil }
        return url
    }

    static func rename(_ canvas: URL, to newName: String) -> URL? {
        let base = ProjectStore.cleanName(newName, fallback: "")
        guard !base.isEmpty else { return nil }
        let dir = canvas.deletingLastPathComponent()
        let target = dir.appendingPathComponent(base).appendingPathExtension(canvas.pathExtension)
        if target == canvas { return canvas }
        // A case-only change is the same file on a case-insensitive volume, so it is allowed.
        if FileManager.default.fileExists(atPath: target.path), target.path.lowercased() != canvas.path.lowercased() { return nil }
        do { try FileManager.default.moveItem(at: canvas, to: target) } catch { return nil }
        if FileManager.default.fileExists(atPath: metaURL(dir).path) {
            update(dir) { meta in
                meta["order"] = (meta["order"] as? [String])?.map { $0 == canvas.lastPathComponent ? target.lastPathComponent : $0 }
            }
        }
        return target
    }

    static func trash(_ canvas: URL) -> URL? {
        let dir = canvas.deletingLastPathComponent()
        let trash = trashURL(dir)
        do { try FileManager.default.createDirectory(at: trash, withIntermediateDirectories: true) } catch { return nil }
        let target = unique(canvas.deletingPathExtension().lastPathComponent, ext: canvas.pathExtension, in: trash)
        do { try FileManager.default.moveItem(at: canvas, to: target) } catch { return nil }
        return target
    }

    static func restore(_ trashed: URL) -> URL? {
        // …/<project>/.pixmith/trash/<file>
        let project = trashed.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let target = unique(trashed.deletingPathExtension().lastPathComponent, ext: trashed.pathExtension, in: project)
        do { try FileManager.default.moveItem(at: trashed, to: target) } catch { return nil }
        return target
    }

    static func importFile(into folder: URL, file: URL) -> URL? {
        let doc: OpaquePointer?
        if file.pathExtension.lowercased() == "png" {
            doc = file.path.withCString { pixmith_import_png($0) }
        } else {
            doc = file.path.withCString { pixmith_open($0) }
        }
        guard let doc else { return nil }
        defer { pixmith_free(doc) }
        let url = unique(ProjectStore.cleanName(file.deletingPathExtension().lastPathComponent, fallback: "Canvas"), ext: "pxmt", in: folder)
        return url.path.withCString({ pixmith_save(doc, $0) }) ? url : nil
    }

    static func thumbnail(_ file: URL, maxWidth: Int, maxHeight: Int) -> CGImage? {
        let doc: OpaquePointer?
        if file.pathExtension.lowercased() == "png" {
            doc = file.path.withCString { pixmith_import_png($0) }
        } else {
            doc = file.path.withCString { pixmith_open($0) }
        }
        guard let doc else { return nil }
        defer { pixmith_free(doc) }
        let w = Int(pixmith_width(doc)), h = Int(pixmith_height(doc))
        guard w > 0, h > 0 else { return nil }
        var rgba = [UInt8](repeating: 0, count: w * h * 4)
        guard pixmith_render(doc, 0, &rgba, rgba.count) else { return nil }
        // Fit inside the box, never enlarging (the view scales small sprites by whole factors).
        let scale = min(1, min(Double(maxWidth) / Double(w), Double(maxHeight) / Double(h)))
        let tw = max(1, Int((Double(w) * scale).rounded())), th = max(1, Int((Double(h) * scale).rounded()))
        guard tw != w || th != h else { return ProjectStore.cgImage(rgba: rgba, width: w, height: h) }
        var out = [UInt8](repeating: 0, count: tw * th * 4)
        for y in 0..<th {
            let sy = min(h - 1, y * h / th)
            for x in 0..<tw {
                let sx = min(w - 1, x * w / tw)
                let s = (sy * w + sx) * 4, d = (y * tw + x) * 4
                out[d] = rgba[s]; out[d + 1] = rgba[s + 1]; out[d + 2] = rgba[s + 2]; out[d + 3] = rgba[s + 3]
            }
        }
        return ProjectStore.cgImage(rgba: out, width: tw, height: th)
    }
}
#endif

// MARK: - Thumbnails

/// Loads document previews on background threads (a few at a time) and keeps them in memory. A preview is keyed by
/// path, modification time and size, so it is rebuilt when the file changes.
final class ThumbnailLoader {
    static let shared = ThumbnailLoader()

    /// Box so a failed load is remembered too (and not retried on every redraw).
    private final class Entry { let image: CGImage?; init(_ image: CGImage?) { self.image = image } }

    /// Pixel size previews are made at: a card's picture area at 2x.
    static let maxWidth = 360, maxHeight = 270

    /// Lets a load that has not started yet be skipped once its card is gone.
    private final class CancelToken {
        private let lock = NSLock()
        private var flag = false
        var cancelled: Bool { lock.lock(); defer { lock.unlock() }; return flag }
        func cancel() { lock.lock(); flag = true; lock.unlock() }
    }

    private let cache = NSCache<NSString, Entry>()
    private let queue: OperationQueue = {
        let q = OperationQueue()
        q.name = "pixmith.thumbnails"
        q.maxConcurrentOperationCount = 3
        q.qualityOfService = .utility
        return q
    }()

    private init() { cache.countLimit = 800 }

    /// The preview if it is already loaded; the outer optional says whether the load has finished.
    func cached(_ key: String) -> CGImage?? {
        cache.object(forKey: key as NSString).map { $0.image }
    }

    func load(_ file: URL, key: String) async -> CGImage? {
        if let hit = cached(key) { return hit }
        let token = CancelToken()
        let image: CGImage? = await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                queue.addOperation {
                    continuation.resume(returning: token.cancelled ? nil
                        : ProjectStore.thumbnail(file, maxWidth: Self.maxWidth, maxHeight: Self.maxHeight))
                }
            }
        } onCancel: {
            token.cancel()
        }
        // A cancelled load (the card scrolled away) is not a result.
        if !Task.isCancelled { cache.setObject(Entry(image), forKey: key as NSString) }
        return image
    }

    func clear() { cache.removeAllObjects() }
}

// MARK: - Recent projects

/// The projects the user has opened or made, most recent first (UserDefaults, like `RecentFiles`).
final class RecentProjects: ObservableObject {
    static let shared = RecentProjects()
    private static let limit = 60

    @Published private(set) var folders: [URL]

    private init() {
        folders = (UserDefaults.standard.stringArray(forKey: Prefs.Key.recentProjects) ?? []).map { URL(fileURLWithPath: $0, isDirectory: true) }
    }

    private func save() { UserDefaults.standard.set(folders.map(\.path), forKey: Prefs.Key.recentProjects) }

    private func normalized(_ url: URL) -> URL { url.standardizedFileURL }

    func add(_ folder: URL) {
        let f = normalized(folder)
        folders = Array(([f] + folders.filter { $0 != f }).prefix(Self.limit))
        save()
    }

    func remove(_ folder: URL) {
        let f = normalized(folder)
        folders.removeAll { $0 == f }
        save()
    }

    func clear() {
        folders = []
        UserDefaults.standard.removeObject(forKey: Prefs.Key.recentProjects)
    }

    /// Re-reads the saved list (tests change the defaults behind the singleton's back).
    func reload() {
        folders = (UserDefaults.standard.stringArray(forKey: Prefs.Key.recentProjects) ?? []).map { URL(fileURLWithPath: $0, isDirectory: true) }
    }
}

/// What a project card needs: name, canvas count, newest change and the cover (its first canvas).
struct ProjectSummary: Equatable {
    var name: String
    var canvasCount: Int
    var modified: Date
    var cover: CanvasInfo?
    var exists: Bool

    static func load(_ folder: URL) -> ProjectSummary {
        guard FileManager.default.fileExists(atPath: folder.path), let info = ProjectStore.scan(folder) else {
            return ProjectSummary(name: folder.lastPathComponent, canvasCount: 0, modified: .distantPast, cover: nil, exists: false)
        }
        let folderDate = (try? folder.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
        let newest = info.canvases.map(\.modified).max() ?? folderDate
        return ProjectSummary(name: info.name, canvasCount: info.canvases.count, modified: newest,
                              cover: info.canvases.first { $0.error == nil }, exists: true)
    }
}
