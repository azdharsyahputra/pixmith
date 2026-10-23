import AppKit
@testable import PixmithKit

func allViews(_ v: NSView) -> [NSView] { [v] + v.subviews.flatMap(allViews) }

/// Runs `body`, which opens a modal alert or panel, while a timer inside the modal loop answers it: `fill` may edit the
/// controls first, then the button titled `title` is clicked.
///
/// The answerer is a run loop timer, not a dispatch block: when `body` ends up in `runModal` from inside a block that is
/// itself running on the main dispatch queue (a background job's completion handler, e.g. a script error alert), the
/// nested modal loop does not run further main-queue blocks, but it does fire timers.
func answerModal(_ title: String, fill: ((NSWindow) -> Void)? = nil, during body: () -> Void,
                 file: String = #fileID, line: UInt = #line) {
    var answered = false
    var attempts = 0
    let timer = Timer(timeInterval: 0.02, repeats: true) { t in
        attempts += 1
        if let w = NSApp.modalWindow, let root = w.contentView {
            fill?(w)
            if let b = allViews(root).compactMap({ $0 as? NSButton }).first(where: { $0.title == title }) {
                answered = true
                t.invalidate()
                b.performClick(nil)
                return
            }
        }
        if attempts >= 200 {
            t.invalidate()
            if NSApp.modalWindow != nil { NSApp.stopModal(withCode: .abort) }
        }
    }
    RunLoop.main.add(timer, forMode: .common)
    body()
    timer.invalidate()
    expect(answered, "modal with a \"\(title)\" button appeared", file: file, line: line)
}

extension NSWindow {
    var textFields: [NSTextField] { contentView.map { allViews($0).compactMap { $0 as? NSTextField }.filter(\.isEditable) } ?? [] }
    var popups: [NSPopUpButton] { contentView.map { allViews($0).compactMap { $0 as? NSPopUpButton } } ?? [] }
    var buttonTitles: [String] { contentView.map { allViews($0).compactMap { ($0 as? NSButton)?.title } } ?? [] }
}
