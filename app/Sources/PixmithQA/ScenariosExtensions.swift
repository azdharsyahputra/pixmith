import AppKit
import SwiftUI
@testable import PixmithKit
import PixmithCore

// Extensions and color themes: packing the template with its own script, installing, listing, enabling, palette
// presets, themes through DSColor, hostile packages, and screenshots of Preferences.

/// The repository root, from this file's place in it.
private let repoRoot: URL = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

/// Runs the template's pack.sh and returns the package.
private func packTemplate(_ name: String = "sample.pixmith-extension") -> URL {
    let out = tmp(name)
    try? FileManager.default.removeItem(at: out)
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/bin/sh")
    p.arguments = [repoRoot.appendingPathComponent("scripts/extension-template/pack.sh").path,
                   repoRoot.appendingPathComponent("scripts/extension-template").path, out.path]
    p.standardOutput = FileHandle.nullDevice
    try? p.run()
    p.waitUntilExit()
    return out
}

private let crcTable: [UInt32] = (0..<256).map { n in
    (0..<8).reduce(UInt32(n)) { c, _ in c & 1 == 1 ? 0xEDB88320 ^ (c >> 1) : c >> 1 }
}

/// A zip with stored entries, for packages the real tools refuse to write (names with `..`).
private func storedZip(_ files: [(String, String)]) -> Data {
    var out = [UInt8](), central = [UInt8]()
    func put<T: FixedWidthInteger>(_ buf: inout [UInt8], _ v: T) { withUnsafeBytes(of: v.littleEndian) { buf.append(contentsOf: $0) } }
    func zeros(_ buf: inout [UInt8], _ n: Int) { buf.append(contentsOf: [UInt8](repeating: 0, count: n)) }
    for (name, text) in files {
        let data = Array(text.utf8), nameBytes = Array(name.utf8)
        var crc: UInt32 = ~0
        for b in data { crc = crcTable[Int((crc ^ UInt32(b)) & 0xFF)] ^ (crc >> 8) }
        crc = ~crc
        let offset = UInt32(out.count)
        put(&out, UInt32(0x04034b50)); put(&out, UInt16(20)); zeros(&out, 8)
        put(&out, crc); put(&out, UInt32(data.count)); put(&out, UInt32(data.count))
        put(&out, UInt16(nameBytes.count)); put(&out, UInt16(0))
        out.append(contentsOf: nameBytes); out.append(contentsOf: data)
        put(&central, UInt32(0x02014b50)); put(&central, UInt16(20)); put(&central, UInt16(20)); zeros(&central, 8)
        put(&central, crc); put(&central, UInt32(data.count)); put(&central, UInt32(data.count))
        put(&central, UInt16(nameBytes.count)); zeros(&central, 12); put(&central, offset)
        central.append(contentsOf: nameBytes)
    }
    let cdOffset = UInt32(out.count)
    out.append(contentsOf: central)
    put(&out, UInt32(0x06054b50)); zeros(&out, 4)
    put(&out, UInt16(files.count)); put(&out, UInt16(files.count))
    put(&out, UInt32(central.count)); put(&out, cdOffset); put(&out, UInt16(0))
    return Data(out)
}

/// A token's value as AppKit resolves it for an appearance, 0xRRGGBBAA.
private func resolved(_ color: NSColor, dark: Bool) -> UInt32 {
    var out: UInt32 = 0
    NSAppearance(named: dark ? .darkAqua : .aqua)!.performAsCurrentDrawingAppearance {
        let c = color.usingColorSpace(.sRGB)!
        func b(_ x: CGFloat) -> UInt32 { UInt32((x * 255).rounded()) }
        out = b(c.redComponent) << 24 | b(c.greenComponent) << 16 | b(c.blueComponent) << 8 | b(c.alphaComponent)
    }
    return out
}

/// Installed extensions start empty in every scenario (the support folder is shared by the QA process).
private func clean() {
    ScriptSessions.shared.presentNotice = { _, _, _ in }
    let lib = ExtensionLibrary.shared
    lib.reload()
    lib.items.forEach { lib.uninstall($0.name) }
    UserDefaults.standard.removeObject(forKey: ThemeStore.defaultsKey)
    ThemeStore.shared.reload()
}

/// A SwiftUI view in an offscreen window of the given appearance, saved as a PNG.
@discardableResult
private func snapshotView<V: View>(_ view: V, size: NSSize, dark: Bool, name: String) -> NSBitmapImageRep {
    let host = NSHostingView(rootView: view)
    let container = Backdrop()
    let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    container.frame = NSRect(origin: .zero, size: size)
    host.frame = container.bounds
    host.autoresizingMask = [.width, .height]
    container.addSubview(host)
    window.contentView = container
    pumpRunLoop(0.2)
    window.layoutIfNeeded()
    host.layoutSubtreeIfNeeded()
    pumpRunLoop(0.1)
    container.displayIfNeeded()
    let rep = container.bitmapImageRepForCachingDisplay(in: container.bounds)!
    container.cacheDisplay(in: container.bounds, to: rep)
    try? FileManager.default.createDirectory(atPath: QA.outDir, withIntermediateDirectories: true)
    if let png = rep.representation(using: .png, properties: [:]) {
        try? png.write(to: URL(fileURLWithPath: QA.outDir).appendingPathComponent("\(name).png"))
    }
    return rep
}

func registerExtensions() {
    scenario("ext/template-installs-and-lists") {
        clean()
        let lib = ExtensionLibrary.shared
        let pkg = packTemplate()
        expect(FileManager.default.fileExists(atPath: pkg.path), "pack.sh made a package")
        expect(lib.install(pkg), "the template installs: \(coreError)")
        expectEq(lib.items.count, 1)
        let e = lib.items[0]
        expectEq(e.name, "sample-extension")
        expectEq(e.displayName, "Sample Extension")
        expectEq(e.version, "1.0.0")
        expect(e.enabled && e.error == nil, "enabled without errors: \(e.error ?? "")")
        expectEq(e.palettes, ["Sunset"])
        expectEq(e.themes, ["Midnight"])
        expectEq(e.scripts.count, 1)
        expectEq(e.summary, "1 script, 1 palette, 1 theme")
        expect(FileManager.default.fileExists(atPath: e.scripts[0].path), "the script is on disk")
        expect(e.dir.contains("Extensions/sample-extension"), e.dir)
        // The folder is the one the core reports, below the app data folder.
        let dir = pixmith_ext_dir().map { p -> String in defer { pixmith_string_free(p) }; return String(cString: p) } ?? ""
        expect(e.dir.hasPrefix(dir), "\(e.dir) is inside \(dir)")
        // Disable, enable, uninstall.
        lib.setEnabled(e.name, false)
        expect(!lib.items[0].enabled)
        expectEq(ThemeStore.shared.themes.count, 0, "a disabled extension offers no themes")
        lib.setEnabled(e.name, true)
        expectEq(ThemeStore.shared.themes.count, 1)
        lib.uninstall(e.name)
        expect(lib.items.isEmpty)
        expect(!FileManager.default.fileExists(atPath: e.dir), "the folder is gone")
        // Opening a package installs it (File > Open, Finder, drop).
        SpriteModel.shared.open(pkg)
        expectEq(lib.items.count, 1, "opening a .pixmith-extension installs it")
        clean()
    }

    scenario("ext/palette-presets") {
        clean()
        let m = fresh()
        let builtin = m.presetNames
        ExtensionLibrary.shared.install(packTemplate())
        let names = m.presetNames
        expectEq(names.count, builtin.count + 1)
        expectEq(names.last, "Sunset (Sample Extension)")
        let before = m.palette
        let n = m.historyCounts.undo
        m.loadPreset(names.count - 1)
        expectEq(m.palette.count, 12, "the contributed palette replaced the document's")
        expectEq(m.palette.first, 0x24123AFF)
        expectEq(m.historyCounts.undo, n + 1, "one undo step")
        m.undo()
        expectEq(m.palette, before)
        ExtensionLibrary.shared.setEnabled("sample-extension", false)
        expectEq(m.presetNames, builtin, "disabled: the palette is gone")
        clean()
    }

    scenario("ext/theme-overrides-dscolor") {
        clean()
        let defaultPanelDark = resolved(DSColor.panelNS, dark: true)
        let defaultPanelLight = resolved(DSColor.panelNS, dark: false)
        let defaultAccent = resolved(DSColor.accentNS, dark: true)
        ExtensionLibrary.shared.install(packTemplate())
        let store = ThemeStore.shared
        expectEq(store.themes.map(\.name), ["Midnight"])
        let revision = store.revision
        store.select("sample-extension/themes/midnight.json")
        expect(store.revision > revision, "views are told to redraw")
        expectEq(resolved(DSColor.panelNS, dark: true), 0x171724FF)
        expectEq(resolved(DSColor.panelNS, dark: false), 0xF4F1FAFF)
        expectEq(resolved(DSColor.textNS, dark: true), 0xEDEDF7FF)
        expectEq(resolved(DSColor.accentNS, dark: true), 0xFF6AC1FF)
        expectEq(resolved(DSColor.accentNS, dark: false), 0xC2278DFF)
        expectEq(resolved(DSColor.hoverNS, dark: true), 0x323250FF)
        expectEq(resolved(DSColor.canvasShadowNS, dark: true), 0x00000059, "tokens the theme leaves out keep their value")
        // Back to the default look.
        store.select("")
        expectEq(resolved(DSColor.panelNS, dark: true), defaultPanelDark)
        expectEq(resolved(DSColor.panelNS, dark: false), defaultPanelLight)
        expectEq(resolved(DSColor.accentNS, dark: true), defaultAccent)
        // The choice survives the extension being disabled and comes back with it.
        store.select("sample-extension/themes/midnight.json")
        ExtensionLibrary.shared.setEnabled("sample-extension", false)
        expectEq(resolved(DSColor.panelNS, dark: true), defaultPanelDark, "disabled: default look")
        expectEq(store.selectedID, "sample-extension/themes/midnight.json", "the choice is remembered")
        ExtensionLibrary.shared.setEnabled("sample-extension", true)
        expectEq(resolved(DSColor.panelNS, dark: true), 0x171724FF, "enabled again: the theme returns")
        clean()
        expectEq(resolved(DSColor.panelNS, dark: true), defaultPanelDark)
    }

    scenario("ext/hostile-packages") {
        clean()
        let lib = ExtensionLibrary.shared
        let good = #"{"name":"evil","version":"1"}"#
        let cases: [(String, [(String, String)])] = [
            ("traversal", [("package.json", good), ("../../escaped.txt", "x")]),
            ("absolute", [("package.json", good), ("/tmp/pixmith-escaped.txt", "x")]),
            ("backslash", [("package.json", good), ("..\\escaped.txt", "x")]),
            ("no-manifest", [("a.lua", "x")]),
            ("bad-name", [("package.json", #"{"name":"../evil","version":"1"}"#)]),
            ("missing-file", [("package.json", #"{"name":"evil","version":"1","contributes":{"scripts":["a.lua"]}}"#)]),
            ("broken-theme", [("package.json", #"{"name":"evil","version":"1","contributes":{"themes":["t.json"]}}"#),
                              ("t.json", #"{"dark":{"panel":"red"}}"#)]),
        ]
        for (label, files) in cases {
            let url = tmp("\(label).pixmith-extension")
            try? storedZip(files).write(to: url)
            SpriteModel.shared.errorMessage = nil
            expect(!lib.install(url), "\(label) was installed")
            expect(SpriteModel.shared.errorMessage?.hasPrefix("Could not install the extension") == true,
                   "\(label): \(SpriteModel.shared.errorMessage ?? "no message")")
        }
        SpriteModel.shared.errorMessage = nil
        expect(lib.items.isEmpty, "nothing was installed")
        expect(!FileManager.default.fileExists(atPath: "/tmp/pixmith-escaped.txt"))
        // A valid stored package from the same builder does install, so the failures above are the checks' doing.
        let ok = tmp("ok.pixmith-extension")
        try? storedZip([("package.json", #"{"name":"fine","version":"2.0"}"#)]).write(to: ok)
        expect(lib.install(ok), "a plain package installs: \(coreError)")
        expectEq(lib.items.map(\.name), ["fine"])
        clean()
    }

    scenario("ext/screenshots") {
        clean()
        let lib = ExtensionLibrary.shared
        let pkg = packTemplate()
        // Preferences > Extensions: empty, then with the sample installed, in both appearances.
        for dark in [true, false] {
            let tag = dark ? "dark" : "light"
            snapshotView(ExtensionsPrefsView().frame(width: 520, height: 460), size: NSSize(width: 520, height: 460),
                         dark: dark, name: "ext-prefs-empty-\(tag)")
        }
        lib.install(pkg)
        let two = tmp("second.pixmith-extension")
        try? storedZip([("package.json", #"{"name":"pico-extras","displayName":"Pico Extras","version":"0.3.1","author":"Someone","description":"Two extra palettes.","contributes":{"palettes":["a.hex"]}}"#),
                        ("a.hex", "000000\nffffff\n")]).write(to: two)
        lib.install(two)
        lib.setEnabled("pico-extras", false)
        for dark in [true, false] {
            let tag = dark ? "dark" : "light"
            snapshotView(ExtensionsPrefsView().frame(width: 520, height: 460), size: NSSize(width: 520, height: 460),
                         dark: dark, name: "ext-prefs-list-\(tag)")
            snapshotView(AppearancePrefsView().frame(width: 520, height: 460), size: NSSize(width: 520, height: 460),
                         dark: dark, name: "ext-prefs-appearance-\(tag)")
        }
        // The whole window with the Midnight theme.
        let m = fresh(24, 16)
        m.zoom = 12
        let c = CanvasDriver()
        m.color = 0xFF6AC1FF
        c.stroke([(2, 2), (12, 2), (12, 10)])
        for dark in [true, false] {
            ThemeStore.shared.select("sample-extension/themes/midnight.json")
            let app = AppDriver(dark: dark)
            let tag = dark ? "dark" : "light"
            let themed = app.snapshot("ext-theme-window-\(tag)")
            expect(themed.pixelsWide > 1000, "themed window rendered")
            ThemeStore.shared.select("")
            let plain = AppDriver(dark: dark).snapshot("ext-default-window-\(tag)")
            // A pixel of the title row differs between the default look and the theme.
            let (x, y) = (themed.pixelsWide / 2, 12)
            expect(themed.colorAt(x: x, y: y) != plain.colorAt(x: x, y: y), "the theme changed the window (\(tag))")
        }
        clean()
    }
}
