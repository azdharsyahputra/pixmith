//! Color profiles, cel z-indexes and reference layers: files from real tools, round trips through `.aseprite` and
//! `.pxmt`, the compositing order, exports that leave reference layers out, and the C entry points.

use pixmith_core::document::spritedata::{self, ColorProfile};
use pixmith_core::document::{Cel, Editor, Layer, Sprite, LAYER_EDITABLE, LAYER_REFERENCE};
use pixmith_core::formats::export_plan::{self, ExportOptions, Format, LayerMode};
use pixmith_core::formats::{ase, export, import, pxmt, sheet};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn fixture(rel: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aseprite").join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn load(rel: &str) -> Sprite {
    ase::read(&fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-spritedata-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cstr(p: &Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}

fn text(p: *mut std::ffi::c_char) -> String {
    assert!(!p.is_null());
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { pixmith_core::ffi::pixmith_string_free(p) };
    s
}

fn all_fixtures() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(p.extension().and_then(|e| e.to_str()), Some("aseprite" | "ase")) {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aseprite"), &mut v);
    v.sort();
    v
}

/// A 1x1 sprite with `n` layers whose cels are the given colors (bottom to top); `None` leaves a cel empty.
fn stack(colors: &[Option<[u8; 4]>]) -> Sprite {
    let mut s = Sprite::new(1, 1);
    while s.layers.len() < colors.len() {
        s.add_layer(s.layers.len() - 1);
    }
    for (i, c) in colors.iter().enumerate() {
        if let Some(c) = c {
            s.draw_line(i, 0, 0, 0, 0, 0, *c, 1);
        }
    }
    s
}

/// The sprite of a document the test owns through a raw pointer.
unsafe fn sprite_of<'a>(doc: *const Editor) -> &'a Sprite {
    &(*doc).sprite
}

fn px(s: &Sprite) -> [u8; 4] {
    s.render(0)[..4].try_into().unwrap()
}

// ---- color profile ---------------------------------------------------------------------------

#[test]
fn profiles_of_real_files_are_read() {
    assert_eq!(load("asefile/256_color_old_palette_chunk.aseprite").color_profile, ColorProfile::None);
    for f in ["asefile/basic-16x16.aseprite", "asefile/indexed.aseprite", "ase-go/test_nrgba.ase"] {
        let p = load(f).color_profile;
        assert_eq!(p.kind(), 1, "{f}");
        assert_eq!(p.label(), "sRGB");
    }
    for f in ["ase-go/prop.ase", "asefile/color-curve.aseprite"] {
        let p = load(f).color_profile;
        assert_eq!(p.kind(), 2, "{f}");
        let ColorProfile::Icc { data, .. } = &p else { unreachable!() };
        assert_eq!(&data[36..40], b"acsp", "{f}: an ICC profile carries its signature at byte 36");
        assert!(p.label().starts_with("Embedded ICC ("), "{}", p.label());
        assert!(!p.label().contains("unnamed"), "{f}: {}", p.label());
    }
    assert_eq!(load("ase-go/prop.ase").color_profile.label(), "Embedded ICC (Display P3)");
}

#[test]
fn every_fixture_keeps_its_profile_bytes_through_ase_and_pxmt() {
    let mut icc = 0;
    for path in all_fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let s = ase::read(&std::fs::read(&path).unwrap()).unwrap();
        let again = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(again.color_profile, s.color_profile, "{name}: .aseprite");
        let via_pxmt = pxmt::read(&pxmt::write(&s).unwrap()).unwrap();
        assert_eq!(via_pxmt.color_profile, s.color_profile, "{name}: .pxmt");
        icc += (s.color_profile.kind() == 2) as usize;
    }
    assert_eq!(icc, 2);
}

#[test]
fn a_profile_survives_a_second_save_byte_for_byte() {
    let s = load("ase-go/prop.ase");
    let first = ase::write(&s);
    let second = ase::write(&ase::read(&first).unwrap());
    assert_eq!(first, second);
    // The chunk is really in the file: type 2 with the profile length and bytes.
    let ColorProfile::Icc { data, .. } = &s.color_profile else { panic!("prop.ase has an ICC profile") };
    assert!(first.windows(data.len()).any(|w| w == data.as_slice()));
}

#[test]
fn pixels_are_not_converted_by_any_profile() {
    let mut s = load("asefile/basic-16x16.aseprite");
    let before = s.render(0);
    s.color_profile = ColorProfile::None;
    assert_eq!(s.render(0), before);
    let back = ase::read(&ase::write(&s)).unwrap();
    assert_eq!(back.color_profile, ColorProfile::None);
    assert_eq!(back.render(0), before);
}

#[test]
fn damaged_profile_chunks_read_as_none() {
    assert_eq!(ColorProfile::from_chunk(&[]), ColorProfile::None);
    let mut icc = vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    icc.extend(u32::MAX.to_le_bytes());
    assert_eq!(ColorProfile::from_chunk(&icc), ColorProfile::None, "the length runs past the chunk");
    assert_eq!(ColorProfile::from_chunk(&[9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), ColorProfile::None);
    assert_eq!(ColorProfile::from_chunk(&[1, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]).kind(), 1);
    assert_eq!(spritedata::icc_description(&[0; 10]), None);
    assert_eq!(spritedata::icc_description(&vec![0xFF; 400]), None);
}

#[test]
fn the_c_entry_points_show_set_and_remove_a_profile_with_undo() {
    unsafe {
        let doc = Box::into_raw(Box::new(Editor::new(load("ase-go/prop.ase"))));
        assert_eq!(spritedata::pixmith_color_profile_kind(doc), 2);
        assert_eq!(text(spritedata::pixmith_color_profile_label(doc)), "Embedded ICC (Display P3)");
        assert!(spritedata::pixmith_color_profile_icc_size(doc) > 400);
        // Replace it with plain sRGB, then remove it.
        assert!(spritedata::pixmith_color_profile_set(doc, 1));
        assert_eq!(text(spritedata::pixmith_color_profile_label(doc)), "sRGB");
        assert_eq!(spritedata::pixmith_color_profile_icc_size(doc), 0);
        assert!(!spritedata::pixmith_color_profile_set(doc, 1), "nothing changes");
        assert!(spritedata::pixmith_color_profile_set(doc, 0));
        assert_eq!(text(spritedata::pixmith_color_profile_label(doc)), "None");
        assert!(!spritedata::pixmith_color_profile_set(doc, 2), "an ICC profile can only be kept");
        // Every step is undoable, back to the embedded profile.
        assert!(pixmith_core::ffi::document::pixmith_undo(doc));
        assert_eq!(spritedata::pixmith_color_profile_kind(doc), 1);
        assert!(pixmith_core::ffi::document::pixmith_undo(doc));
        assert_eq!(spritedata::pixmith_color_profile_kind(doc), 2);
        assert_eq!(spritedata::pixmith_color_profile_kind(std::ptr::null()), -1);
        pixmith_core::ffi::document::pixmith_free(doc);
    }
}

// ---- z-index ----------------------------------------------------------------------------------

#[test]
fn z_indexes_of_real_files_are_read_and_kept() {
    for f in ["ase-go/test_nrgba.ase", "ase-go/test_paletted.ase", "ase-go/test_grayscale.ase"] {
        let s = load(f);
        let z: Vec<_> =
            (0..s.layers.len()).filter_map(|l| s.cel_z_index(l, 0).filter(|&z| z != 0).map(|z| (l, z))).collect();
        assert_eq!(z, vec![(2, 2)], "{f}: the third layer's cel has z-index 2");
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.cel_z_index(2, 0), Some(2), "{f}");
        assert_eq!(back.render(0), s.render(0), "{f}");
        let via_pxmt = pxmt::read(&pxmt::write(&s).unwrap()).unwrap();
        assert_eq!(via_pxmt.cel_z_index(2, 0), Some(2), "{f}: .pxmt");
    }
}

#[test]
fn files_without_z_indexes_are_unchanged_by_the_feature() {
    for path in all_fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("test_") {
            continue;
        }
        let s = ase::read(&std::fs::read(&path).unwrap()).unwrap();
        for f in 0..s.frame_count() {
            for l in 0..s.layers.len() {
                assert!(s.cel_z_index(l, f).is_none_or(|z| z == 0), "{name}");
            }
        }
    }
}

#[test]
fn cels_draw_in_order_of_layer_index_plus_z_index() {
    // Layers 0..2 are red, green, blue; the top layer has a hole at the pixel, so we see the topmost of the others.
    let mut s = stack(&[Some(RED), Some(GREEN), None]);
    assert_eq!(px(&s), GREEN);
    // +1 ties with layer 1 (index 1 + 0): the lower layer index draws first, so green stays on top.
    assert!(s.set_cel_z_index(0, 0, 1));
    assert_eq!(px(&s), GREEN);
    // +2 moves the red cel above the green one.
    assert!(s.set_cel_z_index(0, 0, 2));
    assert_eq!(px(&s), RED);
    // Back to 0, then push the green cel below the red one with a negative index.
    assert!(s.set_cel_z_index(0, 0, 0));
    assert!(s.set_cel_z_index(1, 0, -1));
    assert_eq!(px(&s), GREEN, "1 - 1 = 0 ties with layer 0 and layer 1 draws after it");
    assert!(s.set_cel_z_index(1, 0, -2));
    assert_eq!(px(&s), RED);
}

#[test]
fn ties_go_to_the_lower_layer_index_and_equal_orders_are_stable() {
    // Blue is layer 2 with z -1: order 1 ties with green (layer 1); green draws first, so blue ends up on top.
    let mut s = stack(&[Some(RED), Some(GREEN), Some(BLUE)]);
    assert!(s.set_cel_z_index(2, 0, -1));
    assert_eq!(px(&s), BLUE);
    // Blue at -2 has order 0 and ties with red: red (index 0) draws first, then blue, then green.
    assert!(s.set_cel_z_index(2, 0, -2));
    assert_eq!(px(&s), GREEN);
    // Blue at -3 goes underneath everything.
    assert!(s.set_cel_z_index(2, 0, -3));
    assert_eq!(px(&s), GREEN);
    // A translucent cel shows what it is drawn over.
    let mut s = stack(&[Some(RED), Some([0, 0, 255, 127]), Some(GREEN)]);
    assert!(s.set_cel_z_index(1, 0, -1));
    let under = px(&s);
    assert_eq!(under, GREEN, "an opaque cel on top hides everything");
    assert!(s.set_cel_z_index(1, 0, 5));
    let over = px(&s);
    assert_ne!(over, GREEN);
    assert!(over[2] > 100 && over[1] > 100, "blue drawn over green: {over:?}");
}

#[test]
fn z_index_orders_within_groups_and_uses_global_layer_indexes() {
    // Layers: 0 red, 1 group G, 2 green (in G), 3 blue (in G), 4 yellow (outside, on top, empty at the pixel).
    let mut s = stack(&[Some(RED), None, Some(GREEN), Some(BLUE), None]);
    s.layers[1].kind = 1;
    s.layers[2].child_level = 1;
    s.layers[3].child_level = 1;
    assert_eq!(px(&s), BLUE);
    // Inside the group, green (2) with +2 has order 4 and goes above blue (3).
    assert!(s.set_cel_z_index(2, 0, 2));
    assert_eq!(px(&s), GREEN);
    // The red cel cannot enter a group: its order is compared with the group's own place (layer 1).
    assert!(s.set_cel_z_index(2, 0, 0));
    assert!(s.set_cel_z_index(0, 0, 3));
    assert_eq!(px(&s), RED, "red now draws after the whole group");
    assert!(s.set_cel_z_index(0, 0, 1));
    assert_eq!(px(&s), BLUE, "order 1 ties with the group: the lower index (red) draws first, the group covers it");
}

#[test]
fn z_index_is_clamped_shared_by_linked_cels_and_empty_cels_refuse_it() {
    let mut s = Sprite::new(1, 1);
    s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
    s.add_frame(0, true);
    s.link_cels(0, 0, 1);
    assert!(s.set_cel_z_index(0, 1, 100_000));
    assert_eq!(s.cel_z_index(0, 0), Some(i16::MAX), "clamped, and shared by the linked cel");
    assert!(s.set_cel_z_index(0, 0, -100_000));
    assert_eq!(s.cel_z_index(0, 1), Some(i16::MIN));
    let back = ase::read(&ase::write(&s)).unwrap();
    assert_eq!(back.cel_z_index(0, 0), Some(i16::MIN));
    assert!(back.is_cel_linked(0, 0));
    let mut empty = Sprite::new(1, 1);
    assert_eq!(empty.cel_z_index(0, 0), None);
    assert!(!empty.set_cel_z_index(0, 0, 3));
}

#[test]
fn z_index_round_trips_on_normal_and_tilemap_cels() {
    let mut s = stack(&[Some(RED), Some(GREEN)]);
    assert!(s.set_cel_z_index(0, 0, 7));
    assert!(s.set_cel_z_index(1, 0, -3));
    for (name, back) in
        [(".aseprite", ase::read(&ase::write(&s)).unwrap()), (".pxmt", pxmt::read(&pxmt::write(&s).unwrap()).unwrap())]
    {
        assert_eq!((back.cel_z_index(0, 0), back.cel_z_index(1, 0)), (Some(7), Some(-3)), "{name}");
        assert_eq!(back.render(0), s.render(0), "{name}");
    }
    let tiles = load("asefile/tilemap.aseprite");
    let layer = tiles.layers.iter().position(|l| l.kind == 2).expect("a tilemap layer");
    let frame = (0..tiles.frame_count()).find(|&f| tiles.cels[f][layer].is_some()).unwrap();
    let mut tiles = tiles;
    assert!(tiles.set_cel_z_index(layer, frame, 4));
    let back = ase::read(&ase::write(&tiles)).unwrap();
    assert_eq!(back.cel_z_index(layer, frame), Some(4));
}

#[test]
fn the_c_entry_points_read_and_set_z_indexes_with_undo() {
    unsafe {
        let doc = Box::into_raw(Box::new(Editor::new(stack(&[Some(RED), Some(GREEN)]))));
        let mut z = 99;
        assert!(spritedata::pixmith_cel_z_index(doc, 0, 0, &mut z));
        assert_eq!(z, 0);
        assert!(spritedata::pixmith_cel_set_z_index(doc, 0, 0, 5));
        assert!(spritedata::pixmith_cel_z_index(doc, 0, 0, &mut z));
        assert_eq!(z, 5);
        let mut rgba = [0u8; 4];
        assert!(pixmith_core::ffi::document::pixmith_render(doc, 0, rgba.as_mut_ptr(), 4));
        assert_eq!(rgba, RED, "the red cel is now drawn above the green one");
        assert!(!spritedata::pixmith_cel_set_z_index(doc, 9, 0, 1));
        assert!(!spritedata::pixmith_cel_z_index(doc, 9, 0, &mut z));
        assert!(pixmith_core::ffi::document::pixmith_undo(doc));
        assert!(spritedata::pixmith_cel_z_index(doc, 0, 0, &mut z));
        assert_eq!(z, 0);
        assert!(pixmith_core::ffi::document::pixmith_render(doc, 0, rgba.as_mut_ptr(), 4));
        assert_eq!(rgba, GREEN);
        pixmith_core::ffi::document::pixmith_free(doc);
    }
}

// ---- reference layers --------------------------------------------------------------------------

fn write_png(path: &Path, w: u32, h: u32, color: [u8; 4]) {
    let s = {
        let mut s = Sprite::new(w, h);
        for y in 0..h as i32 {
            s.draw_line(0, 0, 0, y, w as i32 - 1, y, color, 1);
        }
        s
    };
    std::fs::write(path, export::frame_png(&s, 0, 1).unwrap()).unwrap();
}

/// A 4x4 sprite with a red pixel on its normal layer.
fn with_reference() -> Sprite {
    let mut s = Sprite::new(4, 4);
    s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
    let r = s.add_layer(0);
    s.layers[r] = Layer { name: "ref".into(), ..Layer::new_reference("ref") };
    let mut pixels = vec![0u8; 4 * 4 * 4];
    for p in pixels.as_chunks_mut::<4>().0.iter_mut().skip(1) {
        p.copy_from_slice(&[0, 0, 255, 255]);
    }
    s.cels[0][r] = Some(Arc::new(Cel { pixels, ..Default::default() }));
    s
}

#[test]
fn the_reference_flag_round_trips_through_ase_and_pxmt() {
    let s = with_reference();
    assert!(s.layers[1].is_reference() && s.layers[1].is_locked());
    assert_eq!(s.layers[1].flags & LAYER_REFERENCE, LAYER_REFERENCE);
    for (name, back) in
        [(".aseprite", ase::read(&ase::write(&s)).unwrap()), (".pxmt", pxmt::read(&pxmt::write(&s).unwrap()).unwrap())]
    {
        assert!(back.layers[1].is_reference(), "{name}");
        assert!(back.layers[1].is_locked(), "{name}");
        assert!(!back.layers[0].is_reference(), "{name}");
        assert_eq!(back.layers[1].flags, s.layers[1].flags, "{name}");
        assert_eq!(back.cels[0][1], s.cels[0][1], "{name}: the image is kept");
    }
    let mut unlocked = s.clone();
    assert!(unlocked.set_layer_locked(1, false));
    assert_eq!(unlocked.layers[1].flags, LAYER_REFERENCE | LAYER_EDITABLE);
    assert!(!ase::read(&ase::write(&unlocked)).unwrap().layers[1].is_locked());
}

#[test]
fn reference_layers_are_left_out_of_every_render_and_export() {
    let s = with_reference();
    // The normal render, thumbnails, PNG, GIF, sheets: only the red pixel.
    let mut expected = vec![0u8; 4 * 4 * 4];
    expected[..4].copy_from_slice(&RED);
    assert_eq!(s.render(0), expected);
    let png = export::frame_png(&s, 0, 1).unwrap();
    let (_, _, decoded) = import::decode_png(&png).unwrap();
    assert_eq!(decoded, expected);
    let (sheet_png, _json) = sheet::export(&s, sheet::Options { scale: 1, ..Default::default() }, "x.png").unwrap();
    let (_, _, sheet_px) = import::decode_png(&sheet_png).unwrap();
    assert_eq!(sheet_px, expected);
    let gif = pixmith_core::formats::gif::gif_bytes(&s, 1).unwrap();
    assert!(gif.starts_with(b"GIF89a"));
    // Per-layer export lists only the normal layer, and the reference layer cannot be picked.
    let each = export_plan::plan(&s, &ExportOptions { layers: LayerMode::Each, ..Default::default() }).unwrap();
    assert_eq!(each.len(), 1);
    assert_eq!(each[0].parts, vec![s.layers[0].name.clone()]);
    assert!(export_plan::plan(&s, &ExportOptions { layers: LayerMode::Selected(1), ..Default::default() }).is_err());
    let enc = export_plan::EncodeOptions {
        scale: 1,
        frame: 0,
        sheet: Default::default(),
        write_json: false,
        image: Default::default(),
    };
    let outputs = export_plan::render_outputs(&s, Format::Png, &ExportOptions::default(), &enc, "x.png", None).unwrap();
    assert_eq!(outputs.len(), 1);
    // A sprite whose only content is a reference layer exports as an empty image.
    let mut only = s.clone();
    only.cels[0][0] = None;
    assert!(only.render(0).iter().all(|&b| b == 0));
}

#[test]
fn the_editor_draws_reference_layers_at_the_display_opacity_and_exports_never_do() {
    unsafe {
        let doc = Box::into_raw(Box::new(Editor::new(with_reference())));
        let mut full = vec![0u8; 64];
        assert_eq!(spritedata::pixmith_reference_display(doc), 100);
        assert!(pixmith_core::ffi::document::pixmith_render(doc, 0, full.as_mut_ptr(), 64));
        assert_eq!(&full[..4], RED, "the reference image has a hole at the first pixel");
        assert_eq!(&full[4..8], BLUE, "reference pixels show in the editor");
        // 50%: half-transparent blue.
        spritedata::pixmith_set_reference_display(doc, 50);
        let mut half = vec![0u8; 64];
        assert!(pixmith_core::ffi::document::pixmith_render(doc, 0, half.as_mut_ptr(), 64));
        assert_eq!(&half[4..8], [0, 0, 255, 127]);
        // Region renders agree with the full render.
        let mut rect = [0u8; 4];
        assert!(pixmith_core::render::view::pixmith_render_rect(doc, 0, 1, 0, 1, 1, rect.as_mut_ptr(), 4));
        assert_eq!(rect, [0, 0, 255, 127]);
        // 0 hides them.
        spritedata::pixmith_set_reference_display(doc, 0);
        assert!(pixmith_core::ffi::document::pixmith_render(doc, 0, half.as_mut_ptr(), 64));
        assert_eq!(&half[4..8], [0, 0, 0, 0]);
        // The document-level render used by exports ignores the setting.
        spritedata::pixmith_set_reference_display(doc, 100);
        assert_eq!(sprite_of(doc).render(0)[4..8], [0, 0, 0, 0]);
        pixmith_core::ffi::document::pixmith_free(doc);
    }
}

#[test]
fn a_reference_layer_can_be_hidden_like_any_layer() {
    let mut s = with_reference();
    s.layers[1].visible = false;
    assert_eq!(pixmith_core::document::layers::render_frame_rect_with(&s, 0, (0, 0, 4, 4), 100)[4..8], [0, 0, 0, 0]);
    s.layers[1].visible = true;
    assert_eq!(pixmith_core::document::layers::render_frame_rect_with(&s, 0, (0, 0, 4, 4), 100)[4..8], BLUE);
}

#[test]
fn locked_reference_layers_refuse_drawing_until_unlocked() {
    let mut s = with_reference();
    let before = s.cels[0][1].clone();
    s.draw_line(1, 0, 0, 0, 3, 3, GREEN, 1);
    assert_eq!(s.cels[0][1], before, "the locked layer did not change");
    assert!(s.set_cel_opacity(1, 0, 100), "properties can still be edited");
    assert!(s.set_cel_z_index(1, 0, 2));
    assert!(s.set_layer_locked(1, false));
    s.draw_line(1, 0, 0, 0, 0, 0, GREEN, 1);
    assert_eq!(s.cels[0][1].as_ref().unwrap().pixels[..4], GREEN);
    // Normal layers are never refused.
    assert!(s.set_layer_locked(0, true));
    s.draw_line(0, 0, 1, 1, 1, 1, RED, 1);
    assert_eq!(s.cels[0][0].as_ref().unwrap().pixels[(4 + 1) * 4..][..4], RED);
}

#[test]
fn converting_a_layer_to_a_reference_layer_and_back() {
    let mut s = Sprite::new(2, 2);
    assert!(!s.layers[0].is_reference());
    assert!(s.set_layer_reference(0, true));
    assert!(s.layers[0].is_reference() && s.layers[0].is_locked());
    assert!(s.set_layer_reference(0, false));
    assert!(!s.layers[0].is_reference() && !s.layers[0].is_locked());
    s.layers[0].kind = 1;
    assert!(!s.set_layer_reference(0, true), "groups cannot be reference layers");
    // A reference layer cannot be merged with its neighbour.
    let mut s = with_reference();
    assert_eq!(s.merge_down(1), None);
}

#[test]
fn new_reference_layer_from_file_scales_to_fit_and_is_undoable() {
    let dir = temp_dir("newref");
    let wide = dir.join("wide.png");
    write_png(&wide, 8, 2, BLUE);
    let mut e = Editor::new(Sprite::new(4, 4));
    e.sprite.add_frame(0, false);
    unsafe {
        let doc: *mut Editor = &mut e;
        let at = spritedata::pixmith_layer_new_reference(doc, 0, cstr(&wide).as_ptr());
        assert_eq!(at, 1);
        let s = &(*doc).sprite;
        assert_eq!(s.layers[1].name, "wide");
        assert!(s.layers[1].is_reference() && s.layers[1].is_locked());
        // 8x2 fits 4 wide and 1 high: one row in the middle (row 1 of 0..4: (4 - 1) / 2).
        let px = &s.cels[0][1].as_ref().unwrap().pixels;
        let rows: Vec<usize> = (0..4).filter(|&y| (0..4).any(|x| px[(y * 4 + x) * 4 + 3] != 0)).collect();
        assert_eq!(rows, vec![1]);
        assert!((0..4).all(|x| px[(4 + x) * 4..][..4] == BLUE));
        // The image is on every frame (the frames share one buffer).
        assert!(Arc::ptr_eq(s.cels[0][1].as_ref().unwrap(), s.cels[1][1].as_ref().unwrap()));
        // Not part of the normal render.
        assert!(s.render(0).iter().all(|&b| b == 0));
        // Unreadable files report an error and change nothing.
        let missing = dir.join("missing.png");
        assert_eq!(spritedata::pixmith_layer_new_reference(doc, 0, cstr(&missing).as_ptr()), -1);
        assert_eq!(sprite_of(doc).layers.len(), 2);
        // Undo removes the layer, redo brings it back.
        assert!(pixmith_core::ffi::document::pixmith_undo(doc));
        assert_eq!(sprite_of(doc).layers.len(), 1);
        assert!(pixmith_core::ffi::document::pixmith_redo(doc));
        assert!(sprite_of(doc).layers[1].is_reference());
        assert!(spritedata::pixmith_layer_is_reference(doc, 1));
        assert!(spritedata::pixmith_layer_is_locked(doc, 1));
        assert!(spritedata::pixmith_layer_set_locked(doc, 1, false));
        assert!(!spritedata::pixmith_layer_is_locked(doc, 1));
        assert!(spritedata::pixmith_layer_set_reference(doc, 1, false));
        assert!(!spritedata::pixmith_layer_is_reference(doc, 1));
        assert!(!spritedata::pixmith_layer_set_reference(doc, 99, true));
    }
    // And it survives saving.
    let back = ase::read(&ase::write(&e.sprite)).unwrap();
    assert_eq!(back.layers.len(), 2);
}

#[test]
fn fitting_keeps_the_aspect_ratio_and_sharp_pixels_when_growing() {
    // 2x2 into 6x6: each pixel becomes 3x3.
    let src: Vec<u8> = [RED, GREEN, BLUE, [255, 255, 0, 255]].concat();
    let out = spritedata::fit_to_canvas(&src, 2, 2, 6, 6);
    let at = |x: usize, y: usize| <[u8; 4]>::try_from(&out[(y * 6 + x) * 4..][..4]).unwrap();
    assert_eq!((at(0, 0), at(2, 2), at(3, 0), at(0, 3), at(5, 5)), (RED, RED, GREEN, BLUE, [255, 255, 0, 255]));
    // Shrinking averages: a 4x4 checker of black and white into 2x2 is mid gray everywhere.
    let checker: Vec<u8> =
        (0..16).flat_map(|i| if (i % 4 + i / 4) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
    let out = spritedata::fit_to_canvas(&checker, 4, 4, 2, 2);
    assert!(out.as_chunks::<4>().0.iter().all(|p| (126..=129).contains(&p[0]) && p[3] == 255), "{out:?}");
    // A tall image is centered horizontally.
    let tall: Vec<u8> = RED.repeat(2 * 8);
    let out = spritedata::fit_to_canvas(&tall, 2, 8, 8, 8);
    let cols: Vec<usize> = (0..8).filter(|&x| out[x * 4 + 3] != 0).collect();
    assert_eq!(cols, vec![3, 4]);
    // Degenerate input gives an empty canvas.
    assert!(spritedata::fit_to_canvas(&[], 0, 0, 3, 3).iter().all(|&b| b == 0));
}

#[test]
fn z_index_and_reference_flag_survive_canvas_and_scale_edits() {
    use pixmith_core::paint::{filters, selection};
    let mut e = Editor::new(with_reference());
    assert!(e.sprite.set_cel_z_index(0, 0, 3));
    assert!(e.sprite.set_cel_z_index(1, 0, -2));
    assert!(selection::resize_canvas(&mut e, 6, 6, 1, 1));
    assert_eq!((e.sprite.cel_z_index(0, 0), e.sprite.cel_z_index(1, 0)), (Some(3), Some(-2)));
    assert!(e.sprite.layers[1].is_reference());
    assert!(filters::scale_sprite(&mut e.sprite, 12, 12));
    assert_eq!((e.sprite.cel_z_index(0, 0), e.sprite.cel_z_index(1, 0)), (Some(3), Some(-2)));
}

#[test]
fn undo_and_redo_restore_z_indexes_through_many_steps() {
    unsafe {
        let doc = Box::into_raw(Box::new(Editor::new(stack(&[Some(RED), Some(GREEN)]))));
        for z in 1..=40 {
            assert!(spritedata::pixmith_cel_set_z_index(doc, 0, 0, z));
            // Interleave pixel edits so that history entries are stored as deltas.
            (*doc).checkpoint();
            (*doc).sprite.draw_line(1, 0, 0, 0, 0, 0, [z as u8, 0, 0, 255], 1);
        }
        let mut z = 0;
        let mut last = 41;
        while pixmith_core::ffi::document::pixmith_undo(doc) {
            if spritedata::pixmith_cel_z_index(doc, 0, 0, &mut z) {
                assert!(z <= last, "z-index {z} after {last}");
                last = z;
            }
        }
        assert_eq!(z, 0);
        let mut seen = 0;
        while pixmith_core::ffi::document::pixmith_redo(doc) {
            seen += 1;
        }
        assert!(seen >= 40);
        assert!(spritedata::pixmith_cel_z_index(doc, 0, 0, &mut z));
        assert_eq!(z, 40);
        pixmith_core::ffi::document::pixmith_free(doc);
    }
}

#[test]
fn the_command_line_exports_leave_reference_layers_out() {
    use std::process::Command;
    let dir = temp_dir("cli");
    let file = dir.join("ref.aseprite");
    std::fs::write(&file, ase::write(&with_reference())).unwrap();
    let pxmt_file = dir.join("ref.pxmt");
    std::fs::write(&pxmt_file, pxmt::write(&with_reference()).unwrap()).unwrap();
    for input in [&file, &pxmt_file] {
        let stem = input.file_stem().unwrap().to_string_lossy().into_owned();
        let png = dir.join(format!("{stem}-frame.png"));
        let sheet_png = dir.join(format!("{stem}-sheet.png"));
        let gif = dir.join(format!("{stem}.gif"));
        let out = Command::new(env!("CARGO_BIN_EXE_pixmith-cli"))
            .args(["export", input.to_str().unwrap(), "--png", png.to_str().unwrap()])
            .args(["--sheet", sheet_png.to_str().unwrap(), "--gif", gif.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        for path in [&png, &sheet_png] {
            let (_, _, px) = import::decode_png(&std::fs::read(path).unwrap()).unwrap();
            assert!(
                px.as_chunks::<4>().0.iter().all(|p| p[3] == 0 || *p == RED),
                "{}: only the normal layer is exported",
                path.display()
            );
            assert!(px.as_chunks::<4>().0.contains(&RED));
        }
        // Picking the reference layer by name renders nothing, and --split-layers skips it.
        let picked = dir.join(format!("{stem}-picked.png"));
        let by_name = Command::new(env!("CARGO_BIN_EXE_pixmith-cli"))
            .args(["export", input.to_str().unwrap(), "--png", picked.to_str().unwrap(), "--layer", "ref"])
            .output()
            .unwrap();
        assert!(by_name.status.success(), "{}", String::from_utf8_lossy(&by_name.stderr));
        let (_, _, px) = import::decode_png(&std::fs::read(&picked).unwrap()).unwrap();
        assert!(px.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
        let split = dir.join(format!("{stem}-split.png"));
        let out = Command::new(env!("CARGO_BIN_EXE_pixmith-cli"))
            .args(["export", input.to_str().unwrap(), "--png", split.to_str().unwrap(), "--split-layers"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(&format!("{stem}-split")))
            .collect();
        assert_eq!(files.len(), 1, "only the normal layer gets a file: {files:?}");
    }
    // Converting keeps the layer (it is a save, not an export).
    let converted = dir.join("converted.pxmt");
    let out = Command::new(env!("CARGO_BIN_EXE_pixmith-cli"))
        .args(["convert", file.to_str().unwrap(), converted.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(pxmt::open_file(&converted).unwrap().layers[1].is_reference());
}

#[test]
fn lua_reads_and_writes_z_index_and_the_reference_flag() {
    unsafe {
        let doc = Box::into_raw(Box::new(Editor::new(stack(&[Some(RED), Some(GREEN)]))));
        let run = |src: &str| {
            let c = CString::new(src).unwrap();
            let mut messages = std::ptr::null_mut();
            let ok = pixmith_core::scripting::pixmith_run_script(doc, c.as_ptr(), &mut messages);
            let out = if messages.is_null() { String::new() } else { text(messages) };
            (ok, out)
        };
        let (ok, out) = run("local c = sprite:cel(1, 1)\n\
             assert(c.zIndex == 0)\n\
             c.zIndex = 2\n\
             assert(sprite:cel(1, 1).zIndex == 2)\n\
             local ref = sprite.layers[2]\n\
             assert(ref.isReference == false)\n\
             ref.isReference = true\n\
             assert(sprite.layers[2].isReference)\n\
             print('ok')");
        assert!(ok, "{out}");
        assert_eq!((*doc).sprite.cel_z_index(0, 0), Some(2));
        assert!(sprite_of(doc).layers[1].is_reference());
        // The cel of the first layer now draws above the second; the reference layer is not rendered.
        assert_eq!(sprite_of(doc).render(0)[..4], RED);
        // Errors: empty cels and groups.
        let (ok, _) = run("sprite:newLayer('empty'); sprite:cel(3, 1).zIndex = 1");
        assert!(!ok, "an empty cel refuses a z-index");
        pixmith_core::ffi::document::pixmith_free(doc);
    }
}

#[test]
fn filters_leave_locked_reference_layers_alone() {
    use pixmith_core::paint::filters::{self, Filter, Target};
    let mut s = with_reference();
    let reference = s.cels[0][1].clone();
    let normal = s.cels[0][0].clone();
    // A filter on every layer changes the normal layer only.
    assert!(filters::apply(&mut s, &Filter::Invert, Target::AllLayers, 0, 0, None));
    assert_eq!(s.cels[0][1], reference, "the locked reference layer was filtered");
    assert_ne!(s.cels[0][0], normal);
    // Aimed straight at it, nothing changes either.
    assert!(!filters::apply(&mut s, &Filter::Invert, Target::Cel, 1, 0, None));
    // Once unlocked it is an ordinary target again.
    assert!(s.set_layer_locked(1, false));
    assert!(filters::apply(&mut s, &Filter::Invert, Target::Cel, 1, 0, None));
    assert_ne!(s.cels[0][1], reference);
}
