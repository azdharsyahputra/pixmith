//! Copy and paste through the real system clipboard. Skipped (with a message) where there is no clipboard,
//! for instance on a headless Linux box. The test restores the clipboard's previous text or image.

use pixmith_desktop::PixmithApp;
use pixmith_desktop::app::DocTab;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::selection;

const RED: u32 = 0xFF0000FF;

fn pixel(doc: &Doc, x: i32, y: i32) -> u32 {
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    Doc::pixel_of(&buf, doc.width(), x, y).expect("inside the sprite")
}

/// Pixels waiting on the clipboard as an image, if any.
fn image(cb: &mut arboard::Clipboard) -> Option<(usize, usize, Vec<u8>)> {
    cb.get_image().ok().map(|i| (i.width, i.height, i.bytes.into_owned()))
}

#[test]
fn copy_puts_an_image_on_the_clipboard_and_paste_takes_one_from_another_app() {
    let Ok(mut cb) = arboard::Clipboard::new() else {
        eprintln!("SKIPPED: no system clipboard available here");
        return;
    };
    let (old_text, old_image) = (cb.get_text().ok(), image(&mut cb));

    let mut app = PixmithApp::default();
    app.selection.system_clipboard = true;
    app.tabs[0] = DocTab::new(Doc::new(16, 16));
    let layer = app.tab().doc.layer_count() - 1;
    {
        let doc = &mut app.tab_mut().doc;
        doc.begin_edit("Draw");
        doc.stroke_begin(layer, 0, 2, 2, RED, 1, 0);
        doc.stroke_to(5, 2);
        doc.stroke_end();
        doc.sel_rect((2, 2), (5, 5));
    }

    // Copy: the pixels reach the clipboard as an image other apps can paste.
    selection::copy(&mut app, true);
    assert_eq!(pixel(&app.tab().doc, 3, 2) & 0xFF, 0, "cut removed the pixels");
    let Some((w, h, bytes)) = image(&mut cb) else {
        // Some clipboards (a Linux session without a selection owner) cannot hold an image at all.
        eprintln!("SKIPPED: this clipboard does not keep images");
        return;
    };
    assert_eq!((w, h), (4, 4), "the selection's size");
    assert_eq!(&bytes[..4], &[255, 0, 0, 255], "top-left pixel is red");
    assert_eq!(bytes[(4 + 1) * 4 + 3], 0, "transparent pixels stay transparent");

    // Paste right after our own copy: back where it came from.
    app.tab_mut().doc.sel_clear();
    selection::paste(&mut app);
    app.tab_mut().doc.float_commit();
    assert_eq!(pixel(&app.tab().doc, 3, 2), RED);

    // Paste an image from another app: a 3x3 green square lands in the middle of the canvas.
    let green: Vec<u8> = (0..9).flat_map(|_| [0u8, 255, 0, 255]).collect();
    cb.set_image(arboard::ImageData { width: 3, height: 3, bytes: green.into() }).expect("set image");
    let mut other = PixmithApp::default();
    other.selection.system_clipboard = true;
    other.tabs[0] = DocTab::new(Doc::new(16, 16));
    assert!(selection::can_paste(&mut other), "an image on the clipboard can be pasted");
    selection::paste(&mut other);
    other.tab_mut().doc.float_commit();
    assert_eq!(pixel(&other.tab().doc, 7, 7), 0x00FF00FF, "centered at (16 - 3) / 2");
    assert_eq!(pixel(&other.tab().doc, 2, 2) & 0xFF, 0);

    // Leave the clipboard as we found it.
    match (old_image, old_text) {
        (Some((w, h, bytes)), _) => {
            let _ = cb.set_image(arboard::ImageData { width: w, height: h, bytes: bytes.into() });
        }
        (None, Some(t)) => {
            let _ = cb.set_text(t);
        }
        (None, None) => {
            let _ = cb.clear();
        }
    }
}
