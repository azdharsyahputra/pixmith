//! Blur and Jumble: brush strokes that rework the pixels under the brush (`pixmith_retouch_*`).
//!
//! The canvas calls `down`, `drag` and `up` for these two tools. Like the pencil, a stroke opens one undo step
//! (`Doc::begin_edit`), and the core paints the live result as the pointer moves.

use super::draw;
use crate::app::{DocTab, Tool};
use crate::ui::canvas::CanvasState;

/// Strength used by Jumble, which has no strength option: every pixel under the brush is swapped.
const JUMBLE_STRENGTH: u32 = 100;

pub fn is_retouch(tool: Tool) -> bool {
    matches!(tool, Tool::Blur | Tool::Jumble)
}

/// Brush size adjusted by pen pressure when "scale size" is on, otherwise the plain size.
fn sized(state: &CanvasState, brush: u32) -> u32 {
    draw::pressure_style(state, 0xFFFF_FFFF, brush).map_or(brush, |v| v.1)
}

/// Opens the undo step and puts the first dab at `p`.
pub fn down(state: &mut CanvasState, tab: &mut DocTab, tool: Tool, brush: u32, p: (i32, i32)) {
    tab.playing = false;
    tab.doc.begin_edit(tool.title());
    let jumble = tool == Tool::Jumble;
    let strength = if jumble { JUMBLE_STRENGTH } else { state.opts.retouch_strength.round() as u32 };
    let seed = state.tools.draw.next_random();
    let (layer, frame) = (tab.layer, tab.frame);
    let size = sized(state, brush);
    tab.doc.retouch_begin(layer, frame, p, jumble, size, strength, seed, state.opts.sym_flags());
    mutated(tab);
}

/// The pointer moved with the button down.
pub fn drag(state: &mut CanvasState, tab: &mut DocTab, brush: u32, p: (i32, i32)) {
    if state.tools.draw.pressure.is_some() {
        tab.doc.retouch_size(sized(state, brush));
    }
    tab.doc.retouch_to(p);
    mutated(tab);
}

/// The button was released.
pub fn up(tab: &mut DocTab) {
    tab.doc.retouch_end();
    mutated(tab);
}

fn mutated(tab: &mut DocTab) {
    tab.doc.touch();
    tab.changed();
}
