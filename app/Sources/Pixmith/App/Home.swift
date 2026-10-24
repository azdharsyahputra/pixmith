import AppKit
import SwiftUI
import UniformTypeIdentifiers

// The Home screen and the app's navigation between Home, a project and the editor.

// MARK: - Route

/// Which screen the main window shows. The open documents live in `SpriteModel` whatever the screen is, so going Home
/// and back loses nothing.
enum Screen: Equatable {
    case home
    case project(URL)
    case editor
}

enum HomeSheet: Identifiable {
    case newProject
    case newCanvas(URL)

    var id: String {
        switch self {
        case .newProject: "new-project"
        case .newCanvas(let folder): "new-canvas:" + folder.path
        }
    }
}

final class AppRoute: ObservableObject {
    static let shared = AppRoute()

    @Published var screen: Screen = .home
    @Published var sheet: HomeSheet?
    /// Bumped whenever Home appears, so project cards refresh.
    @Published var homeVisits = 0

    var isEditor: Bool { screen == .editor }

    /// Shows the document editor (opening or creating a document does this).
    func enterEditor() {
        if screen != .editor { screen = .editor }
    }

    func showHome(_ model: SpriteModel = .shared) {
        model.finishTransients()
        homeVisits += 1
        screen = .home
    }

    func showProject(_ folder: URL) {
        SpriteModel.shared.finishTransients()
        RecentProjects.shared.add(folder)
        screen = .project(folder.standardizedFileURL)
    }

    /// Back to the editor (only when there is something to edit).
    func showEditor() { screen = .editor }

    var title: String {
        switch screen {
        case .home: "Pixmith"
        case .project(let folder): ProjectBrowserModel.shared(for: folder).name
        case .editor: SpriteModel.shared.windowTitle
        }
    }

    // MARK: Projects

    func requestNewProject() {
        guard !MainWindow.hasSheet else { return }
        sheet = .newProject
    }

    func requestNewCanvas(in folder: URL) {
        guard !MainWindow.hasSheet else { return }
        sheet = .newCanvas(folder)
    }

    /// Asks for a folder and opens it as a project.
    func openProjectPanel() {
        guard !MainWindow.hasSheet else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Open Project"
        panel.message = "Choose a project folder."
        if panel.runModal() == .OK, let url = panel.url { openProject(url) }
    }

    /// Opens a project folder. A folder that is not a project yet can be turned into one.
    @discardableResult
    func openProject(_ folder: URL) -> Bool {
        let folder = folder.standardizedFileURL
        if !ProjectStore.isProject(folder) {
            let alert = NSAlert()
            alert.messageText = "“\(folder.lastPathComponent)” is not a Pixmith project yet."
            alert.informativeText = "Make it a project to browse its canvases here. Your files stay where they are."
            alert.addButton(withTitle: "Make Project")
            alert.addButton(withTitle: "Cancel")
            guard SpriteModel.alertRunner(alert) == .alertFirstButtonReturn else { return false }
            guard ProjectStore.create(folder, name: folder.lastPathComponent) else {
                SpriteModel.shared.errorMessage = "Could not make “\(folder.lastPathComponent)” a project."
                return false
            }
        }
        showProject(folder)
        return true
    }

    /// Makes the folder `location/name`, makes it a project and opens it. Reports problems in the error alert.
    @discardableResult
    func createProject(name: String, in location: URL) -> URL? {
        let clean = ProjectStore.cleanName(name, fallback: "Untitled Project")
        let folder = location.appendingPathComponent(clean, isDirectory: true).standardizedFileURL
        if ProjectStore.isProject(folder) {
            SpriteModel.shared.errorMessage = "“\(clean)” is already a project in that folder. Pick another name, or open it."
            return nil
        }
        guard ProjectStore.create(folder, name: clean) else {
            SpriteModel.shared.errorMessage = "Could not create the project at \(folder.path)."
            return nil
        }
        UserDefaults.standard.set(location.path, forKey: Prefs.Key.projectsLocation)
        showProject(folder)
        return folder
    }

    static var defaultLocation: URL {
        if let saved = UserDefaults.standard.string(forKey: Prefs.Key.projectsLocation),
           FileManager.default.fileExists(atPath: saved) { return URL(fileURLWithPath: saved, isDirectory: true) }
        return FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser
    }
}

// MARK: - Home

enum HomeTab: String { case recent, projects }

enum ProjectSort: String, CaseIterable {
    case recent, name, modified
    var title: String {
        switch self {
        case .recent: "Last opened"
        case .name: "Name"
        case .modified: "Modified"
        }
    }
}

/// Shown at launch: the sidebar (Recent, Projects, New Project) and the main area with the recent canvases and the
/// projects as cards with cover thumbnails.
struct HomeView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var route = AppRoute.shared
    @ObservedObject private var recents = RecentProjects.shared
    @ObservedObject private var recentFiles = RecentFiles.shared
    @AppStorage(Prefs.Key.homeTab) private var tabRaw = HomeTab.recent.rawValue
    @AppStorage(Prefs.Key.projectSort) private var sortRaw = ProjectSort.recent.rawValue
    @State private var query = ""
    @State private var summaries: [String: ProjectSummary] = [:]

    private var tab: HomeTab { HomeTab(rawValue: tabRaw) ?? .recent }
    private var sort: ProjectSort { ProjectSort(rawValue: sortRaw) ?? .recent }

    private func summary(_ folder: URL) -> ProjectSummary? { summaries[folder.path] }

    private var projects: [URL] {
        var list = recents.folders
        let q = query.trimmingCharacters(in: .whitespaces)
        if !q.isEmpty {
            list = list.filter { (summary($0)?.name ?? $0.lastPathComponent).localizedCaseInsensitiveContains(q) }
        }
        guard tab == .projects else { return Array(list.prefix(12)) }
        switch sort {
        case .recent: return list
        case .name:
            return list.sorted { (summary($0)?.name ?? $0.lastPathComponent).localizedStandardCompare(summary($1)?.name ?? $1.lastPathComponent) == .orderedAscending }
        case .modified:
            return list.sorted { (summary($0)?.modified ?? .distantPast) > (summary($1)?.modified ?? .distantPast) }
        }
    }

    private var recentCanvases: [URL] {
        let q = query.trimmingCharacters(in: .whitespaces)
        return recentFiles.existing
            .filter { q.isEmpty || $0.deletingPathExtension().lastPathComponent.localizedCaseInsensitiveContains(q) }
            .prefix(14).map { $0 }
    }

    var body: some View {
        HStack(spacing: 0) {
            HomeSidebar(model: model, tab: Binding(get: { tab }, set: { tabRaw = $0.rawValue }), summaries: summaries)
                .frame(width: 232)
            DSDivider(axis: .vertical)
            VStack(spacing: 0) {
                header
                content
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(DSColor.browse)
        }
        .background(DSColor.panel)
        .task(id: LoadKey(folders: recents.folders, visits: route.homeVisits)) { await loadSummaries() }
    }

    private struct LoadKey: Equatable { let folders: [URL]; let visits: Int }

    private func loadSummaries() async {
        for folder in recents.folders {
            if Task.isCancelled { return }
            let s = await Task.detached(priority: .utility) { ProjectSummary.load(folder) }.value
            if summaries[folder.path] != s { summaries[folder.path] = s }
        }
    }

    private var header: some View {
        HStack(spacing: DS.Space.sm) {
            Text(tab == .recent ? "Recent" : "Projects").font(.system(size: 20, weight: .semibold)).foregroundStyle(DSColor.text)
            Spacer(minLength: DS.Space.lg)
            DSSearchField(text: $query, prompt: tab == .recent ? "Search recent" : "Search projects", width: 220)
            if tab == .projects {
                DSPopup(selection: Binding(get: { sort }, set: { sortRaw = $0.rawValue }),
                        options: ProjectSort.allCases.map { ($0, $0.title) }, help: "Sort projects")
            }
        }
        .padding(.horizontal, DS.Space.xl)
        .frame(height: 64)
    }

    @ViewBuilder
    private var content: some View {
        let strip = tab == .recent ? recentCanvases : []
        if recents.folders.isEmpty && recentFiles.existing.isEmpty {
            HomeEmptyState()
        } else if projects.isEmpty && strip.isEmpty {
            VStack(spacing: DS.Space.sm) {
                Image(systemName: "magnifyingglass").font(.system(size: 22)).foregroundStyle(DSColor.textTertiary)
                Text("Nothing matches “\(query)”").font(DS.Font.bodyLarge).foregroundStyle(DSColor.textSecondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: DS.Space.xl) {
                    if !strip.isEmpty { recentStrip(strip) }
                    if !projects.isEmpty { projectGrid }
                }
                .padding(.horizontal, DS.Space.xl)
                .padding(.bottom, DS.Space.xl)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private func sectionTitle(_ title: String, count: Int? = nil) -> some View {
        HStack(spacing: DS.Space.sm) {
            Text(title).font(DS.Font.title).foregroundStyle(DSColor.text)
            if let count { Text("\(count)").font(DS.Font.number).foregroundStyle(DSColor.textTertiary) }
        }
    }

    private func recentStrip(_ files: [URL]) -> some View {
        VStack(alignment: .leading, spacing: DS.Space.md) {
            sectionTitle("Recent canvases")
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: DS.Space.lg) {
                    ForEach(files, id: \.self) { url in
                        RecentCanvasCard(url: url) { model.requestOpen(url) }
                    }
                }
                .padding(.vertical, 2)
            }
            .padding(.horizontal, -2)
        }
    }

    private var projectGrid: some View {
        VStack(alignment: .leading, spacing: DS.Space.md) {
            sectionTitle(tab == .recent ? "Recent projects" : "All projects", count: tab == .projects ? projects.count : nil)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 224, maximum: 280), spacing: DS.Space.lg, alignment: .top)],
                      alignment: .leading, spacing: DS.Space.lg) {
                ForEach(projects, id: \.self) { folder in
                    ProjectCard(folder: folder, summary: summary(folder)) { route.showProject(folder) }
                }
            }
        }
    }
}

// MARK: - Empty state

private struct HomeEmptyState: View {
    var body: some View {
        VStack(spacing: DS.Space.lg) {
            Image(systemName: "square.stack.3d.up").font(.system(size: 34, weight: .light)).foregroundStyle(DSColor.textTertiary)
            VStack(spacing: DS.Space.xs) {
                Text("No projects yet").font(.system(size: 15, weight: .semibold)).foregroundStyle(DSColor.text)
                Text("A project is a folder of canvases. Make one, or open a folder you already have.")
                    .font(DS.Font.body).foregroundStyle(DSColor.textSecondary).multilineTextAlignment(.center)
            }
            HStack(spacing: DS.Space.sm) {
                Button("Open Project…") { AppRoute.shared.openProjectPanel() }.buttonStyle(DSSecondaryButtonStyle())
                Button("New Project") { AppRoute.shared.requestNewProject() }.buttonStyle(DSPrimaryButtonStyle())
            }
        }
        .frame(maxWidth: 360)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - Sidebar

private struct HomeSidebar: View {
    @ObservedObject var model: SpriteModel
    @Binding var tab: HomeTab
    let summaries: [String: ProjectSummary]
    @ObservedObject private var recents = RecentProjects.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: DS.Space.sm) {
                Image(systemName: "square.grid.3x3.fill")
                    .font(.system(size: 12, weight: .semibold)).foregroundStyle(.white)
                    .frame(width: 24, height: 24)
                    .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(DSColor.accent))
                Text("Pixmith").font(.system(size: 15, weight: .semibold)).foregroundStyle(DSColor.text)
            }
            .padding(.horizontal, DS.Space.lg)
            .frame(height: 64, alignment: .leading)

            Button { AppRoute.shared.requestNewProject() } label: {
                HStack(spacing: 6) {
                    Image(systemName: "plus").font(.system(size: 11, weight: .bold))
                    Text("New Project")
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(DSPrimaryButtonStyle())
            .padding(.horizontal, DS.Space.md)
            .padding(.bottom, DS.Space.md)
            .help("Make a new project folder")

            VStack(spacing: DS.Space.xxs) {
                SidebarRow(symbol: "clock", title: "Recent", selected: tab == .recent) { tab = .recent }
                SidebarRow(symbol: "square.stack", title: "Projects", selected: tab == .projects) { tab = .projects }
            }
            .padding(.horizontal, DS.Space.sm)

            if !recents.folders.isEmpty {
                Text("RECENT PROJECTS").font(.system(size: 10, weight: .semibold)).foregroundStyle(DSColor.textTertiary)
                    .padding(.horizontal, DS.Space.lg).padding(.top, DS.Space.xl).padding(.bottom, DS.Space.xs)
                ScrollView {
                    VStack(spacing: DS.Space.xxs) {
                        ForEach(recents.folders.prefix(12), id: \.self) { folder in
                            SidebarRow(symbol: "folder", title: summaries[folder.path]?.name ?? folder.lastPathComponent,
                                       selected: false) { AppRoute.shared.showProject(folder) }
                        }
                    }
                    .padding(.horizontal, DS.Space.sm)
                }
            }
            Spacer(minLength: 0)
            DSDivider().opacity(0.6)
            VStack(spacing: DS.Space.xxs) {
                if hasOpenDocuments {
                    SidebarRow(symbol: "pencil.and.outline", title: "Continue Editing", detail: "\(model.slots.count)", selected: false) {
                        AppRoute.shared.showEditor()
                    }
                }
                SidebarRow(symbol: "folder.badge.plus", title: "Open Project…", selected: false) { AppRoute.shared.openProjectPanel() }
                SidebarRow(symbol: "doc", title: "Open File…", selected: false) { model.requestOpen() }
            }
            .padding(DS.Space.sm)
        }
        .frame(maxHeight: .infinity, alignment: .topLeading)
        .background(DSColor.panel)
    }

    /// Documents the user has open or edited: more than the blank one a fresh launch starts with.
    private var hasOpenDocuments: Bool { model.slots.count > 1 || !model.activeIsPristine }
}

private struct SidebarRow: View {
    let symbol: String
    let title: String
    var detail: String?
    let selected: Bool
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: DS.Space.sm) {
                Image(systemName: symbol).font(.system(size: 12, weight: .medium))
                    .foregroundStyle(selected ? DSColor.accent : DSColor.textSecondary).frame(width: 18)
                Text(title).font(DS.Font.bodyLarge).foregroundStyle(DSColor.text).lineLimit(1).truncationMode(.tail)
                Spacer(minLength: 0)
                if let detail { Text(detail).font(DS.Font.number).foregroundStyle(DSColor.textTertiary) }
            }
            .padding(.horizontal, DS.Space.sm)
            .frame(height: DS.Metric.iconButton)
            .background(RoundedRectangle(cornerRadius: DS.Radius.control)
                .fill(selected ? DSColor.selected : hover ? DSColor.hover.opacity(0.7) : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

// MARK: - Cards

/// A project: a cover (its first canvas) on a checkerboard, then its name and a line of facts.
struct ProjectCard: View {
    let folder: URL
    let summary: ProjectSummary?
    let open: () -> Void
    @State private var hover = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            cover.frame(height: 148)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Image(systemName: "folder.fill").font(.system(size: 11)).foregroundStyle(DSColor.textTertiary)
                    Text(summary?.name ?? folder.lastPathComponent).font(DS.Font.bodyLarge.weight(.medium))
                        .foregroundStyle(DSColor.text).lineLimit(1).truncationMode(.tail)
                }
                Text(facts).font(DS.Font.caption).foregroundStyle(DSColor.textSecondary).lineLimit(1)
            }
            .padding(.horizontal, DS.Space.md)
            .padding(.vertical, DS.Space.sm + 2)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .cardSurface(hovering: hover)
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture(perform: open)
        .contextMenu {
            Button("Open") { open() }
            Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([folder]) }
            Divider()
            Button("Remove from Recents") { RecentProjects.shared.remove(folder) }
        }
        .help(folder.path)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Project \(summary?.name ?? folder.lastPathComponent)")
        .accessibilityAddTraits(.isButton)
    }

    private var facts: String {
        guard let summary else { return " " }
        guard summary.exists else { return "Folder not found" }
        let n = summary.canvasCount
        return "\(n) canvas\(n == 1 ? "" : "es") · \(Ago.text(summary.modified))"
    }

    @ViewBuilder
    private var cover: some View {
        if let cover = summary?.cover {
            CanvasThumb(url: cover.url, key: cover.thumbnailKey, padding: 16, cell: 8)
        } else {
            ZStack {
                DSCheckerBackdrop()
                if let summary {
                    VStack(spacing: DS.Space.xs) {
                        Image(systemName: summary.exists ? "square.dashed" : "exclamationmark.triangle")
                            .font(.system(size: 22, weight: .light))
                        Text(summary.exists ? "No canvases yet" : "Missing").font(DS.Font.caption)
                    }
                    .foregroundStyle(DSColor.textTertiary)
                }
            }
        }
    }
}

/// A recently opened file in the strip on top of Home.
struct RecentCanvasCard: View {
    let url: URL
    let open: () -> Void
    @State private var hover = false

    private var stamp: (key: String, modified: Date) {
        let v = try? url.resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey])
        let date = v?.contentModificationDate ?? .distantPast
        return ("\(url.path)|\(Int(date.timeIntervalSince1970))|\(v?.fileSize ?? 0)", date)
    }

    var body: some View {
        let stamp = stamp
        VStack(alignment: .leading, spacing: 0) {
            CanvasThumb(url: url, key: stamp.key, padding: 10, cell: 6).frame(width: 168, height: 112)
            VStack(alignment: .leading, spacing: 2) {
                Text(url.deletingPathExtension().lastPathComponent).font(DS.Font.bodyLarge.weight(.medium))
                    .foregroundStyle(DSColor.text).lineLimit(1).truncationMode(.middle)
                Text("\(url.deletingLastPathComponent().lastPathComponent) · \(Ago.text(stamp.modified))")
                    .font(DS.Font.caption).foregroundStyle(DSColor.textSecondary).lineLimit(1).truncationMode(.tail)
            }
            .padding(.horizontal, DS.Space.md - 2)
            .padding(.vertical, DS.Space.sm)
            .frame(width: 168, alignment: .leading)
        }
        .cardSurface(hovering: hover)
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture(perform: open)
        .contextMenu {
            Button("Open") { open() }
            Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([url]) }
        }
        .help(url.path)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Canvas \(url.deletingPathExtension().lastPathComponent)")
        .accessibilityAddTraits(.isButton)
    }
}

// MARK: - New project, new canvas

struct NewProjectSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State private var name = "Untitled Project"
    @State private var location = AppRoute.defaultLocation

    private var clean: String { ProjectStore.cleanName(name, fallback: "") }
    private var destination: String { (location.path as NSString).abbreviatingWithTildeInPath + "/" + clean }

    var body: some View {
        DSDialog(title: "New Project", subtitle: "A project is a folder of canvases.", primary: "Create",
                 primaryEnabled: !clean.isEmpty, width: 400, onPrimary: {
            if AppRoute.shared.createProject(name: name, in: location) != nil { dismiss() }
        }, onCancel: { dismiss() }) {
            DSFormRow(label: "Name", labelWidth: 64) { TextField("Project name", text: $name).dsTextField() }
            DSFormRow(label: "Location", labelWidth: 64) {
                HStack(spacing: DS.Space.sm) {
                    HStack(spacing: 6) {
                        Image(systemName: "folder").font(.system(size: 11)).foregroundStyle(DSColor.textTertiary)
                        Text((location.path as NSString).abbreviatingWithTildeInPath).font(DS.Font.body)
                            .foregroundStyle(DSColor.text).lineLimit(1).truncationMode(.middle)
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, DS.Space.sm)
                    .frame(height: DS.Metric.control)
                    .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
                    Button("Choose…") { choose() }.buttonStyle(DSBarButtonStyle())
                }
            }
            DSNote("Creates \(destination)")
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.directoryURL = location
        panel.prompt = "Choose"
        panel.message = "Choose where the project folder is created."
        if panel.runModal() == .OK, let url = panel.url { location = url }
    }
}

struct NewCanvasSheet: View {
    let folder: URL
    @Environment(\.dismiss) private var dismiss
    @State private var name = "Canvas"
    @State private var width = Prefs.newWidth
    @State private var height = Prefs.newHeight
    @State private var openAfter = true
    private static let presets = [16, 32, 64, 128]

    var body: some View {
        DSDialog(title: "New Canvas", subtitle: "In “\(ProjectBrowserModel.shared(for: folder).name)”", primary: "Create",
                 width: 340, onPrimary: {
            ProjectBrowserModel.shared(for: folder).newCanvas(name: name, width: min(max(width, 1), 4096),
                                                              height: min(max(height, 1), 4096), open: openAfter)
            dismiss()
        }, onCancel: { dismiss() }) {
            DSFormRow(label: "Name") { TextField("Canvas", text: $name).dsTextField() }
            DSFormRow(label: "Width") { DSNumberField("W", value: $width, range: 1...4096, unit: "px", width: 120) }
            DSFormRow(label: "Height") { DSNumberField("H", value: $height, range: 1...4096, unit: "px", width: 120) }
            HStack(spacing: DS.Space.xs) {
                ForEach(Self.presets, id: \.self) { n in
                    Button("\(n)") { width = n; height = n }.buttonStyle(DSBarButtonStyle())
                        .help("\(n) × \(n) px")
                }
            }
            .padding(.leading, 92 + DS.Space.sm)
            DSChip("Open in the editor after creating", symbol: "pencil", help: "Open the new canvas in a tab", isOn: $openAfter)
        }
    }
}
