import AppKit
import PixmithCore
import SwiftUI

extension SpriteModel {
    /// Names the undo step the operation just performed (only if that step has no name yet).
    func labelLastStep(_ name: String) {
        name.withCString { pixmith_checkpoint_label(doc, $0) }
        HistoryStamp.value &+= 1
    }

    /// Runs `body` so that every core operation inside it forms a single undo step.
    func inTransaction<T>(_ body: () -> T) -> T {
        pixmith_begin_transaction(doc)
        defer { pixmith_end_transaction(doc) }
        return body()
    }

    /// Applied undo steps and undone (redoable) steps.
    var historyCounts: (undo: Int, redo: Int) {
        var u: UInt32 = 0, r: UInt32 = 0
        pixmith_history_counts(doc, &u, &r)
        return (Int(u), Int(r))
    }

    func historyLabel(_ i: Int) -> String {
        guard let p = pixmith_history_label(doc, UInt32(i)) else { return "" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    /// Undoes or redoes until `position` steps are applied (0 = the original document).
    func goToHistory(_ position: Int) {
        stop()
        guard pixmith_history_goto(doc, UInt32(position)) else { return }
        clampSelection()
        changed()
    }
}

/// Bumped whenever a step is labeled; with the step counts it tells the history list when its labels are stale.
enum HistoryStamp {
    static var value = 0
}

/// Floating list of undo steps (View > History). Clicking a row jumps to that state.
final class HistoryController {
    static let shared = HistoryController()
    private var panel: NSPanel?

    func toggle(model: SpriteModel) {
        if let p = panel, p.isVisible { p.close() } else { show(model: model) }
    }

    /// The list is only built while the panel is open, so a hidden panel costs nothing per edit.
    func show(model: SpriteModel) {
        if panel == nil {
            let p = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 240, height: 360),
                            styleMask: [.titled, .closable, .resizable, .utilityWindow],
                            backing: .buffered, defer: false)
            p.title = "History"
            p.isFloatingPanel = true
            p.hidesOnDeactivate = false
            p.becomesKeyOnlyIfNeeded = true
            p.isReleasedWhenClosed = false
            p.delegate = observer
            p.setFrameAutosaveName("PixmithHistory")
            if !p.setFrameUsingName("PixmithHistory") { p.center() }
            panel = p
        }
        panel?.contentView = NSHostingView(rootView: HistoryView(model: model))
        panel?.orderFront(nil)
    }

    private lazy var observer = CloseObserver { [weak self] in self?.panel?.contentView = NSView() }

    private final class CloseObserver: NSObject, NSWindowDelegate {
        let onClose: () -> Void
        init(_ onClose: @escaping () -> Void) { self.onClose = onClose }
        func windowWillClose(_ notification: Notification) { onClose() }
    }
}

struct HistoryView: View {
    @ObservedObject var model: SpriteModel
    @StateObject private var cache = Cache()

    /// Step labels, rebuilt only when the step counts or a label change.
    final class Cache: ObservableObject {
        private var key = (applied: -1, undone: -1, stamp: -1)
        private(set) var titles: [String] = []

        func update(_ model: SpriteModel) -> (applied: Int, titles: [String]) {
            let (applied, undone) = model.historyCounts
            let k = (applied, undone, HistoryStamp.value)
            if k != key {
                key = k
                titles = ["Original"] + (0..<(applied + undone)).map { model.historyLabel($0) }
            }
            return (applied, titles)
        }
    }

    var body: some View {
        // `revision` changes with every edit, undo and redo, which keeps this list current.
        let _ = model.revision
        let (applied, titles) = cache.update(model)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(titles.indices, id: \.self) { position in
                        row(titles[position], position: position, applied: applied)
                            .id(position)
                    }
                }
                .padding(DS.Space.sm)
            }
            .onChange(of: applied) { proxy.scrollTo($0) }
            .onAppear { proxy.scrollTo(applied) }
        }
        .background(DSColor.panel)
        .frame(minWidth: 180, minHeight: 120)
    }

    /// Position 0 is the original document; position n is the state after step n.
    private func row(_ title: String, position: Int, applied: Int) -> some View {
        HistoryRow(title: title, current: position == applied, undone: position > applied) { model.goToHistory(position) }
    }
}

private struct HistoryRow: View {
    let title: String
    let current: Bool
    let undone: Bool
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        HStack {
            Text(title).font(DS.Font.body.weight(current ? .medium : .regular)).lineLimit(1)
            Spacer()
        }
        .padding(.horizontal, DS.Space.sm)
        .frame(height: DS.Metric.control + 2)
        .foregroundStyle(undone ? DSColor.textTertiary : current ? DSColor.text : DSColor.textSecondary)
        .background(RoundedRectangle(cornerRadius: DS.Radius.control)
            .fill(current ? DSColor.selected : hover ? DSColor.hover : Color.clear))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture(perform: action)
        .accessibilityAddTraits(current ? .isSelected : [])
    }
}
