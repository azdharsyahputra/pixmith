import AppKit
import SwiftUI
import UniformTypeIdentifiers

// A project's canvases: the model behind the project view and the editor's switcher, and the project view itself.

enum CanvasSort: String, CaseIterable {
    case manual, name, modified
    var title: String {
        switch self {
        case .manual: "Manual order"
        case .name: "Name"
        case .modified: "Modified"
        }
    }
}

struct Toast: Identifiable {
    let id = UUID()
    let message: String
    var actionTitle: String?
    var action: (() -> Void)?
}

/// State and actions of one project folder. One instance per folder (`shared(for:)`), used by both the project
/// view and the editor's canvas switcher, so they always agree.
final class ProjectBrowserModel: ObservableObject {
    private static var instances: [String: ProjectBrowserModel] = [:]

    static func shared(for folder: URL) -> ProjectBrowserModel {
        let key = folder.standardizedFileURL.path
        if let m = instances[key] { return m }
        let m = ProjectBrowserModel(folder: folder.standardizedFileURL)
        instances[key] = m
        return m
    }

    let folder: URL
    @Published private(set) var name: String
    /// Every canvas, in the project's saved order.
    @Published private(set) var all: [CanvasInfo] = []
    @Published private(set) var trashed: [TrashedCanvas] = []
    @Published private(set) var loaded = false
    @Published private(set) var missing = false
    @Published var selection: Set<String> = []
    /// The card the keyboard moves from.
    @Published var lead: String?
    @Published var query = ""
    @Published var sort: CanvasSort = CanvasSort(rawValue: UserDefaults.standard.string(forKey: Prefs.Key.canvasSort) ?? "") ?? .manual {
        didSet { UserDefaults.standard.set(sort.rawValue, forKey: Prefs.Key.canvasSort) }
    }
    /// The canvas being renamed in place.
    @Published var renaming: String?
    @Published var toast: Toast?
    /// Cards per row, reported by the view, so Up and Down move by a row.
    var columns = 4
    /// Drag and drop: the canvas being dragged, and the card it would land next to.
    var dragging: String?
    @Published var dropTarget: String?

    private var generation = 0
    private var toastTimer: Timer?
    private let queue = DispatchQueue(label: "pixmith.project-scan", qos: .userInitiated)

    private init(folder: URL) {
        self.folder = folder
        name = folder.lastPathComponent
    }

    // MARK: Loading

    /// Reads the project in the background; the canvases keep showing while it runs.
    func reload() {
        generation += 1
        let g = generation, folder = folder
        queue.async {
            let info = ProjectStore.scan(folder)
            let trash = ProjectStore.trashed(in: folder)
            DispatchQueue.main.async { if g == self.generation { self.apply(info, trash) } }
        }
    }

    /// Reads the project now (after a change, and in tests).
    func reloadNow() {
        generation += 1
        apply(ProjectStore.scan(folder), ProjectStore.trashed(in: folder))
    }

    private func apply(_ info: ProjectInfo?, _ trashed: [TrashedCanvas]) {
        loaded = true
        guard let info else {
            missing = true
            return
        }
        missing = false
        name = info.name
        all = info.canvases
        self.trashed = trashed
        let paths = Set(all.map(\.path))
        selection = selection.filter { paths.contains($0) }
        if let l = lead, !paths.contains(l) { lead = selection.first }
    }

    // MARK: Showing

    var isReorderable: Bool { sort == .manual && query.trimmingCharacters(in: .whitespaces).isEmpty }

    /// The canvases the grid shows: filtered by the search, in the chosen order.
    var visible: [CanvasInfo] {
        var list = all
        let q = query.trimmingCharacters(in: .whitespaces)
        if !q.isEmpty { list = list.filter { $0.name.localizedCaseInsensitiveContains(q) } }
        switch sort {
        case .manual: return list
        case .name: return list.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        case .modified: return list.sorted { $0.modified > $1.modified }
        }
    }

    func canvas(at path: String) -> CanvasInfo? { all.first { $0.path == path } }

    func isOpen(_ canvas: CanvasInfo) -> Bool { SpriteModel.shared.slot(for: canvas.url) != nil }

    func showToast(_ message: String, actionTitle: String? = nil, action: (() -> Void)? = nil) {
        let t = Toast(message: message, actionTitle: actionTitle, action: action)
        toast = t
        toastTimer?.invalidate()
        toastTimer = Timer.scheduledTimer(withTimeInterval: 7, repeats: false) { [weak self] _ in
            if self?.toast?.id == t.id { self?.toast = nil }
        }
    }

    // MARK: Selection

    func select(_ canvas: CanvasInfo, command: Bool = false, shift: Bool = false) {
        let list = visible
        if shift, let anchor = lead, let a = list.firstIndex(where: { $0.path == anchor }),
           let b = list.firstIndex(where: { $0.path == canvas.path }) {
            selection = Set(list[min(a, b)...max(a, b)].map(\.path))
            return
        }
        if command {
            if selection.contains(canvas.path) { selection.remove(canvas.path) } else { selection.insert(canvas.path) }
        } else {
            selection = [canvas.path]
        }
        lead = canvas.path
    }

    /// The canvases an action on `canvas` applies to: the whole selection when it is part of it, else just that one.
    func targets(for canvas: CanvasInfo) -> [CanvasInfo] {
        selection.contains(canvas.path) ? visible.filter { selection.contains($0.path) } : [canvas]
    }

    var selectedCanvases: [CanvasInfo] { visible.filter { selection.contains($0.path) } }

    /// Moves the selection by `delta` cards (1 = next, `columns` = the row below).
    func move(by delta: Int) {
        let list = visible
        guard !list.isEmpty else { return }
        let from = lead.flatMap { l in list.firstIndex { $0.path == l } } ?? (delta > 0 ? -1 : list.count)
        let to = min(max(from + delta, 0), list.count - 1)
        selection = [list[to].path]
        lead = list[to].path
    }

    // MARK: Keyboard

    /// Handles a key in the project view: arrows move, Return opens, Delete moves to Recently deleted, F2 renames,
    /// Escape clears the selection, ⌘A selects all, ⌘D duplicates. Returns whether the key was used.
    @discardableResult
    func handleKey(code: UInt16, flags: NSEvent.ModifierFlags) -> Bool {
        guard renaming == nil else { return false }
        let mods = flags.intersection([.command, .shift, .control, .option])
        switch (code, mods) {
        case (123, []): move(by: -1)
        case (124, []): move(by: 1)
        case (125, []): move(by: columns)
        case (126, []): move(by: -columns)
        case (36, []), (76, []), (125, .command): openSelected()
        case (51, []), (117, []), (51, .command): trashSelected()
        case (120, []): if selection.count == 1 { renaming = selection.first } else { return false }
        case (53, []): selection = []
        case (0, .command): selection = Set(visible.map(\.path))
        case (2, .command): selectedCanvases.forEach { duplicate($0) }
        default: return false
        }
        return true
    }

    // MARK: Actions

    func open(_ canvas: CanvasInfo) {
        RecentProjects.shared.add(folder)
        SpriteModel.shared.open(canvas.url)
    }

    func openSelected() { selectedCanvases.forEach(open) }

    /// Makes a blank canvas in the project, selects it and (by default) opens it in the editor.
    @discardableResult
    func newCanvas(name: String, width: Int, height: Int, open openIt: Bool = false) -> URL? {
        guard let url = ProjectStore.newCanvas(in: folder, name: name, width: width, height: height) else {
            SpriteModel.shared.errorMessage = "Could not create a canvas in “\(self.name)”. \(ProjectStore.lastError)"
            return nil
        }
        var names = all.map(\.fileName)
        names.append(url.lastPathComponent)
        ProjectStore.setOrder(folder, names)
        query = ""
        reloadNow()
        selectOnly(url)
        if openIt { SpriteModel.shared.open(url); RecentProjects.shared.add(folder) }
        return url
    }

    private func selectOnly(_ url: URL) {
        let path = all.first { $0.url.resolvingSymlinksInPath() == url.resolvingSymlinksInPath() }?.path ?? url.path
        selection = [path]
        lead = path
    }

    @discardableResult
    func duplicate(_ canvas: CanvasInfo) -> URL? {
        guard let copy = ProjectStore.duplicate(canvas.url) else {
            SpriteModel.shared.errorMessage = "Could not duplicate “\(canvas.name)”. \(ProjectStore.lastError)"
            return nil
        }
        var names = all.map(\.fileName)
        names.insert(copy.lastPathComponent, at: (names.firstIndex(of: canvas.fileName) ?? names.count - 1) + 1)
        ProjectStore.setOrder(folder, names)
        reloadNow()
        selectOnly(copy)
        return copy
    }

    /// Renames the canvas file (keeping its extension). A tab showing it follows the new name.
    @discardableResult
    func rename(_ canvas: CanvasInfo, to newName: String) -> URL? {
        renaming = nil
        let clean = ProjectStore.cleanName(newName, fallback: "")
        guard !clean.isEmpty, clean != canvas.name else { return nil }
        guard let url = ProjectStore.rename(canvas.url, to: clean) else {
            SpriteModel.shared.errorMessage = "Could not rename “\(canvas.name)” to “\(clean)”. A canvas with that name may already exist."
            return nil
        }
        SpriteModel.shared.canvasFileMoved(from: canvas.url, to: url)
        ProjectStore.setOrder(folder, all.map { $0.fileName == canvas.fileName ? url.lastPathComponent : $0.fileName })
        reloadNow()
        selectOnly(url)
        return url
    }

    /// Moves canvases to the project's trash, with an Undo in a toast. A tab that shows one keeps its pixels but
    /// loses its file, so saving asks where to put it.
    func trash(_ canvases: [CanvasInfo]) {
        guard !canvases.isEmpty else { return }
        let before = visible
        let orderBefore = all.map(\.fileName)
        var moved: [URL] = []
        for c in canvases {
            if let t = ProjectStore.trash(c.url) {
                moved.append(t)
                SpriteModel.shared.canvasFileMoved(from: c.url, to: nil)
            } else {
                SpriteModel.shared.errorMessage = "Could not move “\(c.name)” to Recently deleted. \(ProjectStore.lastError)"
            }
        }
        guard !moved.isEmpty else { return }
        // Select the card that takes the first one's place, so Delete can be pressed again.
        let gone = Set(canvases.map(\.path))
        let first = before.firstIndex { gone.contains($0.path) } ?? 0
        let next = (before[first...].first { !gone.contains($0.path) } ?? before[..<first].last { !gone.contains($0.path) })?.path
        reloadNow()
        selection = next.map { [$0] } ?? []
        lead = next
        let message = canvases.count == 1 ? "Moved “\(canvases[0].name)” to Recently deleted" : "Moved \(moved.count) canvases to Recently deleted"
        showToast(message, actionTitle: "Undo") { [weak self] in
            guard let self else { return }
            moved.forEach { self.restoreFile($0) }
            self.reloadNow()
            // Back where they were: the saved order may have dropped the trashed names.
            let now = Set(self.all.map(\.fileName))
            let order = orderBefore.filter(now.contains) + self.all.map(\.fileName).filter { !orderBefore.contains($0) }
            if order != self.all.map(\.fileName) {
                ProjectStore.setOrder(self.folder, order)
                self.reloadNow()
            }
            self.toast = nil
        }
    }

    func trashSelected() { trash(selectedCanvases) }

    @discardableResult
    private func restoreFile(_ trashed: URL) -> URL? {
        guard let url = ProjectStore.restore(trashed) else {
            SpriteModel.shared.errorMessage = "Could not restore “\(trashed.deletingPathExtension().lastPathComponent)”. \(ProjectStore.lastError)"
            return nil
        }
        return url
    }

    func restore(_ item: TrashedCanvas) {
        guard let url = restoreFile(item.url) else { return }
        reloadNow()
        selectOnly(url)
        toast = nil
    }

    /// Imports .aseprite / .ase / .png files as new canvases.
    @discardableResult
    func importFiles(_ files: [URL]) -> [URL] {
        var made: [URL] = []
        var names = all.map(\.fileName)
        for file in files where ProjectStore.importExtensions.contains(file.pathExtension.lowercased()) {
            if let url = ProjectStore.importFile(into: folder, file: file) {
                made.append(url)
                names.append(url.lastPathComponent)
            } else {
                SpriteModel.shared.errorMessage = "Could not import “\(file.lastPathComponent)”. \(ProjectStore.lastError)"
            }
        }
        guard let last = made.last else { return [] }
        ProjectStore.setOrder(folder, names)
        reloadNow()
        selection = Set(made.compactMap { u in all.first { $0.url.resolvingSymlinksInPath() == u.resolvingSymlinksInPath() }?.path })
        lead = all.first { $0.url.resolvingSymlinksInPath() == last.resolvingSymlinksInPath() }?.path
        showToast(made.count == 1 ? "Imported “\(made[0].deletingPathExtension().lastPathComponent)”" : "Imported \(made.count) files")
        return made
    }

    func importPanel() {
        guard !MainWindow.hasSheet else { return }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.allowedContentTypes = SpriteModel.asepriteTypes + [.png]
        panel.prompt = "Import"
        panel.message = "Choose Aseprite files or PNG images to add to “\(name)” as canvases."
        if panel.runModal() == .OK { importFiles(panel.urls) }
    }

    func exportAseprite(_ canvas: CanvasInfo) {
        guard !MainWindow.hasSheet else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = SpriteModel.asepriteTypes.prefix(1).map { $0 }
        panel.nameFieldStringValue = canvas.name + ".aseprite"
        panel.message = "Saves a copy of “\(canvas.name)” as an Aseprite file."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if !ProjectStore.exportAseprite(canvas.url, to: url) {
            SpriteModel.shared.errorMessage = "Could not export “\(canvas.name)”. \(ProjectStore.lastError)"
        }
    }

    func renameProject(_ newName: String) {
        let clean = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty, clean != name else { return }
        if ProjectStore.setName(folder, clean) { name = clean } else {
            SpriteModel.shared.errorMessage = "Could not rename the project."
        }
    }

    /// Drop of a dragged card on another: the card moves there and the order is saved.
    func dropDragged(on target: String) {
        defer { dragging = nil; dropTarget = nil }
        guard isReorderable, let dragged = dragging, dragged != target else { return }
        move(dragged, to: target)
    }

    func move(_ dragged: String, to target: String) {
        var list = all
        guard let from = list.firstIndex(where: { $0.path == dragged }), let to = list.firstIndex(where: { $0.path == target }) else { return }
        let item = list.remove(at: from)
        list.insert(item, at: to)
        all = list
        ProjectStore.setOrder(folder, list.map(\.fileName))
    }

    // MARK: Key monitor

    private var monitor: Any?

    func installKeys() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] e in
            guard let self, AppRoute.shared.screen == .project(folder), MainWindow.windowIsKey, !MainWindow.hasSheet,
                  !(NSApp.keyWindow?.firstResponder is NSText) else { return e }
            return handleKey(code: e.keyCode, flags: e.modifierFlags) ? nil : e
        }
    }

    func removeKeys() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }
}

// MARK: - Project view

struct ProjectView: View {
    let folder: URL
    @ObservedObject private var model: ProjectBrowserModel
    @ObservedObject private var route = AppRoute.shared
    @State private var gridWidth: CGFloat = 900
    @State private var trashOpen = false

    private static let cardWidth: CGFloat = 188
    private static let spacing: CGFloat = 16
    private static let inset: CGFloat = 24

    init(folder: URL) {
        self.folder = folder
        _model = ObservedObject(wrappedValue: ProjectBrowserModel.shared(for: folder))
    }

    var body: some View {
        VStack(spacing: 0) {
            ProjectHeader(model: model)
            ZStack(alignment: .bottom) {
                content
                if let toast = model.toast {
                    DSToast(message: toast.message, actionTitle: toast.actionTitle, action: toast.action)
                        .padding(.bottom, DS.Space.lg)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .animation(DS.Anim.quick, value: model.toast?.id)
            if !model.trashed.isEmpty { TrashSection(model: model, open: $trashOpen) }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(DSColor.browse)
        .onAppear {
            model.reload()
            model.installKeys()
        }
        .onDisappear { model.removeKeys() }
        .onDrop(of: [.fileURL], delegate: ImportDropDelegate(model: model))
    }

    @ViewBuilder
    private var content: some View {
        if !model.loaded {
            ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if model.missing {
            ProjectNotice(symbol: "exclamationmark.triangle", title: "This project can’t be found",
                          message: "The folder “\(folder.lastPathComponent)” was moved or deleted.") {
                Button("Back to Home") { route.showHome() }.buttonStyle(DSPrimaryButtonStyle())
            }
        } else if model.all.isEmpty {
            ProjectNotice(symbol: "square.dashed", title: "No canvases yet",
                          message: "Make a blank canvas, or import an Aseprite file or a PNG.") {
                HStack(spacing: DS.Space.sm) {
                    Button("Import…") { model.importPanel() }.buttonStyle(DSSecondaryButtonStyle())
                    Button("New Canvas") { route.requestNewCanvas(in: folder) }.buttonStyle(DSPrimaryButtonStyle())
                }
            }
        } else if model.visible.isEmpty {
            ProjectNotice(symbol: "magnifyingglass", title: "Nothing matches “\(model.query)”", message: nil) { EmptyView() }
        } else {
            grid
        }
    }

    private var grid: some View {
        let columns = [GridItem(.adaptive(minimum: Self.cardWidth, maximum: Self.cardWidth), spacing: Self.spacing, alignment: .top)]
        return ScrollViewReader { proxy in
            ScrollView {
                LazyVGrid(columns: columns, alignment: .leading, spacing: Self.spacing) {
                    ForEach(model.visible) { canvas in
                        CanvasCard(model: model, canvas: canvas, width: Self.cardWidth)
                            .id(canvas.path)
                    }
                }
                .padding(Self.inset)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(GeometryReader { geo in
                    Color.clear.preference(key: GridWidthKey.self, value: geo.size.width)
                })
                // Clicking the empty space around the cards clears the selection.
                .background(Color.clear.contentShape(Rectangle()).onTapGesture { model.selection = [] })
            }
            .onPreferenceChange(GridWidthKey.self) { width in
                gridWidth = width
                model.columns = max(1, Int((width - Self.inset * 2 + Self.spacing) / (Self.cardWidth + Self.spacing)))
            }
            .onChange(of: model.lead) { id in
                if let id { withAnimation(DS.Anim.quick) { proxy.scrollTo(id) } }
            }
        }
    }
}

private struct GridWidthKey: PreferenceKey {
    static var defaultValue: CGFloat = 900
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

/// Files dragged in from Finder become canvases of the project.
private struct ImportDropDelegate: DropDelegate {
    let model: ProjectBrowserModel

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: [.fileURL]) }

    func performDrop(info: DropInfo) -> Bool {
        let model = model
        let providers = info.itemProviders(for: [.fileURL])
        let group = DispatchGroup()
        var urls: [URL] = []
        let lock = NSLock()
        for provider in providers {
            group.enter()
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                if let url { lock.lock(); urls.append(url); lock.unlock() }
                group.leave()
            }
        }
        group.notify(queue: .main) { model.importFiles(urls) }
        return true
    }
}

// MARK: Header

private struct ProjectHeader: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject private var route = AppRoute.shared
    @State private var editing = false
    @State private var draft = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: DS.Space.sm) {
            IconButton("chevron.left", help: "Home", shortcut: "⇧⌘H") { route.showHome() }
            if editing {
                TextField("Project name", text: $draft)
                    .textFieldStyle(.plain)
                    .font(.system(size: 20, weight: .semibold))
                    .focused($focused)
                    .onSubmit(commit)
                    .onExitCommand { editing = false }
                    .onChange(of: focused) { if !$0 { commit() } }
                    .frame(maxWidth: 360)
            } else {
                Text(model.name).font(.system(size: 20, weight: .semibold)).foregroundStyle(DSColor.text)
                    .lineLimit(1).truncationMode(.tail)
                    .onTapGesture(count: 2) { begin() }
                    .help("Double-click to rename the project")
                    .accessibilityAddTraits(.isHeader)
            }
            Text("\(model.all.count) canvas\(model.all.count == 1 ? "" : "es")")
                .font(DS.Font.captionMedium).foregroundStyle(DSColor.textSecondary)
                .padding(.horizontal, DS.Space.sm).frame(height: 20)
                .background(Capsule().fill(DSColor.field))
            IconButton("pencil", help: "Rename project", compact: true) { begin() }
            Spacer(minLength: DS.Space.lg)
            DSSearchField(text: $model.query, prompt: "Search canvases", width: 200)
            DSPopup(selection: $model.sort, options: CanvasSort.allCases.map { ($0, $0.title) }, help: "Sort canvases")
            IconButton("folder", help: "Show in Finder", compact: true) { NSWorkspace.shared.activateFileViewerSelecting([model.folder]) }
            Button { model.importPanel() } label: {
                HStack(spacing: 6) {
                    Image(systemName: "square.and.arrow.down").font(.system(size: 12, weight: .medium))
                    Text("Import…")
                }
            }
            .buttonStyle(DSSecondaryButtonStyle())
            .help("Add Aseprite files or PNG images as canvases")
            Button { route.requestNewCanvas(in: model.folder) } label: {
                HStack(spacing: 6) {
                    Image(systemName: "plus").font(.system(size: 11, weight: .bold))
                    Text("New Canvas")
                }
            }
            .buttonStyle(DSPrimaryButtonStyle())
            .help("Make a blank canvas in this project")
        }
        .padding(.horizontal, DS.Space.xl)
        .frame(height: 64)
    }

    private func begin() {
        draft = model.name
        editing = true
        focused = true
    }

    private func commit() {
        guard editing else { return }
        editing = false
        model.renameProject(draft)
    }
}

// MARK: Cards

struct CanvasCard: View {
    @ObservedObject var model: ProjectBrowserModel
    let canvas: CanvasInfo
    let width: CGFloat
    @State private var hover = false
    @State private var draft = ""
    @FocusState private var nameFocused: Bool

    private var selected: Bool { model.selection.contains(canvas.path) }
    private var editing: Bool { model.renaming == canvas.path }

    private var dropEdge: Edge.Set? {
        guard model.dropTarget == canvas.path, let dragged = model.dragging,
              let from = model.all.firstIndex(where: { $0.path == dragged }),
              let to = model.all.firstIndex(where: { $0.path == canvas.path }) else { return nil }
        return from < to ? .trailing : .leading
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            CanvasThumb(url: canvas.url, key: canvas.thumbnailKey, padding: 12).frame(width: width, height: 132)
                .overlay(alignment: .topLeading) { badge }
                .overlay(alignment: .topTrailing) {
                    if model.isOpen(canvas) {
                        Text("Open").font(.system(size: 9, weight: .semibold)).foregroundStyle(.white)
                            .padding(.horizontal, 5).frame(height: 15)
                            .background(Capsule().fill(DSColor.accent)).padding(6)
                    }
                }
            VStack(alignment: .leading, spacing: 2) {
                if editing {
                    TextField("Name", text: $draft)
                        .textFieldStyle(.plain)
                        .font(DS.Font.bodyLarge.weight(.medium))
                        .focused($nameFocused)
                        .onSubmit { model.rename(canvas, to: draft) }
                        .onExitCommand { model.renaming = nil }
                        .onChange(of: nameFocused) { if !$0, editing { model.rename(canvas, to: draft) } }
                        .padding(.horizontal, 4).frame(height: 18)
                        .dsFieldBackground(focused: true, radius: 3)
                        .onAppear { draft = canvas.name; nameFocused = true }
                } else {
                    Text(canvas.name).font(DS.Font.bodyLarge.weight(.medium)).foregroundStyle(DSColor.text)
                        .lineLimit(1).truncationMode(.middle).frame(height: 18, alignment: .leading)
                }
                HStack(spacing: 4) {
                    Text(canvas.sizeText).font(DS.Font.number)
                    Text("·")
                    Text(Ago.text(canvas.modified)).lineLimit(1)
                }
                .font(DS.Font.caption).foregroundStyle(DSColor.textSecondary)
            }
            .padding(.horizontal, DS.Space.md - 2)
            .padding(.vertical, DS.Space.sm)
            .frame(width: width, alignment: .leading)
        }
        .frame(width: width)
        .cardSurface(selected: selected, hovering: hover, dropEdge: dropEdge)
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture(count: 2) { model.open(canvas) }
        .onTapGesture {
            NSApp.keyWindow?.makeFirstResponder(nil)
            let flags = NSEvent.modifierFlags
            model.select(canvas, command: flags.contains(.command), shift: flags.contains(.shift))
        }
        .contextMenu { menu }
        .onDrag {
            model.dragging = canvas.path
            return NSItemProvider(object: "pixmith-canvas:\(canvas.fileName)" as NSString)
        } preview: {
            CanvasThumb(url: canvas.url, key: canvas.thumbnailKey, padding: 6, cell: 6).frame(width: 96, height: 72)
                .clipShape(RoundedRectangle(cornerRadius: DS.Radius.control))
        }
        .onDrop(of: [.text], delegate: CardDropDelegate(model: model, target: canvas.path))
        .help("\(canvas.fileName) — \(canvas.sizeText)")
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Canvas \(canvas.name), \(canvas.sizeText)")
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    @ViewBuilder
    private var badge: some View {
        if canvas.error != nil {
            Label("Unreadable", systemImage: "exclamationmark.triangle.fill").font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.white).padding(.horizontal, 5).frame(height: 15)
                .background(Capsule().fill(DSColor.destructive)).padding(6)
        } else if canvas.format == "aseprite" {
            Text("ASE").font(.system(size: 9, weight: .semibold)).foregroundStyle(DSColor.textSecondary)
                .padding(.horizontal, 5).frame(height: 15)
                .background(Capsule().fill(DSColor.overlay)).padding(6)
                .help("An Aseprite file; Save writes it back as .aseprite")
        }
    }

    @ViewBuilder
    private var menu: some View {
        let targets = model.targets(for: canvas)
        Button("Open") { targets.forEach(model.open) }
        Divider()
        Button("Rename") { model.select(canvas); model.renaming = canvas.path }.disabled(targets.count != 1)
        Button("Duplicate") { targets.forEach { model.duplicate($0) } }
        Button(targets.count > 1 ? "Move \(targets.count) to Recently Deleted" : "Delete") { model.trash(targets) }
        Divider()
        Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting(targets.map(\.url)) }
        Button("Export As Aseprite…") { model.exportAseprite(canvas) }.disabled(targets.count != 1 || canvas.error != nil)
    }
}

private struct CardDropDelegate: DropDelegate {
    let model: ProjectBrowserModel
    let target: String

    func validateDrop(info: DropInfo) -> Bool { model.dragging != nil && model.isReorderable && info.hasItemsConforming(to: [.text]) }

    func dropEntered(info: DropInfo) { if model.dragging != target { model.dropTarget = target } }

    func dropExited(info: DropInfo) { if model.dropTarget == target { model.dropTarget = nil } }

    func dropUpdated(info: DropInfo) -> DropProposal? { DropProposal(operation: .move) }

    func performDrop(info: DropInfo) -> Bool {
        withAnimation(DS.Anim.quick) { model.dropDragged(on: target) }
        return true
    }
}

// MARK: Notices

private struct ProjectNotice<Actions: View>: View {
    let symbol: String
    let title: String
    let message: String?
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(spacing: DS.Space.lg) {
            Image(systemName: symbol).font(.system(size: 30, weight: .light)).foregroundStyle(DSColor.textTertiary)
            VStack(spacing: DS.Space.xs) {
                Text(title).font(.system(size: 15, weight: .semibold)).foregroundStyle(DSColor.text)
                if let message {
                    Text(message).font(DS.Font.body).foregroundStyle(DSColor.textSecondary).multilineTextAlignment(.center)
                }
            }
            actions
        }
        .frame(maxWidth: 360)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: Recently deleted

private struct TrashSection: View {
    @ObservedObject var model: ProjectBrowserModel
    @Binding var open: Bool

    var body: some View {
        VStack(spacing: 0) {
            DSDivider()
            Button { withAnimation(DS.Anim.quick) { open.toggle() } } label: {
                HStack(spacing: DS.Space.sm) {
                    Image(systemName: "chevron.right").font(.system(size: 8, weight: .bold)).foregroundStyle(DSColor.textTertiary)
                        .rotationEffect(.degrees(open ? 90 : 0)).frame(width: 12)
                    Image(systemName: "trash").font(.system(size: 11)).foregroundStyle(DSColor.textSecondary)
                    Text("Recently deleted").font(DS.Font.section).foregroundStyle(DSColor.text)
                    Text("\(model.trashed.count)").font(DS.Font.number).foregroundStyle(DSColor.textTertiary)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, DS.Space.xl)
                .frame(height: 36)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(model.trashed) { item in
                            HStack(spacing: DS.Space.md) {
                                CanvasThumb(url: item.url, key: "trash|" + item.path, padding: 2, cell: 4).frame(width: 32, height: 32)
                                    .clipShape(RoundedRectangle(cornerRadius: DS.Radius.chip))
                                Text(item.name).font(DS.Font.bodyLarge).foregroundStyle(DSColor.text).lineLimit(1)
                                Text("deleted " + Ago.text(item.deleted)).font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                                Spacer(minLength: 0)
                                Button("Restore") { model.restore(item) }.buttonStyle(DSBarButtonStyle())
                            }
                            .padding(.horizontal, DS.Space.xl)
                            .frame(height: 44)
                        }
                    }
                }
                .frame(maxHeight: 180)
            }
        }
        .background(DSColor.panel)
    }
}
