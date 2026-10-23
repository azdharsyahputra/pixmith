import AppKit
import SwiftUI

/// A single-key shortcut. `key` is the lowercased character the key produces.
struct KeyBinding: Codable, Hashable {
    var key: String
    var shift = false

    var label: String { (shift ? "⇧" : "") + (key == " " ? "Space" : key.uppercased()) }
}

/// Everything that can be triggered by a customizable single key.
enum ShortcutAction: Hashable, Identifiable {
    case tool(Tool)
    case swapColors, brushSmaller, brushLarger, previousFrame, nextFrame, playPause

    static var all: [ShortcutAction] {
        Tool.allCases.map(ShortcutAction.tool) + [.swapColors, .brushSmaller, .brushLarger, .previousFrame, .nextFrame, .playPause]
    }

    var id: String {
        switch self {
        case .tool(let t): "tool.\(t.rawValue)"
        case .swapColors: "swapColors"
        case .brushSmaller: "brushSmaller"
        case .brushLarger: "brushLarger"
        case .previousFrame: "previousFrame"
        case .nextFrame: "nextFrame"
        case .playPause: "playPause"
        }
    }

    var title: String {
        switch self {
        case .tool(let t): t.rawValue.capitalized
        case .swapColors: "Swap Colors"
        case .brushSmaller: "Smaller Brush"
        case .brushLarger: "Larger Brush"
        case .previousFrame: "Previous Frame"
        case .nextFrame: "Next Frame"
        case .playPause: "Play / Pause"
        }
    }

    var isTool: Bool { if case .tool = self { true } else { false } }

    /// Keys that work out of the box.
    var defaults: [KeyBinding] {
        let k = { (key: String, shift: Bool) in KeyBinding(key: key, shift: shift) }
        switch self {
        case .tool(let t):
            switch t {
            case .pencil: return [k("b", false)]
            case .eraser: return [k("e", false)]
            case .fill: return [k("g", false)]
            case .picker: return [k("i", false)]
            case .marquee: return [k("m", false)]
            case .move: return [k("v", false)]
            case .line: return [k("l", false)]
            case .rectangle: return [k("u", false)]
            case .ellipse: return [k("o", false), k("u", true)]
            case .spray: return [k("a", false)]
            case .wand: return [k("w", false)]
            case .lasso: return [k("q", false)]
            case .polygon: return [k("p", false)]
            case .gradient: return [k("g", true)]
            case .slice: return [k("s", false)]
            case .curve: return [k("c", false)]
            case .contour: return [k("d", false)]
            case .shapePolygon: return [k("p", true)]
            case .text: return [k("t", false)]
            }
        case .swapColors: return [k("x", false)]
        case .brushSmaller: return [k("[", false)]
        case .brushLarger: return [k("]", false)]
        case .previousFrame: return [k(",", false)]
        case .nextFrame: return [k(".", false)]
        case .playPause: return [k(" ", false)]
        }
    }
}

/// The central registry of customizable single-key shortcuts, persisted in UserDefaults.
final class Shortcuts: ObservableObject {
    static let shared = Shortcuts()
    private static let defaultsKey = "keyboardShortcuts"

    /// Only actions that differ from their defaults; an empty list means unassigned.
    @Published private(set) var overrides: [String: [KeyBinding]]
    @Published var recording: ShortcutAction?
    @Published var conflict: (action: ShortcutAction, binding: KeyBinding, other: ShortcutAction)?

    private init() {
        overrides = UserDefaults.standard.data(forKey: Self.defaultsKey)
            .flatMap { try? JSONDecoder().decode([String: [KeyBinding]].self, from: $0) } ?? [:]
    }

    func bindings(for a: ShortcutAction) -> [KeyBinding] { overrides[a.id] ?? a.defaults }

    func label(for a: ShortcutAction) -> String { bindings(for: a).first?.label ?? "none" }

    var isCustomized: Bool { !overrides.isEmpty }

    private func set(_ a: ShortcutAction, _ list: [KeyBinding]) {
        if list == a.defaults { overrides[a.id] = nil } else { overrides[a.id] = list }
        if let data = try? JSONEncoder().encode(overrides) {
            UserDefaults.standard.set(data, forKey: Self.defaultsKey)
        }
        SpriteModel.shared.objectWillChange.send()
    }

    func reset() {
        overrides = [:]
        conflict = nil
        recording = nil
        UserDefaults.standard.removeObject(forKey: Self.defaultsKey)
        SpriteModel.shared.objectWillChange.send()
    }

    /// The action bound to a key event: an exact (key, shift) match, else a plain binding ignoring Shift.
    func action(for e: NSEvent) -> ShortcutAction? {
        guard let ch = e.charactersIgnoringModifiers?.lowercased(), !ch.isEmpty else { return nil }
        let exact = KeyBinding(key: ch, shift: e.modifierFlags.contains(.shift))
        let all = ShortcutAction.all
        return all.first { bindings(for: $0).contains(exact) }
            ?? all.first { bindings(for: $0).contains { $0.key == ch && !$0.shift } }
    }

    /// Handles a key press for the single-key shortcuts, or records it while rebinding.
    func handle(_ e: NSEvent, model: SpriteModel) -> Bool {
        if let r = recording {
            capture(e, for: r)
            return true
        }
        guard let a = action(for: e) else { return false }
        model.perform(a)
        return true
    }

    private func capture(_ e: NSEvent, for a: ShortcutAction) {
        switch e.keyCode {
        case 53:
            recording = nil
            return
        case 51, 117:
            set(a, [])
            recording = nil
            return
        default:
            break
        }
        guard let ch = e.charactersIgnoringModifiers?.lowercased(), ch.count == 1,
              ch.unicodeScalars.allSatisfy({ $0.value >= 32 && $0.value < 0xF700 })
        else { return NSSound.beep() }
        let b = KeyBinding(key: ch, shift: e.modifierFlags.contains(.shift))
        recording = nil
        if let other = ShortcutAction.all.first(where: { $0 != a && bindings(for: $0).contains(b) }) {
            conflict = (a, b, other)
        } else {
            set(a, [b])
        }
    }

    /// Gives the key to the pending action and unassigns it from the one that had it.
    func resolveConflict() {
        guard let c = conflict else { return }
        set(c.other, bindings(for: c.other).filter { $0 != c.binding })
        set(c.action, [c.binding])
        conflict = nil
    }
}

extension SpriteModel {
    func perform(_ a: ShortcutAction) {
        switch a {
        case .tool(let t): tool = t
        case .swapColors: swapColors()
        case .brushSmaller: brushSize = max(1, brushSize - 1)
        case .brushLarger: brushSize = min(64, brushSize + 1)
        case .previousFrame: stop(); step(-1)
        case .nextFrame: stop(); step(1)
        case .playPause: togglePlay()
        }
    }
}

extension Tool {
    /// The current key label, for tooltips.
    var key: String { Shortcuts.shared.label(for: .tool(self)) }
}

/// Keys handled elsewhere that cannot be rebound, for the help window.
let fixedShortcuts: [(keys: String, what: String)] = [
    ("← → ↑ ↓", "Nudge the selection or move tool (⇧ moves 10 px)"),
    ("⌫", "Delete the selected pixels"),
    ("↩", "Commit a floating paste or open polygon"),
    ("⎋", "Cancel polygon or paste, then deselect"),
    ("⌥ + click", "Pick a color from the canvas"),
    ("Right-click", "Draw with the secondary color"),
    ("⇧ + drag", "Constrain lines, squares and circles"),
    ("⌘ + scroll, pinch", "Zoom the canvas"),
]

// MARK: Preferences tab

struct KeyboardPrefsView: View {
    @ObservedObject private var shortcuts = Shortcuts.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            List {
                Section("Tools") { rows(ShortcutAction.all.filter(\.isTool)) }
                Section("Actions") { rows(ShortcutAction.all.filter { !$0.isTool }) }
            }
            if let c = shortcuts.conflict {
                HStack {
                    Text("\(c.binding.label) is already used by \(c.other.title).").foregroundStyle(.orange)
                    Spacer()
                    Button("Reassign") { shortcuts.resolveConflict() }
                    Button("Cancel") { shortcuts.conflict = nil }
                }
            } else {
                Text("Click a key, then press the new one. Delete unassigns, Esc cancels.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Restore Default Shortcuts") { shortcuts.reset() }.disabled(!shortcuts.isCustomized)
            }
        }
        .padding()
        .onDisappear { shortcuts.recording = nil; shortcuts.conflict = nil }
    }

    @ViewBuilder
    private func rows(_ actions: [ShortcutAction]) -> some View {
        ForEach(actions) { a in
            HStack {
                Text(a.title)
                Spacer()
                Button(shortcuts.recording == a ? "Press a key…" : shortcuts.label(for: a)) {
                    shortcuts.conflict = nil
                    shortcuts.recording = shortcuts.recording == a ? nil : a
                }
                .buttonStyle(DSBarButtonStyle(prominent: shortcuts.recording == a))
                .frame(width: 120)
            }
        }
    }
}

// MARK: Help window

struct ShortcutsHelpView: View {
    @ObservedObject private var shortcuts = Shortcuts.shared
    @State private var menus: [(menu: String, items: [(title: String, keys: String)])] = []
    @State private var filter = ""

    private func matches(_ s: String) -> Bool { filter.isEmpty || s.localizedCaseInsensitiveContains(filter) }

    var body: some View {
        VStack(spacing: 0) {
            TextField("Filter", text: $filter).dsTextField().padding(DS.Space.md)
            List {
                Section("Tools") {
                    ForEach(ShortcutAction.all.filter(\.isTool).filter { matches($0.title) || matches(shortcuts.label(for: $0)) }) { row($0.title, shortcuts.label(for: $0)) }
                }
                Section("Actions") {
                    ForEach(ShortcutAction.all.filter { !$0.isTool }.filter { matches($0.title) || matches(shortcuts.label(for: $0)) }) { row($0.title, shortcuts.label(for: $0)) }
                }
                Section("Other keys") {
                    ForEach(fixedShortcuts.filter { matches($0.what) || matches($0.keys) }, id: \.keys) { row($0.what, $0.keys) }
                }
                ForEach(menus, id: \.menu) { m in
                    let items = m.items.filter { matches($0.title) || matches($0.keys) || matches(m.menu) }
                    if !items.isEmpty {
                        Section("\(m.menu) menu") {
                            ForEach(Array(items.enumerated()), id: \.offset) { row($0.element.title, $0.element.keys) }
                        }
                    }
                }
            }
        }
        .frame(minWidth: 420, minHeight: 480)
        .onAppear { menus = Self.menuShortcuts() }
    }

    private func row(_ title: String, _ keys: String) -> some View {
        HStack {
            Text(title).font(DS.Font.bodyLarge)
            Spacer()
            KeyHint(keys)
        }
    }

    /// Every menu item with a key equivalent, read from the live menu bar.
    static func menuShortcuts() -> [(menu: String, items: [(title: String, keys: String)])] {
        func collect(_ menu: NSMenu) -> [(title: String, keys: String)] {
            menu.items.flatMap { item -> [(title: String, keys: String)] in
                var out = item.submenu.map(collect) ?? []
                if !item.keyEquivalent.isEmpty, !item.title.isEmpty {
                    let m = item.keyEquivalentModifierMask
                    let mods = (m.contains(.control) ? "⌃" : "") + (m.contains(.option) ? "⌥" : "")
                        + (m.contains(.shift) ? "⇧" : "") + (m.contains(.command) ? "⌘" : "")
                    out.insert((item.title, mods + item.keyEquivalent.uppercased()), at: 0)
                }
                return out
            }
        }
        return (NSApp.mainMenu?.items ?? []).compactMap { top in
            guard let sub = top.submenu else { return nil }
            let items = collect(sub)
            return items.isEmpty ? nil : (sub.title, items)
        }
    }
}

struct HelpCommands: Commands {
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(after: .help) {
            Button("Keyboard Shortcuts") { openWindow(id: "shortcuts") }
        }
    }
}
