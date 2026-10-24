import AppKit
import SwiftUI

/// At the left of the editor's title row: a Home button, and, for a document that lives in a project, the project's name
/// with a menu of its canvases to open in a tab.
struct ProjectSwitcher: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        HStack(spacing: DS.Space.xs) {
            IconButton("house", help: "Home", shortcut: "⇧⌘H") { AppRoute.shared.showHome(model) }
            if let folder = ProjectStore.projectFolder(of: model.fileURL) {
                ProjectChip(model: model, folder: folder)
            }
            DSGroupDivider()
        }
    }
}

private struct ProjectChip: View {
    @ObservedObject var model: SpriteModel
    let folder: URL
    @ObservedObject private var project: ProjectBrowserModel
    @State private var open = false
    @State private var hover = false

    init(model: SpriteModel, folder: URL) {
        self.model = model
        self.folder = folder
        _project = ObservedObject(wrappedValue: ProjectBrowserModel.shared(for: folder))
    }

    var body: some View {
        Button { open.toggle() } label: {
            HStack(spacing: 6) {
                Image(systemName: "folder.fill").font(.system(size: 11)).foregroundStyle(DSColor.textSecondary)
                Text(project.name.count > 24 ? String(project.name.prefix(23)) + "…" : project.name).font(DS.Font.bodyLarge.weight(.medium)).foregroundStyle(DSColor.text)
                    .lineLimit(1)
                Image(systemName: "chevron.up.chevron.down").font(.system(size: 8, weight: .bold)).foregroundStyle(DSColor.textTertiary)
            }
            .padding(.horizontal, DS.Space.sm + 2)
            .frame(height: DS.Metric.iconButton)
            .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(open || hover ? DSColor.hover : DSColor.field))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .help("Switch canvas in “\(project.name)”")
        .accessibilityLabel("Project \(project.name), switch canvas")
        .popover(isPresented: $open, arrowEdge: .bottom) {
            CanvasSwitcherList(model: model, project: project) { open = false }
        }
        .onAppear { project.reload() }
    }
}

/// The popover: the project's canvases with thumbnails; a click opens one in a tab.
struct CanvasSwitcherList: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var project: ProjectBrowserModel
    let dismiss: () -> Void
    @State private var query = ""

    private var canvases: [CanvasInfo] {
        let q = query.trimmingCharacters(in: .whitespaces)
        return q.isEmpty ? project.all : project.all.filter { $0.name.localizedCaseInsensitiveContains(q) }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: DS.Space.sm) {
                Text(project.name).font(DS.Font.title).foregroundStyle(DSColor.text).lineLimit(1)
                Text("\(project.all.count)").font(DS.Font.number).foregroundStyle(DSColor.textTertiary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, DS.Space.md).frame(height: 36)
            if project.all.count > 7 {
                DSSearchField(text: $query, prompt: "Search canvases", width: nil)
                    .padding(.horizontal, DS.Space.sm).padding(.bottom, DS.Space.sm)
            }
            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(canvases) { canvas in
                        SwitcherRow(model: model, canvas: canvas) {
                            dismiss()
                            project.open(canvas)
                        }
                    }
                    if canvases.isEmpty {
                        Text(project.all.isEmpty ? "No canvases yet" : "Nothing matches")
                            .font(DS.Font.body).foregroundStyle(DSColor.textTertiary).padding(DS.Space.lg)
                    }
                }
                .padding(.horizontal, DS.Space.xs)
            }
            .frame(maxHeight: 320)
            DSDivider().opacity(0.6)
            VStack(spacing: 2) {
                FooterRow(symbol: "plus", title: "New Canvas…") {
                    dismiss()
                    AppRoute.shared.requestNewCanvas(in: project.folder)
                }
                FooterRow(symbol: "square.grid.2x2", title: "All Canvases") {
                    dismiss()
                    AppRoute.shared.showProject(project.folder)
                }
                FooterRow(symbol: "house", title: "Back to Home") {
                    dismiss()
                    AppRoute.shared.showHome(model)
                }
            }
            .padding(DS.Space.xs)
        }
        .frame(width: 300)
        .background(DSColor.elevated)
        .onAppear { project.reload() }
    }
}

private struct SwitcherRow: View {
    @ObservedObject var model: SpriteModel
    let canvas: CanvasInfo
    let action: () -> Void
    @State private var hover = false

    private var current: Bool { model.fileURL?.resolvingSymlinksInPath() == canvas.url.resolvingSymlinksInPath() }
    private var openInTab: Bool { model.slot(for: canvas.url) != nil }

    var body: some View {
        Button(action: action) {
            HStack(spacing: DS.Space.sm + 2) {
                CanvasThumb(url: canvas.url, key: canvas.thumbnailKey, padding: 2, cell: 4).frame(width: 32, height: 32)
                    .clipShape(RoundedRectangle(cornerRadius: DS.Radius.chip))
                VStack(alignment: .leading, spacing: 1) {
                    Text(canvas.name).font(DS.Font.bodyLarge).foregroundStyle(DSColor.text).lineLimit(1).truncationMode(.middle)
                    Text(canvas.sizeText).font(DS.Font.number).foregroundStyle(DSColor.textTertiary)
                }
                Spacer(minLength: 0)
                if current {
                    Image(systemName: "checkmark").font(.system(size: 10, weight: .bold)).foregroundStyle(DSColor.accent)
                } else if openInTab {
                    Circle().fill(DSColor.textTertiary).frame(width: 5, height: 5).help("Open in a tab")
                }
            }
            .padding(.horizontal, DS.Space.sm)
            .frame(height: 44)
            .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(current ? DSColor.selected : hover ? DSColor.hover : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .accessibilityLabel("\(canvas.name), \(canvas.sizeText)")
        .accessibilityAddTraits(current ? .isSelected : [])
    }
}

private struct FooterRow: View {
    let symbol: String
    let title: String
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: DS.Space.sm) {
                Image(systemName: symbol).font(.system(size: 11, weight: .medium)).foregroundStyle(DSColor.textSecondary).frame(width: 18)
                Text(title).font(DS.Font.bodyLarge).foregroundStyle(DSColor.text)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, DS.Space.sm)
            .frame(height: DS.Metric.iconButton)
            .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(hover ? DSColor.hover : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
    }
}
