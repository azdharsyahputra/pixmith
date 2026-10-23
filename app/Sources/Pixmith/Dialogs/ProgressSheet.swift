import AppKit
import PixmithCore
import SwiftUI

/// Progress shown while work runs off the main thread.
struct ProgressSheet: View {
    let title: String
    var detail: String?
    /// Shown as a Cancel button when set.
    var onCancel: (() -> Void)?
    @State private var cancelling = false

    var body: some View {
        VStack(alignment: .leading, spacing: DS.Space.md) {
            Text(title).font(.system(size: 15, weight: .semibold))
            ProgressView().progressViewStyle(.linear)
            if let detail { DSNote(detail) }
            if let onCancel {
                HStack {
                    Spacer()
                    Button(cancelling ? "Cancelling…" : "Cancel") {
                        cancelling = true
                        onCancel()
                    }
                    .buttonStyle(DSSecondaryButtonStyle())
                    .disabled(cancelling)
                    .keyboardShortcut(.cancelAction)
                }
            }
        }
        .padding(DS.Space.xl - 4)
        .frame(width: 340)
        .background(DSColor.panel)
    }
}

/// A snapshot document handed to one worker thread; the worker is its only user (see docs/ARCHITECTURE.md, Threading).
struct SendableDoc: @unchecked Sendable {
    let doc: OpaquePointer
}

/// Whether the document window has a sheet. Menus observe it to disable commands that would change the
/// document under a dialog or a running job.
final class SheetState: ObservableObject {
    static let shared = SheetState()
    @Published private(set) var isOpen = false

    private init() {
        for name in [NSWindow.willBeginSheetNotification, NSWindow.didEndSheetNotification] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                // The notification arrives before the sheet is attached or detached.
                DispatchQueue.main.async { self?.isOpen = MainWindow.hasSheet }
            }
        }
    }
}

/// Runs work on a background queue while a modal progress sheet covers the document window.
///
/// Threading rule: `work` must never touch the document that the UI is using. Give it a snapshot
/// (`pixmith_snapshot`), an independent document, or plain data; `done` runs on the main thread and may
/// apply the result.
enum Background {
    /// One job at a time.
    private(set) static var busy = false

    /// Starts `work`. Returns false, doing nothing, if another job is running. The sheet appears at
    /// once with `immediate` (use it when the UI must not change meanwhile), otherwise only if the job
    /// takes longer than a quarter of a second.
    @discardableResult
    static func run<T>(title: String, immediate: Bool = false, cancel: (() -> Void)? = nil,
                       work: @escaping () -> T, done: @escaping (T) -> Void) -> Bool {
        guard !busy else { return false }
        busy = true
        let sheet = JobSheet(title: title, cancel: cancel)
        if immediate { sheet.show() } else { sheet.show(after: 0.25) }
        DispatchQueue.global(qos: .userInitiated).async {
            let result = work()
            DispatchQueue.main.async {
                sheet.dismiss()
                busy = false
                done(result)
            }
        }
        return true
    }
}

private final class JobSheet {
    private let title: String
    private let cancel: (() -> Void)?
    private var window: NSWindow?
    private var pending: DispatchWorkItem?
    private var dismissed = false

    init(title: String, cancel: (() -> Void)?) {
        self.title = title
        self.cancel = cancel
    }

    func show(after delay: TimeInterval) {
        let item = DispatchWorkItem { [weak self] in self?.show() }
        pending = item
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
    }

    func show() {
        guard window == nil, !dismissed, let host = MainWindow.window, host.attachedSheet == nil else { return }
        let w = NSWindow(contentViewController: NSHostingController(rootView: ProgressSheet(title: title, onCancel: cancel)))
        w.styleMask = [.titled]
        window = w
        host.beginSheet(w)
    }

    func dismiss() {
        dismissed = true
        pending?.cancel()
        if let w = window, let host = MainWindow.window { host.endSheet(w) }
        window = nil
    }
}

extension SpriteModel {
    /// Runs an export or other read-only job on a snapshot of the active document, off the main thread.
    /// The error text is read on the worker thread, where the core recorded it.
    func exportInBackground(_ title: String, _ body: @escaping (OpaquePointer) -> Bool) {
        guard let snap = pixmith_snapshot(doc) else { return }
        let started = Background.run(title: title, work: { () -> (ok: Bool, error: String) in
            let ok = body(snap)
            return (ok, ok ? "" : String(cString: pixmith_last_error()))
        }, done: { [weak self] result in
            pixmith_free(snap)
            if !result.ok { self?.errorMessage = result.error }
        })
        if !started { pixmith_free(snap) }
    }

    /// Builds a document from a file off the main thread and opens it in a tab.
    func importInBackground(_ title: String, _ load: @escaping () -> OpaquePointer?) {
        Background.run(title: title, work: { () -> (doc: OpaquePointer?, error: String) in
            let d = load()
            return (d, d == nil ? String(cString: pixmith_last_error()) : "")
        }, done: { [weak self] result in
            guard let self else { return }
            if let d = result.doc {
                self.adopt(d)
            } else {
                self.errorMessage = result.error
            }
        })
    }
}
