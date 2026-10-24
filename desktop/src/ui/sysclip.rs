//! System clipboard helpers for copy and paste of pixels (`selection.rs`).
//!
//! * Copy puts the selection on the system clipboard as an image (`arboard::Clipboard::set_image`), so other
//!   apps can paste it. The app keeps its own copy as well, which remembers where the pixels came from.
//! * Paste must also work when the clipboard holds only an image. egui-winit turns Ctrl+V into an
//!   `Event::Paste(text)` and *swallows* the key press, so with an image-only clipboard (a screenshot, a copy
//!   from another editor) nothing reaches the app on press. [`PasteWatch`] therefore also looks at the key
//!   release, which egui-winit does forward, and asks for a paste when no paste event came with the press.

use eframe::egui;

/// Puts straight-alpha RGBA pixels on the system clipboard as an image. False if the clipboard refused.
pub fn put_image(cb: &mut arboard::Clipboard, w: u32, h: u32, rgba: &[u8]) -> bool {
    if w == 0 || h == 0 || rgba.len() != w as usize * h as usize * 4 {
        return false;
    }
    let image = arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(rgba) };
    cb.set_image(image).is_ok()
}

/// Whether two RGBA images show the same picture. Clipboards round-trip colors through other pixel formats
/// (premultiplied alpha, BGRA), so colors may differ by a few steps and the color of a fully transparent pixel
/// does not count.
pub fn same_picture(a: (u32, u32, &[u8]), b: (u32, u32, &[u8])) -> bool {
    if (a.0, a.1) != (b.0, b.1) || a.2.len() != b.2.len() {
        return false;
    }
    a.2.as_chunks::<4>()
        .0
        .iter()
        .zip(b.2.as_chunks::<4>().0)
        .all(|(p, q)| p[3].abs_diff(q[3]) <= 1 && (p[3] == 0 && q[3] == 0 || (0..3).all(|c| p[c].abs_diff(q[c]) <= 4)))
}

/// Decides from the frame's input events whether the user asked to paste (Cmd/Ctrl+V).
#[derive(Default, Debug)]
pub struct PasteWatch {
    /// A paste was already asked for by the key press that is still held down.
    asked: bool,
    /// When a paste last ran, whatever started it (the menu consumes the key press before this sees it).
    last_paste: Option<std::time::Instant>,
}

/// A key release this soon after a paste belongs to the key press that caused it.
const SAME_PRESS: std::time::Duration = std::time::Duration::from_millis(1500);

impl PasteWatch {
    /// Records that a paste ran (from the keyboard, the menu or anywhere else).
    pub fn note_pasted(&mut self) {
        self.last_paste = Some(std::time::Instant::now());
    }

    /// Call once per frame with the raw events, before any of them is consumed.
    pub fn paste_requested(&mut self, events: &[egui::Event]) -> bool {
        let mut paste = false;
        for e in events {
            match e {
                // Text on the clipboard: egui-winit sends this on the key press.
                egui::Event::Paste(_) => {
                    paste = true;
                    self.asked = true;
                }
                // Other integrations (and tests) send the plain key press.
                egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if modifiers.command => {
                    paste = true;
                    self.asked = true;
                }
                // The press was swallowed (image-only clipboard): the release still arrives.
                egui::Event::Key { key: egui::Key::V, pressed: false, modifiers, .. } if modifiers.command => {
                    let ran = self.last_paste.is_some_and(|t| t.elapsed() < SAME_PRESS);
                    paste |= !(std::mem::take(&mut self.asked) || ran);
                }
                _ => {}
            }
        }
        paste
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(pressed: bool, command: bool) -> egui::Event {
        let modifiers = if command { egui::Modifiers::COMMAND } else { egui::Modifiers::NONE };
        egui::Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers }
    }

    #[test]
    fn a_text_paste_is_not_repeated_by_the_key_release() {
        let mut w = PasteWatch::default();
        assert!(w.paste_requested(&[egui::Event::Paste("x".into())]));
        assert!(!w.paste_requested(&[v(false, true)]), "the release of the same key press does nothing");
        assert!(w.paste_requested(&[v(false, true)]), "but a release with no earlier request pastes");
    }

    #[test]
    fn a_paste_that_already_ran_covers_the_release() {
        let mut w = PasteWatch::default();
        // The press was consumed elsewhere (the Edit menu) and pasted; only the release reaches the watch.
        w.note_pasted();
        assert!(!w.paste_requested(&[v(false, true)]));
    }

    #[test]
    fn an_image_only_clipboard_pastes_on_release() {
        let mut w = PasteWatch::default();
        assert!(!w.paste_requested(&[]), "nothing happened");
        assert!(w.paste_requested(&[v(false, true)]));
        assert!(!w.paste_requested(&[v(false, false)]), "V without Ctrl is the plain key");
    }

    #[test]
    fn a_plain_press_and_release_paste_once() {
        let mut w = PasteWatch::default();
        assert!(w.paste_requested(&[v(true, true), v(false, true)]));
        assert!(!w.paste_requested(&[v(true, false)]));
    }

    #[test]
    fn same_picture_ignores_rounding_and_hidden_colors() {
        let a = [10u8, 20, 30, 255, 99, 98, 97, 0];
        let b = [12u8, 18, 31, 255, 0, 0, 0, 0];
        assert!(same_picture((2, 1, &a), (2, 1, &b)));
        assert!(!same_picture((2, 1, &a), (1, 2, &b)));
        let c = [10u8, 20, 90, 255, 0, 0, 0, 0];
        assert!(!same_picture((2, 1, &a), (2, 1, &c)));
    }
}
