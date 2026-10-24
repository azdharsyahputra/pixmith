import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

// The macOS renderer of docs/LUA_DIALOG.md: a Lua `Dialog` as a sheet (modal) or a floating panel (non-modal), built
// from the JSON the core describes with DesignSystem controls. The core owns the widget values; this file keeps its
// own copy of what is shown, sends what the user does (`widget`/`dialog.closed` events) through `ScriptSessions`, and
// applies the core's patches (`dialog.modify`).

/// One widget of a dialog, as the core describes it. Every field is optional on the wire; `apply` merges a patch.
struct DialogWidget: Identifiable {
    var id: String
    var kind = "label"
    var label: String?
    var enabled = true
    var visible = true
    var focus = false
    var hexpand = false
    var text = ""
    var selected = false
    var group = 0
    var value: Double = 0
    var minValue: Double = 0
    var maxValue: Double = 100
    var decimals = 0
    var color: UInt32 = 0
    var colors: [UInt32] = []
    var options: [String] = []
    var option: String?
    var filename = ""
    var open = true
    var save = false
    var filetypes: [String] = []
    var title = ""

    init(id: String) { self.id = id }

    init(json j: [String: Any]) {
        self.init(id: j["id"] as? String ?? UUID().uuidString)
        kind = j["kind"] as? String ?? "label"
        // The core sends `hexpand` for every widget; when it is absent the kind decides (docs/LUA_DIALOG.md 1.1).
        hexpand = ["entry", "number", "slider", "color", "combobox", "file"].contains(kind)
        apply(j)
    }

    private static func color(_ any: Any?) -> UInt32? { (any as? NSNumber)?.uint32Value }

    mutating func apply(_ j: [String: Any]) {
        if let v = j["label"] { label = v as? String }
        if let v = j["enabled"] as? Bool { enabled = v }
        if let v = j["visible"] as? Bool { visible = v }
        if let v = j["focus"] as? Bool { focus = v }
        if let v = j["hexpand"] as? Bool { hexpand = v }
        if let v = j["text"] as? String { text = v }
        if let v = j["selected"] as? Bool { selected = v }
        if let v = j["group"] as? NSNumber { group = v.intValue }
        if let v = j["value"] as? NSNumber { value = v.doubleValue }
        if let v = j["min"] as? NSNumber { minValue = v.doubleValue }
        if let v = j["max"] as? NSNumber { maxValue = v.doubleValue }
        if let v = j["decimals"] as? NSNumber { decimals = max(0, min(6, v.intValue)) }
        if let v = Self.color(j["color"]) { color = v }
        if let v = j["colors"] as? [NSNumber] { colors = v.map(\.uint32Value) }
        if let v = j["options"] as? [String] { options = v }
        if j.keys.contains("option") { option = j["option"] as? String }
        if let v = j["filename"] as? String { filename = v }
        if let v = j["open"] as? Bool { open = v }
        if let v = j["save"] as? Bool { save = v }
        if let v = j["filetypes"] as? [String] { filetypes = v }
        if let v = j["title"] as? String { title = v }
    }
}

/// The text of a button or label with the mnemonic marker removed (`&OK` is "OK", `&&` is a literal `&`).
func dialogText(_ s: String) -> String {
    var out = "", pending = false
    for ch in s {
        if pending { out.append(ch); pending = false } else if ch == "&" { pending = true } else { out.append(ch) }
    }
    return out
}

/// An open dialog: what the core said to show, and the user's edits since.
final class DialogModel: ObservableObject, Identifiable {
    let id: String
    let session: UInt64
    let modal: Bool
    let notitlebar: Bool
    let rows: [[String]]
    @Published var title: String
    @Published var widgets: [String: DialogWidget]
    /// Called with `(widget id, event, extra fields)`; the host turns it into a `widget` message.
    var send: (String, String, [String: Any]) -> Void = { _, _, _ in }
    /// Called when the user closes the window (Esc or the close button).
    var userClosed: () -> Void = {}

    init?(session: UInt64, json j: [String: Any]) {
        guard let id = j["id"] as? String else { return nil }
        self.id = id
        self.session = session
        modal = j["modal"] as? Bool ?? true
        notitlebar = j["notitlebar"] as? Bool ?? false
        title = j["title"] as? String ?? ""
        var map: [String: DialogWidget] = [:]
        var rows: [[String]] = []
        for row in j["rows"] as? [[String: Any]] ?? [] {
            var ids: [String] = []
            for w in row["widgets"] as? [[String: Any]] ?? [] {
                let widget = DialogWidget(json: w)
                map[widget.id] = widget
                ids.append(widget.id)
            }
            rows.append(ids)
        }
        self.rows = rows
        widgets = map
    }

    /// Applies a `dialog.modify` message.
    func modify(_ j: [String: Any]) {
        if let t = j["title"] as? String { title = t }
        for (wid, patch) in j["widgets"] as? [String: Any] ?? [:] {
            guard var w = widgets[wid], let p = patch as? [String: Any] else { continue }
            w.apply(p)
            widgets[wid] = w
        }
    }

    /// The width of the label column: the longest label of the dialog, within sane bounds.
    var labelWidth: CGFloat {
        let longest = widgets.values.compactMap { $0.label }.map { dialogText($0).count }.max() ?? 0
        return min(max(CGFloat(longest) * 6.4 + 6, 56), 190)
    }

    var hasLabels: Bool { widgets.values.contains { $0.label != nil } }

    // MARK: Edits from the views

    func edit(_ id: String, _ body: (inout DialogWidget) -> Void) {
        guard var w = widgets[id] else { return }
        body(&w)
        widgets[id] = w
    }

    func click(_ id: String, value: Bool? = nil, color: UInt32? = nil) {
        var extra: [String: Any] = [:]
        if let value { extra["value"] = value }
        if let color { extra["color"] = color }
        send(id, "click", extra)
    }

    func change(_ id: String, value: Any) { send(id, "change", ["value": value]) }
    func release(_ id: String) { send(id, "release", [:]) }

    /// A radio was chosen: it becomes selected locally at once and so do the others of its group clear (the core
    /// also patches them, but the view must not wait).
    func choose(radio id: String) {
        guard let picked = widgets[id] else { return }
        for (wid, w) in widgets where w.kind == "radio" && w.group == picked.group && wid != id && w.selected {
            edit(wid) { $0.selected = false }
        }
        edit(id) { $0.selected = true }
        click(id, value: true)
    }
}

// MARK: - View

struct LuaDialogView: View {
    @ObservedObject var model: DialogModel
    /// Sheets show the title inside the view; a panel has it in its title bar.
    var showsTitle = true
    @FocusState private var focused: String?

    var body: some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            if showsTitle, !model.title.isEmpty, !model.notitlebar {
                Text(model.title).font(.system(size: 15, weight: .semibold)).padding(.bottom, DS.Space.xs)
            }
            ForEach(model.rows.indices, id: \.self) { i in row(model.rows[i]) }
            // Esc closes a dialog like the Cancel button of a DSDialog.
            Button("") { model.userClosed() }.keyboardShortcut(.cancelAction).opacity(0).frame(width: 0, height: 0)
        }
        .padding(DS.Space.xl - 4)
        .frame(minWidth: 300, maxWidth: 560, alignment: .topLeading)
        .fixedSize(horizontal: false, vertical: true)
        .background(DSColor.panel)
        .onAppear { focused = model.widgets.values.first { $0.focus }?.id }
    }

    @ViewBuilder private func row(_ ids: [String]) -> some View {
        let widgets = ids.compactMap { model.widgets[$0] }.filter(\.visible)
        if widgets.count == 1, widgets[0].kind == "separator" {
            separator(widgets[0])
        } else if !widgets.isEmpty {
            HStack(alignment: .center, spacing: DS.Space.sm) {
                ForEach(Array(widgets.enumerated()), id: \.element.id) { index, w in
                    if let label = w.label {
                        Text(dialogText(label)).font(DS.Font.body).foregroundStyle(DSColor.textSecondary)
                            .frame(width: index == 0 ? model.labelWidth : nil, alignment: .leading)
                            .fixedSize(horizontal: index != 0, vertical: false)
                    } else if index == 0, model.hasLabels, widgets.count == 1, w.kind != "label", w.kind != "button" {
                        Color.clear.frame(width: model.labelWidth, height: 1)
                    }
                    control(w)
                        .frame(maxWidth: w.hexpand ? .infinity : nil, alignment: .leading)
                        .disabled(!w.enabled)
                }
            }
        }
    }

    private func separator(_ w: DialogWidget) -> some View {
        HStack(spacing: DS.Space.sm) {
            if !w.text.isEmpty {
                Text(dialogText(w.text)).font(DS.Font.section).foregroundStyle(DSColor.textSecondary)
            }
            DSDivider()
        }
        .padding(.vertical, DS.Space.xs)
    }

    @ViewBuilder private func control(_ w: DialogWidget) -> some View {
        switch w.kind {
        case "label":
            Text(dialogText(w.text)).font(DS.Font.body).foregroundStyle(DSColor.text)
                .fixedSize(horizontal: false, vertical: true)
        case "button":
            if w.focus {
                Button(dialogText(w.text)) { model.click(w.id) }
                    .buttonStyle(DSPrimaryButtonStyle()).keyboardShortcut(.defaultAction)
            } else {
                Button(dialogText(w.text)) { model.click(w.id) }.buttonStyle(DSSecondaryButtonStyle())
            }
        case "check":
            Toggle(dialogText(w.text), isOn: Binding(get: { model.widgets[w.id]?.selected ?? false }, set: { on in
                model.edit(w.id) { $0.selected = on }
                model.click(w.id, value: on)
            })).toggleStyle(.checkbox).font(DS.Font.body)
        case "radio":
            Button { model.choose(radio: w.id) } label: {
                HStack(spacing: DS.Space.xs + 2) {
                    Image(systemName: w.selected ? "largecircle.fill.circle" : "circle")
                        .foregroundStyle(w.selected ? DSColor.accent : DSColor.textSecondary)
                    Text(dialogText(w.text)).font(DS.Font.body).foregroundStyle(DSColor.text)
                }
            }
            .buttonStyle(.plain)
        case "entry":
            TextField("", text: Binding(get: { model.widgets[w.id]?.text ?? "" }, set: { t in
                model.edit(w.id) { $0.text = t }
                model.change(w.id, value: t)
            }))
            .textFieldStyle(.plain).font(DS.Font.body)
            .focused($focused, equals: w.id)
            .padding(.horizontal, DS.Space.sm).frame(height: DS.Metric.control)
            .dsFieldBackground(focused: focused == w.id)
        case "number":
            DialogNumberField(model: model, id: w.id)
        case "slider":
            DSSlider("", value: Binding(get: { model.widgets[w.id]?.value ?? 0 }, set: { v in
                let r = v.rounded()
                guard r != model.widgets[w.id]?.value else { return }
                model.edit(w.id) { $0.value = r }
                model.change(w.id, value: Int(r))
            }), range: w.minValue...max(w.maxValue, w.minValue + 1), onEditing: { editing in
                if !editing { model.release(w.id) }
            })
            .frame(minWidth: 140)
        case "color":
            ColorPicker("", selection: Binding(get: { Color(rgba: model.widgets[w.id]?.color ?? 0) }, set: { c in
                let v = UInt32(c.cgColor ?? .black)
                guard v != model.widgets[w.id]?.color else { return }
                model.edit(w.id) { $0.color = v }
                model.change(w.id, value: v)
            }), supportsOpacity: true)
            .labelsHidden()
        case "combobox":
            DSPopup(selection: Binding(get: { model.widgets[w.id]?.option ?? "" }, set: { o in
                model.edit(w.id) { $0.option = o }
                model.change(w.id, value: o)
            }), options: w.options.map { (value: $0, title: $0) }, fill: true)
        case "file":
            fileField(w)
        case "shades":
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 22), spacing: 3)], alignment: .leading, spacing: 3) {
                ForEach(Array(w.colors.enumerated()), id: \.offset) { _, c in
                    Swatch(color: Color(rgba: c), size: 20)
                        .onTapGesture { model.click(w.id, color: c) }
                }
            }
        case "separator":
            separator(w)
        default:
            EmptyView()
        }
    }

    private func fileField(_ w: DialogWidget) -> some View {
        HStack(spacing: DS.Space.sm) {
            Text(w.filename.isEmpty ? "No file" : (w.filename as NSString).lastPathComponent)
                .font(DS.Font.body).foregroundStyle(w.filename.isEmpty ? DSColor.textTertiary : DSColor.text)
                .lineLimit(1).truncationMode(.middle)
                .padding(.horizontal, DS.Space.sm).frame(maxWidth: .infinity, alignment: .leading)
                .frame(height: DS.Metric.control)
                .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
            Button("Choose…") { choose(file: w) }.buttonStyle(DSSecondaryButtonStyle())
        }
    }

    private func choose(file w: DialogWidget) {
        let types = w.filetypes.compactMap { UTType(filenameExtension: $0) }
        var picked: URL?
        if w.save && !w.open {
            let panel = NSSavePanel()
            if !types.isEmpty { panel.allowedContentTypes = types }
            if !w.title.isEmpty { panel.message = w.title }
            if !w.filename.isEmpty { panel.nameFieldStringValue = (w.filename as NSString).lastPathComponent }
            if panel.runModal() == .OK { picked = panel.url }
        } else {
            let panel = NSOpenPanel()
            if !types.isEmpty { panel.allowedContentTypes = types }
            if !w.title.isEmpty { panel.message = w.title }
            if panel.runModal() == .OK { picked = panel.url }
        }
        guard let url = picked else { return }
        model.edit(w.id) { $0.filename = url.path }
        model.change(w.id, value: url.path)
    }
}

/// A number field that keeps what the user is typing until it parses, so "1." does not snap back to "1".
private struct DialogNumberField: View {
    @ObservedObject var model: DialogModel
    let id: String
    @State private var text = ""
    @FocusState private var focused: Bool

    private var widget: DialogWidget { model.widgets[id] ?? DialogWidget(id: id) }

    private func format(_ v: Double) -> String { String(format: "%.\(widget.decimals)f", v) }

    var body: some View {
        TextField("", text: $text)
            .textFieldStyle(.plain).font(DS.Font.numberLarge)
            .multilineTextAlignment(.trailing)
            .focused($focused)
            .padding(.horizontal, DS.Space.sm).frame(height: DS.Metric.control)
            .dsFieldBackground(focused: focused)
            .onAppear { text = format(widget.value) }
            .onChange(of: text) { t in
                guard let v = Double(t.replacingOccurrences(of: ",", with: ".")), v != widget.value else { return }
                model.edit(id) { $0.value = v }
                model.change(id, value: v)
            }
            .onChange(of: widget.value) { v in
                // The script changed it (`modify`/`data`): show it unless it is what is being typed.
                if Double(text.replacingOccurrences(of: ",", with: ".")) != v { text = format(v) }
            }
            .onChange(of: focused) { f in if !f { text = format(widget.value) } }
            .onSubmit { text = format(widget.value) }
    }
}
