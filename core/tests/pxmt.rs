//! `.pxmt` documents: every fixture round-trips, thumbnails, damaged input, the C entry points and the CLI.

use pixmith_core::document::{Editor, Sprite, Tag};
use pixmith_core::formats::pxmt::{self, Thumbnail};
use pixmith_core::formats::{ase, export, import};
use pixmith_core::gamedev::godot::settings;
use std::ffi::{CStr, CString};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> Vec<PathBuf> {
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
    assert!(v.len() >= 30, "fixtures went missing");
    v
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-pxmt-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cstr(p: &Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}

fn sample() -> Sprite {
    let mut s = Sprite::new(10, 6);
    s.layers[0].name = "bg".into();
    s.draw_line(0, 0, 0, 0, 9, 5, [200, 30, 30, 255], 1);
    let top = s.add_layer(0);
    s.layers[top].name = "top".into();
    s.layers[top].opacity = 128;
    s.draw_line(top, 0, 0, 5, 9, 0, [0, 0, 255, 255], 2);
    s.add_frame(0, true);
    s.durations = vec![80, 120];
    s.tags.push(Tag {
        name: "all".into(),
        from: 0,
        to: 1,
        direction: 2,
        repeat: 3,
        color: [9, 8, 7],
        user_data: Default::default(),
    });
    s
}

/// Everything the editor shows must be equal, not only the pixels.
fn assert_same(a: &Sprite, b: &Sprite, what: &str) {
    assert_eq!((a.width, a.height), (b.width, b.height), "{what}: size");
    assert_eq!(a.layers, b.layers, "{what}: layers");
    assert_eq!(a.durations, b.durations, "{what}: durations");
    assert_eq!(a.tags, b.tags, "{what}: tags");
    assert_eq!(a.palette, b.palette, "{what}: palette");
    assert_eq!(a.color_mode, b.color_mode, "{what}: color mode");
    assert_eq!(a.transparent_index, b.transparent_index, "{what}: transparent index");
    assert_eq!(a.tilesets, b.tilesets, "{what}: tilesets");
    assert_eq!(a.extra, b.extra, "{what}: slices and user data");
    for f in 0..a.frame_count() {
        assert_eq!(a.render(f), b.render(f), "{what}: frame {} renders differently", f + 1);
        for l in 0..a.layers.len() {
            assert_eq!(a.cels[f][l], b.cels[f][l], "{what}: cel (frame {}, layer {})", f + 1, l);
        }
    }
    assert!(a == b, "{what}: sprites differ in an unlisted field");
}

/// Splits a `.pxmt` file into (tag, payload) pairs.
fn chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut v = Vec::new();
    let mut at = 10;
    while at + 8 <= bytes.len() {
        let tag: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        v.push((tag, bytes[at + 8..at + 8 + len].to_vec()));
        at += 8 + len + 4;
    }
    v
}

#[test]
fn every_fixture_round_trips_through_pxmt() {
    let mut failures = Vec::new();
    for path in fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let original = ase::read(&std::fs::read(&path).unwrap()).unwrap();
        let outcome = std::panic::catch_unwind(|| {
            let bytes = pxmt::write(&original).unwrap();
            let back = pxmt::read(&bytes).unwrap();
            assert_same(&original, &back, &name);
            // Same result as going through .aseprite directly, and a second generation is stable.
            assert!(
                back == ase::read(&ase::write(&original)).unwrap(),
                "{name}: differs from the .aseprite round trip"
            );
            let again = pxmt::read(&pxmt::write(&back).unwrap()).unwrap();
            assert_same(&back, &again, &name);
            // META agrees with the sprite.
            let meta = pxmt::read_meta(Cursor::new(&bytes)).unwrap().unwrap();
            assert_eq!(
                (meta.width, meta.height, meta.frames, meta.layers),
                (original.width, original.height, original.frame_count() as u32, original.layers.len() as u32),
                "{name}: META"
            );
            // read_document picks the format by content, whatever the name.
            assert_same(&original, &pxmt::read_document(&bytes).unwrap(), &name);
        });
        if outcome.is_err() {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "failed: {failures:?}");
}

#[test]
fn every_fixture_has_a_fitted_thumbnail_of_frame_one() {
    for path in fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let s = ase::read(&std::fs::read(&path).unwrap()).unwrap();
        let bytes = pxmt::write(&s).unwrap();
        let t = pxmt::read_thumbnail(Cursor::new(&bytes)).unwrap().unwrap_or_else(|| panic!("{name}: no thumbnail"));
        let (w, h) = pxmt::fit_size(s.width, s.height, 256, 256, false);
        assert_eq!((t.width, t.height), (w, h), "{name}");
        assert!(t.width <= 256 && t.height <= 256 && t.rgba.len() == (w * h * 4) as usize, "{name}");
        if s.width <= 256 && s.height <= 256 {
            assert_eq!(t.rgba, s.render(0), "{name}: small sprites are stored 1:1");
        }
        // The same through the THMB-only reader that never decodes the sprite.
        let dir = temp_dir("thumbs");
        let file = dir.join("t.pxmt");
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(pxmt::read_thumbnail_file(&file).unwrap().unwrap(), t, "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_large_canvas_gets_a_downscaled_thumbnail() {
    let mut s = Sprite::new(1024, 512);
    s.fill(0, 0, 0, 0, [10, 200, 30, 255]);
    let bytes = pxmt::write(&s).unwrap();
    let t = pxmt::read_thumbnail(Cursor::new(&bytes)).unwrap().unwrap();
    assert_eq!((t.width, t.height), (256, 128));
    assert!(t.rgba.as_chunks::<4>().0.iter().all(|p| *p == [10, 200, 30, 255]));
    // Asking for a smaller preview fits again; asking for a bigger one scales up by a whole factor.
    let small = t.fitted(100, 100).unwrap();
    assert_eq!((small.width, small.height), (100, 50));
    let big = Thumbnail { width: 16, height: 8, rgba: vec![255; 16 * 8 * 4] }.fitted(100, 100).unwrap();
    assert_eq!((big.width, big.height), (96, 48));
}

#[test]
fn file_thumbnail_for_every_kind_of_file() {
    let dir = temp_dir("filethumb");
    let s = sample();
    let (pxmt_path, ase_path, png_path) = (dir.join("a.pxmt"), dir.join("a.aseprite"), dir.join("a.png"));
    std::fs::write(&pxmt_path, pxmt::write(&s).unwrap()).unwrap();
    std::fs::write(&ase_path, ase::write(&s)).unwrap();
    std::fs::write(&png_path, export::frame_png(&s, 0, 1).unwrap()).unwrap();
    let want = s.render(0);
    for p in [&pxmt_path, &ase_path, &png_path] {
        let t = pxmt::file_thumbnail(p, 10, 10).unwrap();
        assert_eq!((t.width, t.height), (10, 6), "{p:?}");
        assert_eq!(t.rgba, want, "{p:?}");
        let t = pxmt::file_thumbnail(p, 30, 30).unwrap();
        assert_eq!((t.width, t.height), (30, 18), "{p:?}");
        let t = pxmt::file_thumbnail(p, 5, 5).unwrap();
        assert_eq!((t.width, t.height), (5, 3), "{p:?}");
        assert!(pxmt::file_thumbnail(p, 0, 5).is_err());
    }
    // A .pxmt without THMB (written by something else) falls back to decoding the sprite.
    let bytes = pxmt::write(&s).unwrap();
    let kept: Vec<_> = chunks(&bytes).into_iter().filter(|(t, _)| t != b"THMB").collect();
    std::fs::write(&pxmt_path, rebuild(&kept)).unwrap();
    assert!(pxmt::read_thumbnail_file(&pxmt_path).unwrap().is_none());
    assert_eq!(pxmt::file_thumbnail(&pxmt_path, 10, 10).unwrap().rgba, want);
    // So does one whose thumbnail is damaged.
    let mut bad = bytes.clone();
    let at = bad.windows(4).position(|w| w == b"THMB").unwrap();
    bad[at + 12] ^= 0xff;
    std::fs::write(&pxmt_path, &bad).unwrap();
    assert!(pxmt::read_thumbnail_file(&pxmt_path).is_err());
    assert_eq!(pxmt::file_thumbnail(&pxmt_path, 10, 10).unwrap().rgba, want);
    assert_same(&s, &pxmt::read(&bad).unwrap(), "damaged thumbnail does not matter");
    // Garbage, empty and missing files are errors.
    std::fs::write(dir.join("junk.pxmt"), b"hello").unwrap();
    std::fs::write(dir.join("empty.png"), b"").unwrap();
    assert!(pxmt::file_thumbnail(dir.join("junk.pxmt"), 10, 10).is_err());
    assert!(pxmt::file_thumbnail(dir.join("empty.png"), 10, 10).is_err());
    assert!(pxmt::file_thumbnail(dir.join("missing.pxmt"), 10, 10).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

fn crc(tag: &[u8], payload: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(tag);
    c.update(payload);
    c.sum()
}

fn rebuild(parts: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = b"PXMT\r\n\x1a\n\x01\x00".to_vec();
    for (tag, payload) in parts {
        out.extend(tag);
        out.extend((payload.len() as u32).to_le_bytes());
        out.extend(payload);
        out.extend(crc(tag, payload).to_le_bytes());
    }
    out.extend(b"PEND");
    out.extend(0u32.to_le_bytes());
    out.extend(crc(b"PEND", &[]).to_le_bytes());
    out
}

#[test]
fn unknown_chunks_are_skipped_in_any_position() {
    let s = sample();
    let parts = chunks(&pxmt::write(&s).unwrap());
    let kinds: Vec<_> = parts.iter().map(|(t, _)| *t).collect();
    assert_eq!(kinds, [*b"META", *b"THMB", *b"SPRT", *b"PEND"]);
    let extra = (*b"fOOb", b"data from the future".to_vec());
    let live = parts[..3].to_vec();
    for at in 0..=3 {
        let mut v = live.clone();
        v.insert(at, extra.clone());
        let bytes = rebuild(&v);
        assert_same(&s, &pxmt::read(&bytes).unwrap(), "unknown chunk");
        assert!(pxmt::read_meta(Cursor::new(&bytes)).unwrap().is_some());
        assert!(pxmt::read_thumbnail(Cursor::new(&bytes)).unwrap().is_some());
    }
    // Chunk order does not matter and a skipped chunk's checksum is not checked.
    let mut reordered = vec![live[2].clone(), live[1].clone(), live[0].clone()];
    assert_same(&s, &pxmt::read(&rebuild(&reordered)).unwrap(), "reordered");
    reordered.insert(0, extra);
    let mut bytes = rebuild(&reordered);
    bytes[10 + 8] ^= 0xff; // inside the unknown chunk's payload
    assert_same(&s, &pxmt::read(&bytes).unwrap(), "unchecked unknown chunk");
    // A file without META or THMB is still a document.
    assert_same(&s, &pxmt::read(&rebuild(&live[2..])).unwrap(), "sprite only");
    assert!(pxmt::read_meta(Cursor::new(rebuild(&live[2..]))).unwrap().is_none());
    // A newer META keeps its extra members when the document is saved again.
    let mut meta = pxmt::Meta::from_json(&String::from_utf8(live[0].1.clone()).unwrap()).unwrap();
    meta.extra.push(("future".into(), pixmith_core::formats::json::parse(r#"{"x":[1,true,"é"]}"#).unwrap()));
    meta.created = 1234;
    let resaved = pxmt::write_with(&s, Some(&meta)).unwrap();
    let back = pxmt::read_meta(Cursor::new(&resaved)).unwrap().unwrap();
    assert_eq!(back.created, 1234);
    assert_eq!(back.extra, meta.extra);
    assert!(back.modified >= meta.modified);
}

#[test]
fn damaged_and_hostile_input_is_an_error_not_a_panic() {
    let s = sample();
    let good = pxmt::write(&s).unwrap();
    let err = |b: &[u8]| pxmt::read(b).unwrap_err();
    assert!(err(b"").contains("not a Pixmith document"));
    assert!(err(b"PXMT").contains("not a Pixmith document"));
    assert!(err(&[0u8; 4096]).contains("not a Pixmith document"));
    assert!(err(b"PXMT\r\n\x1a\n").contains("not a Pixmith document"));
    assert!(err(&ase::write(&s)).contains("not a Pixmith document"));
    assert!(err(b"PXMT\r\n\x1a\n\x01\x00").contains("truncated"));
    // A newer version is refused with a hint, not misread.
    let mut future = good.clone();
    future[8] = 3;
    assert!(err(&future).contains("version 3") && err(&future).contains("update Pixmith"));
    // A flipped payload byte is caught by the checksum.
    let mut flipped = good.clone();
    let at = flipped.windows(4).position(|w| w == b"SPRT").unwrap() + 40;
    flipped[at] ^= 1;
    assert!(err(&flipped).contains("checksum"), "{}", err(&flipped));
    // A chunk claiming more than the file holds.
    let mut huge = good.clone();
    huge[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(err(&huge).contains("truncated"));
    // No sprite chunk.
    let parts = chunks(&good);
    assert!(err(&rebuild(&parts[..2])).contains("no sprite data"));
    // A sprite chunk that is not a sprite.
    let junk = rebuild(&[(*b"SPRT", b"not an aseprite file".to_vec())]);
    assert!(err(&junk).contains("sprite data"));
    // Every prefix and a sweep of single-byte corruptions: errors or (for unchecked bytes) success, never a panic.
    for n in 0..good.len() - 12 {
        assert!(pxmt::read(&good[..n]).is_err(), "prefix of {n} bytes");
    }
    for i in 0..good.len() {
        for flip in [0x01u8, 0x80, 0xff] {
            let mut b = good.clone();
            b[i] ^= flip;
            let _ = pxmt::read(&b);
            let _ = pxmt::read_meta(Cursor::new(&b));
            let _ = pxmt::read_thumbnail(Cursor::new(&b));
        }
    }
    // Pseudo-random garbage behind a valid header.
    let mut x = 0x2545F4914F6CDD1Du64;
    for round in 0..50 {
        let mut b = b"PXMT\r\n\x1a\n\x01\x00".to_vec();
        for _ in 0..(round * 37 % 600) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            b.push(x as u8);
        }
        let _ = pxmt::read(&b);
        let _ = pxmt::read_meta(Cursor::new(&b));
        let _ = pxmt::read_thumbnail(Cursor::new(&b));
    }
    // A thumbnail that is not a PNG, or claims an absurd size, is rejected without decoding.
    let bad_png = rebuild(&[(*b"THMB", b"GIF89a...".to_vec())]);
    assert!(pxmt::read_thumbnail(Cursor::new(&bad_png)).is_err());
    let mut ihdr = import_png_header(60000, 60000);
    ihdr.extend([0; 32]);
    assert!(pxmt::read_thumbnail(Cursor::new(rebuild(&[(*b"THMB", ihdr)]))).is_err());
    // Damaged real files: truncated copies of every fixture's .pxmt never panic.
    for path in fixtures().iter().step_by(7) {
        let bytes = pxmt::write(&ase::read(&std::fs::read(path).unwrap()).unwrap()).unwrap();
        for cut in [0, 5, 9, 10, 17, 18, bytes.len() / 3, bytes.len() / 2, bytes.len() - 13] {
            assert!(pxmt::read(&bytes[..cut]).is_err());
        }
    }
}

fn import_png_header(w: u32, h: u32) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    v.extend(w.to_be_bytes());
    v.extend(h.to_be_bytes());
    v.extend([8, 6, 0, 0, 0]);
    let _ = import::decode_png;
    v
}

#[test]
fn pixmith_only_data_survives() {
    // Tweens, terrains and Godot export settings live in user data, which the sprite payload carries.
    let mut s = sample();
    s.layers[0].user_data.text = Some("my note\n--\npixmith.autotile.v1:{\"terrains\":[1,2,3]}".into());
    s.layers[1].user_data.text = Some("pixmith.tween.v1:{\"keys\":[[0,1.5],[3,2.5]]}".into());
    s.layers[1].user_data.color = Some([1, 2, 3, 4]);
    let json = r#"{"v":1,"root":"/game","tres":"/game/a.tres","padding":2}"#;
    settings::store(&mut s, json);
    s.extra.sprite.color = Some([9, 9, 9, 255]);
    let back = pxmt::read(&pxmt::write(&s).unwrap()).unwrap();
    assert_same(&s, &back, "user data");
    assert_eq!(settings::load(&back).as_deref(), Some(json));
    assert_eq!(back.layers[1].user_data.text.as_deref(), s.layers[1].user_data.text.as_deref());
}

#[test]
fn c_entry_points_pick_the_format_themselves() {
    use pixmith_core::ffi::document::*;
    use pixmith_core::ffi::{pixmith_last_error, pixmith_string_free};
    let dir = temp_dir("ffi");
    let s = sample();
    unsafe {
        let doc = Box::into_raw(Box::new(Editor::new(s.clone())));
        // Save: .pxmt (any case) writes .pxmt, anything else .aseprite.
        for (name, is_pxmt) in
            [("a.pxmt", true), ("b.PXMT", true), ("c.aseprite", false), ("d.ase", false), ("e", false)]
        {
            let p = dir.join(name);
            assert!(pixmith_save(doc, cstr(&p).as_ptr()), "{name}");
            let bytes = std::fs::read(&p).unwrap();
            assert_eq!(pxmt::is_pxmt(&bytes), is_pxmt, "{name}");
            // Open detects by magic, not by extension.
            let renamed = dir.join(format!("renamed-{name}.dat"));
            std::fs::copy(&p, &renamed).unwrap();
            for q in [&p, &renamed] {
                let back = pixmith_open(cstr(q).as_ptr());
                assert!(!back.is_null(), "{q:?}");
                assert_same(&s, &(*back).sprite, name);
                pixmith_free(back);
            }
        }
        // Export As Aseprite ignores the extension.
        let p = dir.join("export.pxmt");
        assert!(pixmith_save_aseprite(doc, cstr(&p).as_ptr()));
        assert!(!pxmt::is_pxmt(&std::fs::read(&p).unwrap()));
        assert_same(&s, &ase::read(&std::fs::read(&p).unwrap()).unwrap(), "aseprite export");
        // No temporary files are left behind.
        assert!(std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".pixmith-tmp")));

        // Thumbnails.
        let mut out = vec![0u8; 20 * 20 * 4];
        let (mut w, mut h) = (0u32, 0u32);
        for name in ["a.pxmt", "c.aseprite", "renamed-a.pxmt.dat"] {
            assert!(
                pixmith_file_thumbnail(
                    cstr(&dir.join(name)).as_ptr(),
                    20,
                    20,
                    out.as_mut_ptr(),
                    out.len(),
                    &mut w,
                    &mut h
                ),
                "{name}"
            );
            assert_eq!((w, h), (20, 12), "{name}");
            assert_eq!(out[..4], s.render(0)[..4]);
        }
        // Bad arguments: errors, no crash.
        assert!(!pixmith_file_thumbnail(std::ptr::null(), 20, 20, out.as_mut_ptr(), out.len(), &mut w, &mut h));
        assert!(!pixmith_file_thumbnail(
            cstr(&dir.join("a.pxmt")).as_ptr(),
            20,
            20,
            std::ptr::null_mut(),
            0,
            &mut w,
            &mut h
        ));
        assert!(!pixmith_file_thumbnail(
            cstr(&dir.join("a.pxmt")).as_ptr(),
            20,
            20,
            out.as_mut_ptr(),
            8,
            &mut w,
            &mut h
        ));
        assert!(!pixmith_file_thumbnail(
            cstr(&dir.join("a.pxmt")).as_ptr(),
            0,
            20,
            out.as_mut_ptr(),
            out.len(),
            &mut w,
            &mut h
        ));
        assert!(!pixmith_file_thumbnail(
            cstr(&dir.join("nope.pxmt")).as_ptr(),
            20,
            20,
            out.as_mut_ptr(),
            out.len(),
            &mut w,
            &mut h
        ));
        assert!(pixmith_file_thumbnail(
            cstr(&dir.join("a.pxmt")).as_ptr(),
            20,
            20,
            out.as_mut_ptr(),
            out.len(),
            std::ptr::null_mut(),
            std::ptr::null_mut()
        ));
        assert!(!pixmith_save_aseprite(std::ptr::null(), cstr(&p).as_ptr()));
        assert!(!pixmith_save_aseprite(doc, std::ptr::null()));
        assert!(!pixmith_save(doc, std::ptr::null()));
        assert!(!pixmith_save(std::ptr::null(), cstr(&p).as_ptr()));
        // A damaged .pxmt reports a clear error and opens as null.
        let mut bad = std::fs::read(dir.join("a.pxmt")).unwrap();
        let at = bad.windows(4).position(|w| w == b"SPRT").unwrap() + 30;
        bad[at] ^= 0x10;
        let bad_path = dir.join("bad.pxmt");
        std::fs::write(&bad_path, &bad).unwrap();
        assert!(pixmith_open(cstr(&bad_path).as_ptr()).is_null());
        let msg = CStr::from_ptr(pixmith_last_error()).to_string_lossy().into_owned();
        assert!(msg.contains("bad.pxmt") && msg.contains("checksum"), "{msg}");
        // A failed save leaves the previous file alone.
        let before = std::fs::read(dir.join("a.pxmt")).unwrap();
        assert!(!pixmith_save(doc, cstr(&dir.join("no-such-dir").join("x.pxmt")).as_ptr()));
        assert_eq!(std::fs::read(dir.join("a.pxmt")).unwrap(), before);
        pixmith_free(doc);
        pixmith_string_free(std::ptr::null_mut());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_over_a_pxmt_keeps_its_creation_time() {
    let dir = temp_dir("created");
    let path = dir.join("a.pxmt");
    let p = path.to_str().unwrap();
    let mut s = sample();
    pxmt::save_file(p, &s).unwrap();
    let first = pxmt::read_meta_file(&path).unwrap().unwrap();
    // Pretend the file is old.
    let old = pxmt::Meta { created: 1_000_000, ..first.clone() };
    std::fs::write(&path, pxmt::write_with(&s, Some(&old)).unwrap()).unwrap();
    s.draw_line(0, 0, 0, 0, 1, 1, [1, 2, 3, 255], 1);
    pxmt::save_file(p, &s).unwrap();
    let second = pxmt::read_meta_file(&path).unwrap().unwrap();
    assert_eq!(second.created, 1_000_000);
    assert!(second.modified >= first.modified);
    assert_same(&s, &pxmt::open_file(&path).unwrap(), "resaved");
    let _ = std::fs::remove_dir_all(&dir);
}

// MARK: CLI

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pixmith-cli")).args(args).output().unwrap()
}

#[test]
fn cli_reads_pxmt_everywhere_and_converts() {
    let dir = temp_dir("cli");
    let s = sample();
    let ase_path = dir.join("hero.aseprite");
    std::fs::write(&ase_path, ase::write(&s)).unwrap();
    let (a, pxmt_path, back) = (ase_path.to_str().unwrap(), dir.join("hero.pxmt"), dir.join("back.ase"));
    let p = pxmt_path.to_str().unwrap();

    let out = cli(&["convert", a, p]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(pxmt::is_pxmt(&std::fs::read(&pxmt_path).unwrap()));
    let out = cli(&["convert", p, back.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&back).unwrap();
    assert!(!pxmt::is_pxmt(&bytes));
    assert_same(&s, &ase::read(&bytes).unwrap(), "pxmt -> aseprite");
    assert_same(&s, &pxmt::open_file(&pxmt_path).unwrap(), "aseprite -> pxmt");

    // info and export accept .pxmt, with the same output as for the .aseprite.
    let info_a = cli(&["info", "--json", a]);
    let info_p = cli(&["info", "--json", p]);
    assert!(info_p.status.success());
    assert_eq!(
        String::from_utf8_lossy(&info_p.stdout).replace("hero.pxmt", "hero.aseprite"),
        String::from_utf8_lossy(&info_a.stdout)
    );
    let png = dir.join("out.png");
    let out = cli(&["export", p, "--png", png.to_str().unwrap(), "--frame", "2", "--scale", "2"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (w, h, px) = import::decode_png(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((w, h), (20, 12));
    assert_eq!(px.len(), w * h * 4);

    // Misuse: bad output extension, same file, missing input, wrong argument count, a non-document.
    for args in [
        vec!["convert", a, dir.join("x.png").to_str().unwrap()],
        vec!["convert", a, a],
        vec!["convert", dir.join("missing.pxmt").to_str().unwrap(), dir.join("y.pxmt").to_str().unwrap()],
    ] {
        let out = cli(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
    }
    assert_eq!(cli(&["convert", a]).status.code(), Some(2));
    assert_eq!(cli(&["convert"]).status.code(), Some(2));
    assert_eq!(cli(&["convert", "a", "b", "c"]).status.code(), Some(2));
    std::fs::write(dir.join("junk.pxmt"), b"PXMT\r\n\x1a\nzzzz").unwrap();
    let out = cli(&["convert", dir.join("junk.pxmt").to_str().unwrap(), dir.join("z.aseprite").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!dir.join("z.aseprite").exists());
    assert!(String::from_utf8_lossy(&cli(&["--help"]).stdout).contains("convert <in> <out>"));
    let _ = std::fs::remove_dir_all(&dir);
}
