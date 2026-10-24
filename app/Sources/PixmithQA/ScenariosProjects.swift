import AppKit
import SwiftUI
@testable import PixmithKit
import PixmithCore

// pixmith-qa scenarios for .pxmt documents and projects: the Home screen, a project's canvases (new, rename,
// duplicate, trash, restore, reorder, import), opening canvases in tabs, the editor's project switcher, Save and Export.
// ProjectStore is the only code that talks to the core's project functions, so these pass with the pure-Swift fallback
// and with the core implementation alike.

// MARK: - Helpers

/// A folder for one scenario's projects, empty.
func projectsRoot(_ name: String) -> URL {
    let u = tmp("projects-" + name)
    try? FileManager.default.removeItem(at: u)
    try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
    return u
}

/// Writes a small sprite file (any extension `pixmith_save` knows) with a coloured dot at (1,1) and returns its URL.
@discardableResult
func writeSprite(_ url: URL, w: Int = 8, h: Int = 8, dot: UInt32 = RED) -> URL {
    let doc = pixmith_new(UInt32(w), UInt32(h))!
    pixmith_draw_line(doc, 0, 0, 1, 1, 1, 1, dot, 1)
    expect(url.path.withCString { pixmith_save(doc, $0) }, "wrote \(url.lastPathComponent): \(coreError)")
    pixmith_free(doc)
    return url
}

/// Back to a known state: Home, no projects remembered, one pristine tab.
@discardableResult
func resetProjects() -> SpriteModel {
    ThumbnailLoader.shared.clear()
    let m = resetTabs()
    RecentProjects.shared.clear()
    RecentFiles.shared.clear()
    AppRoute.shared.screen = .home
    AppRoute.shared.sheet = nil
    m.errorMessage = nil
    return m
}

/// A project with `names` as canvases (8x8 .pxmt files), made through the store.
@discardableResult
func makeProject(_ name: String, canvases names: [String], in root: URL? = nil) -> (folder: URL, files: [URL]) {
    let folder = (root ?? projectsRoot(name)).appendingPathComponent(name, isDirectory: true)
    expect(ProjectStore.create(folder, name: name), "project created: \(coreError)")
    let files = names.map { n in ProjectStore.newCanvas(in: folder, name: n, width: 8, height: 8)! }
    ProjectStore.setOrder(folder, files.map(\.lastPathComponent))
    return (folder, files)
}

func asepriteMagic(_ url: URL) -> Bool {
    guard let d = try? Data(contentsOf: url), d.count > 6 else { return false }
    return d[4] == 0xE0 && d[5] == 0xA5
}

/// Waits for an async thumbnail load to finish.
func loadThumbnail(_ url: URL, key: String = UUID().uuidString) -> CGImage? {
    var result: CGImage??
    Task { result = .some(await ThumbnailLoader.shared.load(url, key: key)) }
    waitUntil { result != nil }
    return result ?? nil
}

/// Pixel `i` of an image as 0xRRGGBBAA.
func imagePixel(_ image: CGImage, _ x: Int, _ y: Int) -> UInt32 {
    var px = [UInt8](repeating: 0, count: 4)
    let ctx = CGContext(data: &px, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .none
    ctx.draw(image, to: CGRect(x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
    return UInt32(px[0]) << 24 | UInt32(px[1]) << 16 | UInt32(px[2]) << 8 | UInt32(px[3])
}

private extension CGContext {
    func draw(_ image: CGImage, to rect: CGRect) { draw(image, in: rect) }
}

// MARK: - Scenarios

func registerProjects() {
    scenario("projects/home-shows-at-launch") {
        // A fresh route starts on Home; opening a document (File > Open, Finder, drag and drop) skips it.
        expectEq(AppRoute().screen, Screen.home)
        let m = resetProjects()
        let file = writeSprite(tmp("skip-home.pxmt"))
        expectEq(AppRoute.shared.screen, Screen.home)
        m.requestOpen(file)
        expectEq(AppRoute.shared.screen, Screen.editor, "opening a file goes to the editor, not Home")
        expectEq(m.fileURL, file)
        // Home on request (⇧⌘H) keeps the document; opening an open file returns to the editor.
        AppRoute.shared.showHome(m)
        expectEq(AppRoute.shared.screen, Screen.home)
        expectEq(m.slots.count, 1)
        m.requestOpen(file)
        expectEq(AppRoute.shared.screen, Screen.editor)
        expectEq(m.slots.count, 1, "no second tab for a file that is open")
        // A new sprite also goes to the editor.
        AppRoute.shared.showHome(m)
        m.newSprite(width: 8, height: 8)
        expectEq(AppRoute.shared.screen, Screen.editor)
        // The Home view renders (offscreen) with recent projects in it.
        let p = makeProject("Home Render", canvases: ["a", "b"])
        RecentProjects.shared.add(p.folder)
        AppRoute.shared.showHome(m)
        let app = AppDriver()
        for _ in 0..<20 { app.settle(0.1) }
        let rep = app.snapshot("home")
        expect(rep.pixelsWide > 1000, "Home rendered")
        resetProjects()
    }

    scenario("projects/parse-scan-json") {
        // The JSON shape of pixmith_project_scan (core/pixmith_project.h), including a file the core could not read.
        let json = """
        {"name":"Hero","folder":"/tmp/hero","canvases":[
          {"path":"/tmp/hero/idle.pxmt","name":"idle","format":"pxmt","width":32,"height":24,"frames":4,"layers":3,"modified":1729700000,"size":12345},
          {"path":"/tmp/hero/old.aseprite","name":"old","format":"aseprite","width":16,"height":16,"frames":1,"layers":1,"modified":1700000000,"size":999},
          {"path":"/tmp/hero/bad.pxmt","name":"bad","format":"pxmt","modified":1,"error":"not a pixmith document"}]}
        """
        guard let info = ProjectStore.parseScan(json) else { return expect(false, "parsed") }
        expectEq(info.name, "Hero")
        expectEq(info.folder.path, "/tmp/hero")
        expectEq(info.canvases.map(\.name), ["idle", "old", "bad"])
        expectEq(info.canvases[0].width, 32)
        expectEq(info.canvases[0].frames, 4)
        expectEq(info.canvases[0].layers, 3)
        expectEq(info.canvases[0].modified.timeIntervalSince1970, 1729700000)
        expectEq(info.canvases[0].sizeText, "32 × 24")
        expectEq(info.canvases[1].format, "aseprite")
        expectEq(info.canvases[2].error, "not a pixmith document")
        expectEq(info.canvases[2].sizeText, "Unreadable")
        expect(ProjectStore.parseScan("not json") == nil)
        expect(ProjectStore.parseScan("{}") == nil)
    }

    scenario("projects/create-scan-and-canvases") {
        resetProjects()
        let root = projectsRoot("create")
        let folder = root.appendingPathComponent("Alpha")
        expect(!ProjectStore.isProject(folder))
        expect(ProjectStore.create(folder, name: "Alpha"), coreError)
        expect(ProjectStore.isProject(folder))
        expect(FileManager.default.fileExists(atPath: folder.appendingPathComponent(".pixmith/project.json").path))
        var info = ProjectStore.scan(folder)
        expectEq(info?.name, "Alpha")
        expectEq(info?.canvases.count, 0)
        // New canvases get unique names and the size asked for.
        let a = ProjectStore.newCanvas(in: folder, name: "Canvas", width: 16, height: 12)
        let b = ProjectStore.newCanvas(in: folder, name: "Canvas", width: 32, height: 32)
        expectEq(a?.lastPathComponent, "Canvas.pxmt")
        expectEq(b?.lastPathComponent, "Canvas 2.pxmt")
        info = ProjectStore.scan(folder)
        // (Files nobody has ordered yet come back by name; which of "Canvas" and "Canvas 2" is first is the core's call.)
        expectEq(Set(info?.canvases.map(\.name) ?? []), ["Canvas", "Canvas 2"])
        expectEq(info?.canvases.first { $0.name == "Canvas" }?.sizeText, "16 × 12")
        expectEq(info?.canvases.first { $0.name == "Canvas 2" }?.sizeText, "32 × 32")
        // An .aseprite found in the folder is listed too; unreadable files are listed with an error.
        writeSprite(folder.appendingPathComponent("legacy.aseprite"), w: 5, h: 6)
        try? Data("junk".utf8).write(to: folder.appendingPathComponent("broken.pxmt"))
        info = ProjectStore.scan(folder)
        let byName = Dictionary(uniqueKeysWithValues: (info?.canvases ?? []).map { ($0.name, $0) })
        expectEq(byName["legacy"]?.format, "aseprite")
        expectEq(byName["legacy"]?.width, 5)
        expect(byName["broken"]?.error != nil, "an unreadable file is listed with its error")
        // Order is saved and followed; new files come after it, by name.
        expect(ProjectStore.setOrder(folder, ["Canvas 2.pxmt", "Canvas.pxmt"]))
        info = ProjectStore.scan(folder)
        expectEq(info?.canvases.map(\.name).prefix(2).map { $0 }, ["Canvas 2", "Canvas"])
        expect(ProjectStore.setName(folder, "Alpha Two"))
        expectEq(ProjectStore.scan(folder)?.name, "Alpha Two")
        // Creating again keeps the canvases.
        expect(ProjectStore.create(folder, name: "Alpha Two"))
        expectEq(ProjectStore.scan(folder)?.canvases.count, 4)
        // Not a project, and a missing folder.
        expect(!ProjectStore.isProject(root))
        expect(ProjectStore.scan(root.appendingPathComponent("nope")) == nil)
    }

    scenario("projects/new-project-flow") {
        let m = resetProjects()
        let root = projectsRoot("flow")
        let folder = AppRoute.shared.createProject(name: "My Game", in: root)
        expectEq(folder?.lastPathComponent, "My Game")
        expect(ProjectStore.isProject(root.appendingPathComponent("My Game")))
        expectEq(AppRoute.shared.screen, Screen.project(root.appendingPathComponent("My Game").standardizedFileURL))
        expectEq(RecentProjects.shared.folders.first?.lastPathComponent, "My Game")
        expectEq(UserDefaults.standard.string(forKey: Prefs.Key.projectsLocation), root.path, "the location is remembered")
        // Persisted in UserDefaults: a reload sees it.
        RecentProjects.shared.reload()
        expectEq(RecentProjects.shared.folders.count, 1)
        // The same name again is refused with a message.
        expect(AppRoute.shared.createProject(name: "My Game", in: root) == nil)
        expect(m.errorMessage != nil, "the user is told the project exists")
        m.errorMessage = nil
        // Names with slashes cannot escape the location.
        let odd = AppRoute.shared.createProject(name: "a/b", in: root)
        expectEq(odd?.deletingLastPathComponent().standardizedFileURL, root.standardizedFileURL)
        // Recent projects: newest first, no duplicates, removable.
        let other = makeProject("Other", canvases: [], in: root).folder
        AppRoute.shared.showProject(other)
        AppRoute.shared.showProject(root.appendingPathComponent("My Game"))
        expectEq(RecentProjects.shared.folders.map(\.lastPathComponent).prefix(3).map { $0 }, ["My Game", "Other", "a-b"])
        RecentProjects.shared.remove(other)
        expect(!RecentProjects.shared.folders.contains(other.standardizedFileURL))
        // Open Project on a plain folder offers to make it a project.
        let plain = root.appendingPathComponent("Plain")
        try? FileManager.default.createDirectory(at: plain, withIntermediateDirectories: true)
        SpriteModel.alertRunner = { _ in .alertSecondButtonReturn }
        expect(!AppRoute.shared.openProject(plain), "cancel leaves the folder alone")
        expect(!ProjectStore.isProject(plain))
        SpriteModel.alertRunner = { _ in .alertFirstButtonReturn }
        expect(AppRoute.shared.openProject(plain))
        expect(ProjectStore.isProject(plain))
        SpriteModel.alertRunner = { _ in AlertScript.dontSave }
        // A project folder handed to Open (Finder, the Dock) opens the project.
        AppRoute.shared.showHome(m)
        m.requestOpen(plain)
        expectEq(AppRoute.shared.screen, Screen.project(plain.standardizedFileURL))
        resetProjects()
    }

    scenario("projects/canvas-new-rename-duplicate-trash-restore") {
        let m = resetProjects()
        let p = makeProject("Ops", canvases: ["hero", "tree"])
        let model = ProjectBrowserModel.shared(for: p.folder)
        model.reloadNow()
        expectEq(model.all.map(\.name), ["hero", "tree"])
        // New canvas: made, selected, appended to the order.
        let made = model.newCanvas(name: "boss", width: 24, height: 20)
        expectEq(made?.lastPathComponent, "boss.pxmt")
        expectEq(model.all.map(\.name), ["hero", "tree", "boss"])
        expectEq(model.selection, [model.all.last!.path])
        expectEq(model.all.last?.sizeText, "24 × 20")
        expectEq(AppRoute.shared.screen, Screen.home, "creating without \"open\" stays in the project")
        // Rename keeps the extension, refuses a taken name, and keeps the place in the order.
        let hero = model.all[0]
        let renamed = model.rename(hero, to: "knight")
        expectEq(renamed?.lastPathComponent, "knight.pxmt")
        expectEq(model.all.map(\.name), ["knight", "tree", "boss"])
        expect(!FileManager.default.fileExists(atPath: hero.path))
        expect(model.rename(model.all[1], to: "knight") == nil, "an existing name is refused")
        expect(m.errorMessage != nil)
        m.errorMessage = nil
        expectEq(model.all.map(\.name), ["knight", "tree", "boss"])
        // Duplicate lands next to its original.
        let copy = model.duplicate(model.all[0])
        expectEq(copy?.lastPathComponent, "knight copy.pxmt")
        expectEq(model.all.map(\.name), ["knight", "knight copy", "tree", "boss"])
        expectEq(model.selection, [model.all[1].path])
        let second = model.duplicate(model.all[0])
        expect(second != nil && second != copy, "a second copy gets its own name")
        expectEq(model.all.count, 5)
        expectEq(model.all.first?.name, "knight")
        expect(model.all.dropFirst().prefix(2).allSatisfy { $0.name.hasPrefix("knight copy") }, "copies follow their original: \(model.all.map(\.name))")
        // Trash moves the file into the project's trash, selects a neighbour and offers Undo.
        let tree = model.all[3]
        model.trash([tree])
        expect(!FileManager.default.fileExists(atPath: tree.path), "moved out of the project")
        expectEq(model.all.count, 4)
        expect(!model.all.map(\.name).contains("tree"))
        expectEq(model.trashed.map(\.name), ["tree"])
        expect(model.toast?.actionTitle == "Undo")
        expectEq(model.selection.count, 1)
        model.toast?.action?()
        expectEq(model.all.count, 5, "Undo puts it back")
        expect(model.all.map(\.name).contains("tree"))
        expect(model.trashed.isEmpty)
        // Restore from the Recently deleted list.
        model.trash([model.all[4], model.all[3]])
        expectEq(Set(model.trashed.map(\.name)), ["boss", "tree"])
        expect(model.toast?.message.contains("2 canvases") == true)
        let boss = model.trashed.first { $0.name == "boss" }!
        model.restore(boss)
        expect(model.all.map(\.name).contains("boss"))
        expectEq(model.trashed.map(\.name), ["tree"])
        // A restore never overwrites a canvas that was made in the meantime.
        _ = ProjectStore.newCanvas(in: p.folder, name: "tree", width: 8, height: 8)
        model.reloadNow()
        model.restore(model.trashed[0])
        expectEq(model.all.map(\.name).filter { $0.hasPrefix("tree") }.count, 2, "restored under another name: \(model.all.map(\.name))")
        expect(model.trashed.isEmpty)
        // The project's name.
        model.renameProject("Ops Renamed")
        expectEq(model.name, "Ops Renamed")
        expectEq(ProjectStore.scan(p.folder)?.name, "Ops Renamed")
        resetProjects()
    }

    scenario("projects/open-canvas-in-tab-and-switch") {
        let m = resetProjects()
        let p = makeProject("Tabs", canvases: ["one", "two", "three"])
        let model = ProjectBrowserModel.shared(for: p.folder)
        model.reloadNow()
        // Opening a canvas (double-click) opens it in a tab and shows the editor; the blank launch tab is replaced.
        AppRoute.shared.showProject(p.folder)
        model.open(model.all[0])
        expectEq(AppRoute.shared.screen, Screen.editor)
        expectEq(m.fileURL, model.all[0].url)
        expectEq(m.slots.count, 1)
        // The editor knows the project: the switcher lists its canvases.
        expectEq(ProjectStore.projectFolder(of: m.fileURL)?.standardizedFileURL, p.folder.standardizedFileURL)
        expectEq(ProjectStore.projectFolder(of: tmp("loose.pxmt")), nil, "a file outside a project has no switcher")
        // Switching opens the other canvas in its own tab; switching back finds the tab that exists.
        model.open(model.all[1])
        expectEq(m.slots.count, 2)
        expectEq(m.fileURL?.lastPathComponent, "two.pxmt")
        model.open(model.all[0])
        expectEq(m.slots.count, 2, "no duplicate tab")
        expectEq(m.fileURL?.lastPathComponent, "one.pxmt")
        expect(model.isOpen(model.all[1]))
        expect(!model.isOpen(model.all[2]))
        // Back to Home and into the project again: the tabs are still there.
        AppRoute.shared.showHome(m)
        expectEq(m.slots.count, 2)
        // Renaming a canvas that is open renames its tab; trashing one detaches it (the pixels stay).
        AppRoute.shared.showProject(p.folder)
        let two = model.all[1]
        let newURL = model.rename(two, to: "second")
        expect(m.slot(for: newURL!) != nil, "the tab follows the renamed file")
        expectEq(m.slots.map { m.name(of: $0) }.sorted(), ["one.pxmt", "second.pxmt"])
        model.trash([model.all[0]])
        expect(m.slot(for: p.files[0]) == nil, "no tab keeps pointing at the trashed file")
        expectEq(m.slots.count, 2, "the tab stays open")
        expectEq(m.name(of: m.slots.first { $0.fileURL == nil }!), "one", "it keeps its name")
        // The switcher popover and the title row render.
        AppRoute.shared.showEditor()
        m.open(model.all[0].url)
        let app = AppDriver()
        for _ in 0..<5 { app.settle(0.1) }
        app.snapshot("editor-project-switcher")
        resetProjects()
    }

    scenario("projects/keyboard-and-selection") {
        let m = resetProjects()
        let p = makeProject("Keys", canvases: ["a", "b", "c", "d", "e"])
        let model = ProjectBrowserModel.shared(for: p.folder)
        model.reloadNow()
        model.columns = 2
        AppRoute.shared.showProject(p.folder)
        // Arrow keys move through the grid: right/left by one, down/up by a row.
        expect(model.handleKey(code: 124, flags: []))
        expectEq(model.selectedCanvases.map(\.name), ["a"])
        model.handleKey(code: 124, flags: [])
        expectEq(model.selectedCanvases.map(\.name), ["b"])
        model.handleKey(code: 125, flags: [])
        expectEq(model.selectedCanvases.map(\.name), ["d"])
        model.handleKey(code: 126, flags: [])
        expectEq(model.selectedCanvases.map(\.name), ["b"])
        model.handleKey(code: 123, flags: [])
        model.handleKey(code: 123, flags: [])
        expectEq(model.selectedCanvases.map(\.name), ["a"], "stops at the first card")
        for _ in 0..<9 { model.handleKey(code: 124, flags: []) }
        expectEq(model.selectedCanvases.map(\.name), ["e"], "stops at the last card")
        // Delete moves to Recently deleted and selects the neighbour; Return opens.
        expect(model.handleKey(code: 51, flags: []))
        expectEq(model.all.map(\.name), ["a", "b", "c", "d"])
        expectEq(model.selectedCanvases.map(\.name), ["d"])
        expectEq(model.trashed.map(\.name), ["e"])
        expect(model.handleKey(code: 36, flags: []))
        expectEq(AppRoute.shared.screen, Screen.editor)
        expectEq(m.fileURL?.lastPathComponent, "d.pxmt")
        AppRoute.shared.showProject(p.folder)
        // Command- and Shift-click select several; Delete trashes them all (one Undo).
        model.select(model.all[0])
        model.select(model.all[2], command: true)
        expectEq(Set(model.selectedCanvases.map(\.name)), ["a", "c"])
        model.select(model.all[2], command: true)
        expectEq(model.selectedCanvases.map(\.name), ["a"])
        model.select(model.all[0])
        model.select(model.all[3], shift: true)
        expectEq(model.selectedCanvases.map(\.name), ["a", "b", "c", "d"])
        model.handleKey(code: 0, flags: .command)
        expectEq(model.selection.count, 4, "⌘A selects all")
        model.handleKey(code: 53, flags: [])
        expect(model.selection.isEmpty, "Escape clears")
        model.select(model.all[0]); model.select(model.all[1], shift: true)
        model.handleKey(code: 117, flags: [])
        expectEq(model.all.map(\.name), ["c", "d"])
        model.toast?.action?()
        expectEq(model.all.map(\.name), ["a", "b", "c", "d"])
        // F2 renames the one selected card in place; typing keys are left alone while renaming.
        model.select(model.all[1])
        expect(model.handleKey(code: 120, flags: []))
        expectEq(model.renaming, model.all[1].path)
        expect(!model.handleKey(code: 125, flags: []), "arrows belong to the text field while renaming")
        model.rename(model.all[1], to: "bee")
        expectEq(model.all.map(\.name), ["a", "bee", "c", "d"])
        expect(model.renaming == nil)
        // The window's key monitor feeds the same handler, but only while this project is shown.
        model.installKeys()
        model.select(model.all[0])
        let down = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
                                    characters: "\u{F701}", charactersIgnoringModifiers: "\u{F701}", isARepeat: false, keyCode: 125)!
        NSApp.sendEvent(down)
        expectEq(model.selectedCanvases.map(\.name), ["c"], "Down moved by a row (2 columns)")
        AppRoute.shared.showHome(m)
        NSApp.sendEvent(down)
        expectEq(model.selectedCanvases.map(\.name), ["c"], "Home ignores the grid's keys")
        model.removeKeys()
        resetProjects()
    }

    scenario("projects/reorder-sort-search") {
        resetProjects()
        let p = makeProject("Order", canvases: ["delta", "alpha", "charlie", "bravo"])
        let model = ProjectBrowserModel.shared(for: p.folder)
        model.reloadNow()
        expectEq(model.visible.map(\.name), ["delta", "alpha", "charlie", "bravo"], "manual order = creation order")
        // Drag delta onto charlie: it moves there, and the order is saved for the next launch.
        model.move(model.all[0].path, to: model.all[2].path)
        expectEq(model.all.map(\.name), ["alpha", "charlie", "delta", "bravo"])
        let saved = ProjectStore.scan(p.folder)?.canvases.map(\.name)
        expectEq(saved, model.all.map(\.name), "the new order is on disk")
        model.dragging = model.all[3].path
        model.dropDragged(on: model.all[0].path)
        expectEq(model.all.map(\.name), ["bravo", "alpha", "charlie", "delta"], "dropping onto the first card moves to the front")
        expectEq(ProjectStore.scan(p.folder)?.canvases.map(\.name), model.all.map(\.name))
        // Sorting and searching do not touch the saved order, and reordering is off while they are on.
        model.sort = .name
        expectEq(model.visible.map(\.name), ["alpha", "bravo", "charlie", "delta"])
        expect(!model.isReorderable)
        model.dragging = model.all[1].path
        let before = model.all.map(\.name)
        model.dropDragged(on: model.all[0].path)
        expectEq(model.all.map(\.name), before, "no reorder while sorted")
        model.sort = .manual
        model.query = "AR"
        expectEq(model.visible.map(\.name), ["charlie"])
        model.query = " "
        expectEq(model.visible.count, 4)
        model.query = ""
        model.sort = .modified
        expectEq(model.visible.count, 4)
        model.sort = .manual
        UserDefaults.standard.removeObject(forKey: Prefs.Key.canvasSort)
    }

    scenario("projects/import-and-export-aseprite") {
        let m = resetProjects()
        let p = makeProject("Imports", canvases: ["base"])
        let model = ProjectBrowserModel.shared(for: p.folder)
        model.reloadNow()
        let outside = projectsRoot("outside")
        let ase = writeSprite(outside.appendingPathComponent("hero.aseprite"), w: 12, h: 10, dot: GREEN)
        // A PNG made from a sprite.
        let doc = pixmith_new(6, 4)!
        pixmith_draw_line(doc, 0, 0, 0, 0, 0, 0, BLUE, 1)
        let png = outside.appendingPathComponent("tile.png")
        expect(png.path.withCString { pixmith_export_png(doc, 0, 1, $0) })
        pixmith_free(doc)
        let made = model.importFiles([ase, png, outside.appendingPathComponent("notes.txt")])
        expectEq(made.map(\.lastPathComponent), ["hero.pxmt", "tile.pxmt"], "imports become .pxmt canvases; other files are skipped")
        expectEq(model.all.map(\.name), ["base", "hero", "tile"])
        expectEq(model.all[1].sizeText, "12 × 10")
        expectEq(model.all[2].sizeText, "6 × 4")
        expect(FileManager.default.fileExists(atPath: ase.path), "the original is untouched")
        expect(model.toast?.message.contains("Imported") == true)
        // The imported canvas has the pixels of the original.
        guard let d = made.first.flatMap({ u in u.path.withCString { pixmith_open($0) } }) else { return expect(false, "opened import") }
        expectEq(renderPixels(d, frame: 0)[1 * 12 + 1], GREEN)
        pixmith_free(d)
        // Importing the same file again makes another canvas instead of overwriting.
        let again = model.importFiles([ase])
        expectEq(again.count, 1)
        expect(again[0].lastPathComponent != "hero.pxmt" && FileManager.default.fileExists(atPath: again[0].path), "another canvas, not an overwrite")
        // Export a project canvas as Aseprite without opening it in the editor.
        let out = outside.appendingPathComponent("hero-export.aseprite")
        expect(ProjectStore.exportAseprite(made[0], to: out), coreError)
        expect(asepriteMagic(out), "the export is an Aseprite file")
        guard let back = out.path.withCString({ pixmith_open($0) }) else { return expect(false, "exported file opens") }
        expectEq(Int(pixmith_width(back)), 12)
        expectEq(renderPixels(back, frame: 0)[1 * 12 + 1], GREEN)
        pixmith_free(back)
        expectEq(m.slots.count, 1)
        resetProjects()
    }

    scenario("projects/save-pxmt-aseprite-roundtrip") {
        let m = fresh(10, 6), d = CanvasDriver()
        AppRoute.shared.screen = .editor
        put(d, 2, 3, RED)
        // Untitled documents save as .pxmt, and a name without an extension gets one.
        expectEq(SpriteModel.withDocumentExtension(tmp("art")).lastPathComponent, "art.pxmt")
        expectEq(SpriteModel.withDocumentExtension(tmp("art.aseprite")).lastPathComponent, "art.aseprite")
        expectEq(SpriteModel.withDocumentExtension(tmp("art.PXMT")).lastPathComponent, "art.PXMT")
        expectEq(SpriteModel.pxmtType.preferredFilenameExtension ?? "pxmt", "pxmt")
        let pxmt = tmp("art.pxmt")
        m.fileURL = pxmt
        m.save()
        expect(!m.isDirty)
        expect(FileManager.default.fileExists(atPath: pxmt.path))
        if ProjectStore.usesCore { expect(!asepriteMagic(pxmt), "the core writes its own format to .pxmt") }
        // Reopen it in a new tab: same pixels.
        put(d, 4, 4, BLUE)
        m.save()
        let reopened = SpriteModel.shared
        reopened.fileURL = nil
        reopened.replace(with: pixmith_new(4, 4))
        reopened.open(pxmt)
        expectEq(reopened.fileURL, pxmt)
        expectEq(reopened.width, 10)
        expectPixel(2, 3, RED)
        expectPixel(4, 4, BLUE)
        // Save As .aseprite from a .pxmt document keeps .aseprite; the document now points at it.
        let ase = tmp("art-as.aseprite")
        m.fileURL = nil
        m.fileURL = pxmt
        m.exportAseprite(to: ase)
        expect(asepriteMagic(ase), "Export As Aseprite writes an Aseprite file")
        expectEq(m.fileURL, pxmt, "export leaves the document on its own file")
        expect(!m.isDirty, "export does not change the document's saved state")
        // A document opened from .aseprite saves back to .aseprite.
        let opened = tmp("legacy.aseprite")
        writeSprite(opened, w: 6, h: 6)
        m.requestOpen(opened)
        expectEq(m.fileURL, opened)
        put(d, 3, 3, GREEN)
        m.save()
        expect(asepriteMagic(opened), "Save keeps the Aseprite format")
        expectEq(m.fileURL, opened, "the document keeps its file")
        // A tab rename keeps .pxmt (and .aseprite).
        let slot = m.activeSlotObject!
        m.renameDocument(slot, to: "renamed.pxmt")
        expectEq(m.fileURL?.lastPathComponent, "renamed.aseprite", "the extension of the file stays")
        expect(FileManager.default.fileExists(atPath: tmp("renamed.aseprite").path))
        m.activate(0)
        m.renameDocument(m.slots[0], to: "art-renamed")
        expectEq(m.slots[0].fileURL?.lastPathComponent, "art-renamed.pxmt")
        // Recent files remember .pxmt files.
        expect(RecentFiles.shared.urls.contains { $0.pathExtension == "pxmt" })
        resetTabs()
    }

    scenario("projects/recovery-writes-pxmt") {
        let m = resetTabs(), d = CanvasDriver()
        defer { resetTabs() }
        mark(d, 1, 1, RED)
        let slot = m.activeSlotObject!
        Autosaver.shared.start(m)
        Autosaver.shared.tick()
        expect(waitUntil { FileManager.default.fileExists(atPath: recoveryFile(slot).path) }, "recovery copy is a .pxmt")
        // It opens like any document.
        let doc = recoveryFile(slot).path.withCString { pixmith_open($0) }
        expect(doc != nil)
        if let doc { pixmith_free(doc) }
    }

    scenario("projects/thumbnails-and-big-projects") {
        resetProjects()
        // A thumbnail shows the first frame, keeps small sprites at their size and is cached by the file's stamp.
        let file = writeSprite(tmp("thumb.pxmt"), w: 8, h: 8, dot: RED)
        let image = loadThumbnail(file)
        expect(image != nil, "thumbnail loaded")
        if let image {
            expect(image.width <= ThumbnailLoader.maxWidth && image.height <= ThumbnailLoader.maxHeight)
            expectEq(image.width * 8, image.height * 8, "aspect kept")
            expectEq(imagePixel(image, 1 * image.width / 8, 1 * image.height / 8), RED, "the dot is where the sprite has it")
        }
        expect(ThumbnailLoader.shared.cached("same-key") == nil)
        _ = loadThumbnail(file, key: "same-key")
        expect(ThumbnailLoader.shared.cached("same-key") != nil, "kept in the cache")
        // A wide sprite is fitted inside the box.
        let wide = writeSprite(tmp("wide.pxmt"), w: 600, h: 40)
        if let image = loadThumbnail(wide) {
            expect(image.width <= ThumbnailLoader.maxWidth, "reduced to fit: \(image.width)")
            expectEq(image.width / image.height, 15, "aspect kept")
        } else { expect(false, "wide thumbnail") }
        expect(loadThumbnail(tmp("missing.pxmt")) == nil, "a missing file has no thumbnail")
        // Hundreds of canvases: scanning, filtering and sorting stay fast, and the loader does a few at a time.
        let p = makeProject("Big", canvases: [])
        let doc = pixmith_new(8, 8)!
        for i in 0..<300 {
            expect(p.folder.appendingPathComponent(String(format: "sprite %03d.pxmt", i)).path.withCString { pixmith_save(doc, $0) })
        }
        pixmith_free(doc)
        let model = ProjectBrowserModel.shared(for: p.folder)
        let t0 = Date()
        model.reloadNow()
        let scanTime = Date().timeIntervalSince(t0)
        expectEq(model.all.count, 300)
        expect(scanTime < 20, "scan of 300 canvases took \(scanTime)s")
        let t1 = Date()
        model.query = "sprite 2"
        _ = model.visible
        model.sort = .name
        _ = model.visible
        model.sort = .manual
        model.query = ""
        expect(Date().timeIntervalSince(t1) < 0.5, "filter and sort of 300 canvases")
        // A second scan reuses what it learned.
        let t2 = Date()
        model.reloadNow()
        expect(Date().timeIntervalSince(t2) <= scanTime + 0.5)
        // Loading 40 thumbnails at once finishes and every one arrives.
        var done = 0
        for c in model.all.prefix(40) { Task { _ = await ThumbnailLoader.shared.load(c.url, key: c.thumbnailKey); done += 1 } }
        expect(waitUntil { done == 40 }, "40 thumbnails loaded: \(done)")
        // A load that is cancelled before it starts does not hang and is not cached.
        let task = Task { await ThumbnailLoader.shared.load(file, key: "cancelled") }
        task.cancel()
        var finished = false
        Task { _ = await task.value; finished = true }
        expect(waitUntil { finished }, "a cancelled load returns")
        UserDefaults.standard.removeObject(forKey: Prefs.Key.canvasSort)
    }

    scenario("projects/home-and-project-screenshots") {
        let m = resetProjects()
        // A tidy set of projects and canvases for the screenshots in docs/design/screens.
        let root = projectsRoot("shots")
        let names: [(String, [String])] = [
            ("Dungeon Tiles", ["floor", "wall", "torch", "door", "chest", "stairs", "pillar", "crate"]),
            ("Hero Sprites", ["idle", "run", "jump", "attack", "hurt", "climb"]),
            ("UI Icons", ["heart", "coin", "key", "gem", "star", "potion", "sword", "shield", "bomb", "map"]),
            ("Forest Pack", ["oak", "pine", "bush", "rock", "mushroom"]),
            ("Title Screen", ["logo", "background"]),
        ]
        var projects: [(URL, [URL])] = []
        for (i, (name, canvases)) in names.enumerated() {
            let folder = root.appendingPathComponent(name, isDirectory: true)
            ProjectStore.create(folder, name: name)
            var files: [URL] = []
            for (j, c) in canvases.enumerated() {
                let size = [16, 16, 32, 24][(i + j) % 4]
                guard let url = ProjectStore.newCanvas(in: folder, name: c, width: size, height: size) else { continue }
                paintSample(url, seed: i * 10 + j)
                files.append(url)
            }
            ProjectStore.setOrder(folder, files.map(\.lastPathComponent))
            projects.append((folder, files))
        }
        for (folder, _) in projects.reversed() { RecentProjects.shared.add(folder) }
        for f in projects[1].1.prefix(4) { RecentFiles.shared.add(f) }
        for f in projects[2].1.prefix(3) { RecentFiles.shared.add(f) }
        // A trashed canvas, so the Recently deleted bar shows.
        let hero = ProjectBrowserModel.shared(for: projects[1].0)
        hero.reloadNow()
        hero.trash([hero.all[hero.all.count - 1]])
        hero.toast = nil
        for dark in [true, false] {
            let suffix = dark ? "dark" : "light"
            AppRoute.shared.showHome(m)
            var app = AppDriver(size: NSSize(width: 1280, height: 800), dark: dark)
            settleThumbnails(app)
            app.snapshot("home-\(suffix)")
            AppRoute.shared.showProject(projects[1].0)
            app = AppDriver(size: NSSize(width: 1280, height: 800), dark: dark)
            settleThumbnails(app)
            ProjectBrowserModel.shared(for: projects[1].0).select(ProjectBrowserModel.shared(for: projects[1].0).all[1])
            settleThumbnails(app)
            app.snapshot("project-\(suffix)")
        }
        resetProjects()
    }
}

/// Lets the Home or project view load its thumbnails (background queue plus main-queue updates).
func settleThumbnails(_ app: AppDriver) {
    for _ in 0..<40 { app.settle(0.08) }
}

/// Paints a recognizable little sprite (a gem, a face, a tree, …) into a canvas file.
func paintSample(_ url: URL, seed: Int) {
    guard let doc = url.path.withCString({ pixmith_open($0) }) else { return }
    let w = Int(pixmith_width(doc)), h = Int(pixmith_height(doc))
    let palettes: [[UInt32]] = [
        [0xF2A65AFF, 0xD9823BFF, 0x7A4A2BFF, 0x1A1A2EFF], [0x7BD88FFF, 0x3FA66AFF, 0x1F6B45FF, 0x10281FFF],
        [0x6C9CFFFF, 0x4468D8FF, 0x2B3A8FFF, 0x141A3CFF], [0xFF7A8AFF, 0xD94862FF, 0x8F2540FF, 0x3A0F20FF],
        [0xC9A0FFFF, 0x9568E0FF, 0x5F3FA0FF, 0x25164AFF], [0xFFE27AFF, 0xE0B33AFF, 0x9A6F1AFF, 0x3A2A0AFF],
    ]
    let pal = palettes[seed % palettes.count]
    let shape = seed % 4
    let cx = Double(w - 1) / 2, cy = Double(h - 1) / 2
    let r = Double(min(w, h)) / 2 - 1.5
    func dot(_ x: Int, _ y: Int, _ c: UInt32) { pixmith_draw_line(doc, 0, 0, Int32(x), Int32(y), Int32(x), Int32(y), c, 1) }
    for y in 0..<h {
        for x in 0..<w {
            let dx = Double(x) - cx, dy = Double(y) - cy
            let inside: Bool
            switch shape {
            case 0: inside = dx * dx + dy * dy <= r * r
            case 1: inside = abs(dx) + abs(dy) <= r * 1.15
            case 2: inside = abs(dx) <= r * 0.8 && abs(dy) <= r * 0.8 && (abs(dx) + abs(dy) < r * 1.35)
            default: inside = (dy > -r * 0.2 && abs(dx) <= r * 0.85 && dy < r) || (dy <= -r * 0.2 && abs(dx) <= (dy + r) * 0.9)
            }
            guard inside else { continue }
            let shade = (dx + dy) / (r * 2)
            var c = shade < -0.25 ? pal[0] : shade < 0.25 ? pal[1] : pal[2]
            let edge = !((dx * dx + dy * dy) <= (r - 1.2) * (r - 1.2)) && shape == 0
            if edge || (shape != 0 && (abs(abs(dx) - r * 0.8) < 0.6 || abs(abs(dy) - r * 0.8) < 0.6)) { c = pal[3] }
            dot(x, y, c)
        }
    }
    // Two eyes on the first shape give it a face.
    if shape == 0 {
        let ey = Int(cy - r * 0.15), ex = max(2, Int(r * 0.35))
        dot(Int(cx) - ex, ey, pal[3]); dot(Int(cx) + ex + (w % 2 == 0 ? 1 : 0), ey, pal[3])
    }
    _ = url.path.withCString { pixmith_save(doc, $0) }
    pixmith_free(doc)
}
