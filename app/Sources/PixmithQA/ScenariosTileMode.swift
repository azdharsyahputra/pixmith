import AppKit
@testable import PixmithKit
import PixmithCore

// pixmith-qa scenarios for the Pixels | Tiles mode of tilemap layers (stamp on = Tiles).

func registerTileMode() {
    scenario("tilemap/mode-leaves-tiles") {
        let m = fresh(32, 32), d = CanvasDriver()
        let tm = TilemapState.shared
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        m.setTileMode(true)
        expect(tm.stamp, "Tiles is on")
        expect(m.tilesMode)
        // The pencil is already selected: its key still leaves Tiles.
        expect(m.tool == .pencil)
        d.key("b")
        expect(!tm.stamp, "choosing the tool that is already selected leaves Tiles")
        // The tool rail sets the tool the same way.
        m.setTileMode(true)
        m.tool = .pencil
        expect(!tm.stamp, "the tool rail leaves Tiles")
        m.setTileMode(true)
        d.key("e")
        expect(!tm.stamp && m.tool == .eraser, "another tool leaves Tiles")
        // A new primary color means the user wants to draw.
        m.setTileMode(true)
        m.color = RED
        expect(!tm.stamp, "choosing a color leaves Tiles")
        m.setTileMode(true)
        expect(tm.stamp && m.tool == .eraser, "Tiles works with the eraser")
        m.setTileMode(false)
        expect(!tm.stamp && !AutotileState.shared.brush, "Pixels is stamp and terrain brush off")
    }

    scenario("tilemap/mode-tab-toggles") {
        let m = fresh(32, 32), d = CanvasDriver()
        let tm = TilemapState.shared
        let s = Shortcuts.shared
        s.reset()
        defer { s.reset() }
        expectEq(s.label(for: .tileMode), "⇥")
        expectEq(ShortcutAction.tileMode.title, "Pixels / Tiles")
        expectEq(ShortcutAction.tileMode.id, "tileMode")
        expect(!m.isTilemapLayer)
        d.key("\t", code: 48)
        expect(!tm.stamp, "Tab does nothing on a normal layer")
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        d.key("\t", code: 48)
        expect(tm.stamp, "Tab switches to Tiles")
        d.key("\t", code: 48)
        expect(!tm.stamp, "Tab switches back to Pixels")
        // Tab can be rebound like the other keys.
        s.recording = .tileMode
        d.key("j")
        expectEq(s.label(for: .tileMode), "J")
        d.key("\t", code: 48)
        expect(!tm.stamp, "the old key no longer toggles")
        d.key("j")
        expect(tm.stamp, "the new key toggles")
    }

    scenario("tilemap/mode-tiles-needs-pencil") {
        let m = fresh(32, 32)
        let tm = TilemapState.shared
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        m.tool = .fill
        m.setTileMode(true)
        expect(m.tool == .pencil, "choosing Tiles with Fill switches to the pencil")
        expect(tm.stamp, "and Tiles is on")
        m.tool = .eraser
        m.setTileMode(true)
        expect(m.tool == .eraser && tm.stamp, "the eraser stays")
        // Not a tilemap layer: no Tiles.
        m.setTileMode(false)
        m.convertToNormal()
        m.setTileMode(true)
        expect(!tm.stamp, "no Tiles on a normal layer")
    }

    scenario("tilemap/mode-stuck-hint") {
        let m = fresh(32, 32), d = CanvasDriver()
        let tm = TilemapState.shared
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        m.addTile()
        m.setTileMode(true)
        d.click(3, 3)
        expect(!tm.stampHint, "a stamp that changes the cell shows no hint")
        d.click(3, 3)
        expect(!tm.stampHint, "one no-op stamp is not enough")
        d.click(12, 12)
        d.click(12, 12)
        expect(!tm.stampHint, "a stamp that changed a cell reset the count")
        d.click(12, 12)
        expect(tm.stampHint, "two no-op stamps in a row show the hint")
        // Dragging inside one cell is not a row of clicks.
        tm.stampHint = false
        d.stroke([(20, 20), (21, 21), (22, 22), (23, 23)])
        d.stroke([(20, 20), (21, 21), (22, 22), (23, 23)])
        expect(!tm.stampHint, "a drag is one stamp")
        // Back to Pixels hides it.
        d.click(20, 20)
        d.click(20, 20)
        expect(tm.stampHint)
        m.setTileMode(false)
        expect(!tm.stampHint, "Pixels hides the hint")
    }

    scenario("tilemap/mode-ghost-and-copies") {
        let m = fresh(32, 32), d = CanvasDriver()
        let tm = TilemapState.shared
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        // A tile with a red pixel at (3, 3), shown in two cells.
        m.color = RED
        d.click(3, 3)
        let tile = Int(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 3, 3) & 0x1FFFFFFF)
        expect(tile > 0)
        tm.selected = tile
        tm.stamp = true
        d.click(12, 12)
        tm.stamp = false
        // Pixels: the copies of the hovered tile are outlined.
        d.move(4, 4)
        expectEq(m.tileCopies(at: (4, 4)).count, 2, "two cells show the tile")
        expectEq(TileCopiesHover.boxes.count, 2)
        // Tiles: no copies outline; the selected tile follows the pointer at the cell under it.
        m.setTileMode(true)
        expectEq(m.tileCopies(at: (4, 4)).count, 0, "no copies outline in Tiles mode")
        d.move(28, 28)
        expectEq(TileGhostHover.cell, CGRect(x: 24, y: 24, width: 8, height: 8))
        let rep = d.bitmap()
        let c = d.shown(24 + 3, 24 + 3, in: rep) ?? 0
        expect((c >> 16 & 0xFF) > (c >> 8 & 0xFF) + 40, "the ghost shows the tile's red pixel (\(String(c, radix: 16)))")
        // The ghost follows the cell, and goes away with the pointer or with Tiles.
        d.move(2, 20)
        expectEq(TileGhostHover.cell, CGRect(x: 0, y: 16, width: 8, height: 8))
        d.leave()
        expect(TileGhostHover.cell == nil, "no ghost without the pointer")
        d.move(28, 28)
        m.setTileMode(false)
        expect(m.tileGhostCell(at: (28, 28)) == nil, "no ghost in Pixels mode")
    }
}
