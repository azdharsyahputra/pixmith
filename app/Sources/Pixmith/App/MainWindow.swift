import AppKit

/// The document window, recorded when it appears. Plain-key handlers and file commands check it so
/// they never act from the History, Palette, Preview, Preferences or help windows, or under a sheet.
enum MainWindow {
    static weak var window: NSWindow?

    /// True when the document window itself is the key window (a sheet being key makes this false).
    static var windowIsKey: Bool { assumeKey || (window != nil && NSApp.keyWindow === window) }

    /// True when the document window is key and shows the editor: the single-key tool shortcuts and tab keys
    /// belong to it, not to Home or a project's canvas grid.
    static var isKey: Bool { assumeKey || (windowIsKey && AppRoute.shared.isEditor) }

    /// Set by the QA runner, whose offscreen windows never become key.
    static var assumeKey = false

    static var hasSheet: Bool { window?.attachedSheet != nil }

    private static var didFillScreen = false

    /// Opens the document window filling the screen (not full-screen mode): a smaller default window cut off
    /// the inspector and timeline. Once per launch, so a size the user picks afterwards is kept.
    static func fillScreenOnce(_ window: NSWindow) {
        guard !didFillScreen, !assumeKey, let screen = window.screen ?? NSScreen.main else { return }
        didFillScreen = true
        window.setFrame(screen.visibleFrame, display: true)
    }
}
