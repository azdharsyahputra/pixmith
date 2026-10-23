import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

enum LayerDropZone { case above, below, into }

/// Layer reordering built on the core's step-wise move / into-group / out-of-group operations. Layer indices
/// grow upwards: a row shown higher in the panel has a higher index.
extension SpriteModel {
    private func subtreeSize(_ i: Int) -> Int {
        var end = i + 1
        while end < layerCount, level(end) > level(i) { end += 1 }
        return end - i
    }

    private func parentLayer(_ i: Int) -> Int? {
        let l = level(i)
        return l == 0 ? nil : (0..<i).reversed().first { level($0) == l - 1 }
    }

    /// Indices of the layers directly inside `parent` (or at the top level), bottom to top.
    private func siblings(of parent: Int?) -> [Int] {
        let l = parent.map { level($0) + 1 } ?? 0
        var out: [Int] = []
        var j = (parent ?? -1) + 1
        while j < layerCount, level(j) >= l {
            if level(j) == l { out.append(j) }
            j += 1
        }
        return out
    }

    /// Moves layer `i` (with its children) next to, or into, layer `t`. Returns false when the drop is invalid.
    /// The whole drop is one undo step.
    @discardableResult
    func dropLayer(_ i: Int, on t: Int, zone: LayerDropZone) -> Bool {
        guard i != t, i >= 0, t >= 0, i < layerCount, t < layerCount else { return false }
        let n = subtreeSize(i)
        guard !(i..<i + n).contains(t) else { return false }
        var cur = i, tgt = t
        let selectedBefore = layer

        /// Records that the block moved from `cur` to `new`, keeping `tgt` pointing at the same layer.
        func moved(to new: Int32) -> Bool {
            guard new >= 0 else { return false }
            var t2 = tgt
            if t2 > cur { t2 -= n }
            if t2 >= Int(new) { t2 += n }
            tgt = t2
            cur = Int(new)
            steps += 1
            return true
        }

        var ok = true
        var steps = 0
        pixmith_begin_transaction(doc)
        defer { pixmith_end_transaction(doc) }
        if zone == .into, isGroup(tgt) {
            if parentLayer(cur) != tgt { ok = moved(to: pixmith_layer_into_group(doc, UInt32(cur), UInt32(tgt))) }
        } else {
            if parentLayer(cur) != parentLayer(tgt) {
                if let p = parentLayer(tgt) {
                    ok = moved(to: pixmith_layer_into_group(doc, UInt32(cur), UInt32(p)))
                } else {
                    while ok, level(cur) > 0 { ok = moved(to: pixmith_layer_out_of_group(doc, UInt32(cur))) }
                }
            }
            var guardCount = 0
            while ok, guardCount < 2 * layerCount {
                guardCount += 1
                let sib = siblings(of: parentLayer(tgt))
                guard let p = sib.firstIndex(of: cur), let q = sib.firstIndex(of: tgt) else { break }
                let goal = zone == .below ? q - 1 : q + 1
                if p == goal { break }
                ok = moved(to: pixmith_layer_move(doc, UInt32(cur), p < goal))
            }
        }
        if steps > 0 { labelLastStep("Reorder Layers") }
        // Keep editing the same layer, or the first drawable one inside a dragged group.
        if isGroup(cur) {
            layer = min(max(selectedBefore, 0), layerCount - 1)
        } else {
            layer = cur
        }
        clampSelection()
        changed()
        return ok
    }
}

/// Source row of the drag in progress (the drag stays inside the app, so no payload decoding is needed).
enum LayerDragState {
    static var source: Int?
}

private struct LayerDropDelegate: DropDelegate {
    let model: SpriteModel
    let index: Int
    @Binding var zone: LayerDropZone?
    @Binding var height: CGFloat

    func zone(at y: CGFloat) -> LayerDropZone {
        let h = max(height, 1)
        if model.isGroup(index) { return y < h / 4 ? .above : y > h * 3 / 4 ? .below : .into }
        return y < h / 2 ? .above : .below
    }

    func validateDrop(info: DropInfo) -> Bool { LayerDragState.source != nil }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        // Rows are drawn top-down, so the top of a row is "above" it (higher index).
        zone = zone(at: info.location.y)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { zone = nil }

    func performDrop(info: DropInfo) -> Bool {
        defer { zone = nil; LayerDragState.source = nil }
        guard let src = LayerDragState.source else { return false }
        return model.dropLayer(src, on: index, zone: zone(at: info.location.y))
    }
}

private struct LayerReorder: ViewModifier {
    let model: SpriteModel
    let index: Int
    @State private var zone: LayerDropZone?
    @State private var height: CGFloat = 28

    func body(content: Content) -> some View {
        content
            .background(GeometryReader { g in Color.clear.onAppear { height = g.size.height }
                .onChange(of: g.size.height) { height = $0 } })
            .overlay(alignment: zone == .below ? .bottom : .top) {
                switch zone {
                case .into:
                    RoundedRectangle(cornerRadius: 5).stroke(Color.accentColor, lineWidth: 2)
                case .above, .below:
                    Rectangle().fill(Color.accentColor).frame(height: 2)
                case nil:
                    EmptyView()
                }
            }
            .onDrag {
                LayerDragState.source = index
                return NSItemProvider(object: "layer \(index)" as NSString)
            }
            .onDrop(of: [.text], delegate: LayerDropDelegate(model: model, index: index, zone: $zone, height: $height))
    }
}

extension View {
    /// Makes a layers-panel row draggable and a drop target for reordering / regrouping.
    func layerReorder(model: SpriteModel, index: Int) -> some View {
        modifier(LayerReorder(model: model, index: index))
    }
}
