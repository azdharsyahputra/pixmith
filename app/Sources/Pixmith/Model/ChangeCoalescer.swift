import AppKit
import QuartzCore

/// Publishes `SpriteModel.revision` at most once per display refresh while changes arrive in a burst (a brush
/// stroke produces one `changed()` per mouse event, far more than the screen can show). A change after an idle
/// moment publishes immediately; changes that follow within the same refresh are folded into the next tick.
final class ChangeCoalescer: NSObject {
    static let shared = ChangeCoalescer()

    private weak var model: SpriteModel?
    private var pending = false
    private var idleTicks = 0
    private var link: AnyObject?
    private var timer: Timer?
    /// Publishes made by ticks instead of at once (diagnostics).
    private(set) var coalesced = 0
    /// With this off every change publishes at once (tests and scripts that read `revision` right away).
    var enabled = true

    var isRunning: Bool { link != nil || timer != nil }

    func noteChange(_ m: SpriteModel) {
        model = m
        guard enabled else {
            m.revision &+= 1
            return
        }
        if isRunning {
            if pending { coalesced += 1 }
            pending = true
            idleTicks = 0
        } else {
            m.revision &+= 1
            start()
        }
    }

    /// Publishes a pending change now.
    func flush() {
        guard pending, let m = model else { return }
        pending = false
        m.revision &+= 1
    }

    private func start() {
        idleTicks = 0
        if #available(macOS 14.0, *), let view = NSApp.keyWindow?.contentView ?? NSApp.windows.first?.contentView {
            let l = view.displayLink(target: self, selector: #selector(tick))
            l.add(to: .main, forMode: .common)
            link = l
        } else {
            let t = Timer(timeInterval: 1.0 / 60.0, target: self, selector: #selector(tick), userInfo: nil, repeats: true)
            RunLoop.main.add(t, forMode: .common)
            timer = t
        }
    }

    private func stop() {
        if #available(macOS 14.0, *), let l = link as? CADisplayLink { l.invalidate() }
        link = nil
        timer?.invalidate()
        timer = nil
    }

    @objc private func tick() {
        if pending {
            flush()
            idleTicks = 0
        } else {
            idleTicks += 1
            if idleTicks >= 3 { stop() }
        }
    }
}
