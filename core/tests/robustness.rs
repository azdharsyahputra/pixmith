//! Robustness tests: corrupt and hostile `.aseprite` input must never panic or allocate wildly
//! (the release build aborts on panic).

mod common;

use common::*;
use flate2::{write::ZlibEncoder, Compression};
use pixmith_core::formats::ase;
use std::io::Write;

// ---- tests: valid files, truncation, mutation --------------------------------------------

#[test]
fn sample_files_round_trip() {
    let _g = serial();
    for (name, bytes) in samples() {
        let s = ase::read(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        check_sprite(&s);
    }
}

#[test]
fn truncated_at_every_offset() {
    let _g = serial();
    let mut failures = Vec::new();
    for (name, bytes) in samples() {
        for n in 0..bytes.len() {
            read_and_check(&bytes[..n], &format!("{name} truncated to {n}"), &mut failures);
        }
    }
    assert_modest_allocations("truncation");
    report(failures);
}

#[test]
fn random_byte_flips() {
    let _g = serial();
    let mut rng = Rng(0x2545F4914F6CDD1D);
    let mut failures = Vec::new();
    for (name, bytes) in samples() {
        for i in 0..3000 {
            let mut b = bytes.clone();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(b.len());
                b[at] = match rng.below(4) {
                    0 => 0x00,
                    1 => 0xFF,
                    _ => rng.next() as u8,
                };
            }
            read_and_check(&b, &format!("{name} flip #{i}"), &mut failures);
        }
    }
    assert_modest_allocations("byte flips");
    report(failures);
}

#[test]
fn extreme_field_values() {
    let _g = serial();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut failures = Vec::new();
    let interesting: [&[u8]; 6] =
        [&[0xFF, 0xFF], &[0, 0], &[0xFF, 0x7F], &[0, 0x80], &[0xFF, 0xFF, 0xFF, 0xFF], &[0xFF, 0xFF, 0xFF, 0x7F]];
    for (name, bytes) in samples() {
        for i in 0..800 {
            let mut b = bytes.clone();
            for _ in 0..1 + rng.below(2) {
                let v = interesting[rng.below(interesting.len())];
                // Bias toward the header and the first chunks, where the sizes live.
                let at = if rng.below(2) == 0 { rng.below(b.len().min(200)) } else { rng.below(b.len()) };
                for (k, &x) in v.iter().enumerate() {
                    if let Some(slot) = b.get_mut(at + k) {
                        *slot = x;
                    }
                }
            }
            read_and_check(&b, &format!("{name} extreme #{i}"), &mut failures);
        }
    }
    assert_modest_allocations("extreme values");
    report(failures);
}

// ---- tests: crafted hostile files --------------------------------------------------------

fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes());
}

fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
}

fn header(frames: u16, w: u16, h: u16, depth: u16) -> Vec<u8> {
    let mut b = Vec::new();
    put32(&mut b, 0);
    put16(&mut b, 0xA5E0);
    put16(&mut b, frames);
    put16(&mut b, w);
    put16(&mut b, h);
    put16(&mut b, depth);
    put32(&mut b, 1);
    b.resize(128, 0);
    b
}

fn chunk(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut c = Vec::new();
    put32(&mut c, body.len() as u32 + 6);
    put16(&mut c, kind);
    c.extend(body);
    c
}

fn frame(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut f = Vec::new();
    put32(&mut f, 16 + chunks.iter().map(Vec::len).sum::<usize>() as u32);
    put16(&mut f, 0xF1FA);
    put16(&mut f, chunks.len() as u16);
    put16(&mut f, 100);
    f.extend([0; 2]);
    put32(&mut f, chunks.len() as u32);
    for c in chunks {
        f.extend(c);
    }
    f
}

fn layer_chunk(name: &str) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 3); // visible, editable
    put16(&mut b, 0);
    put16(&mut b, 0);
    put32(&mut b, 0);
    put16(&mut b, 0);
    b.push(255);
    b.extend([0; 3]);
    put16(&mut b, name.len() as u16);
    b.extend(name.as_bytes());
    chunk(0x2004, &b)
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
    z.write_all(data).unwrap();
    z.finish().unwrap()
}

/// A compressed image cel chunk declaring a `w` x `h` image whose zlib payload is `payload`.
fn cel_chunk(layer: u16, w: u16, h: u16, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, layer);
    put16(&mut b, 0);
    put16(&mut b, 0);
    b.push(255);
    put16(&mut b, 2);
    put16(&mut b, 0);
    b.extend([0; 5]);
    put16(&mut b, w);
    put16(&mut b, h);
    b.extend(payload);
    chunk(0x2005, &b)
}

fn file(frames: u16, w: u16, h: u16, depth: u16, body: Vec<Vec<u8>>) -> Vec<u8> {
    let mut f = header(frames, w, h, depth);
    for fr in body {
        f.extend(fr);
    }
    f
}

#[test]
fn huge_canvas_is_rejected_cheaply() {
    let _g = serial();
    for (w, h) in [(65535, 65535), (65535, 2000), (20000, 20000), (9000, 9000)] {
        let f = file(1, w, h, 32, vec![frame(&[layer_chunk("a"), cel_chunk(0, 1, 1, &zlib(&[1, 2, 3, 4]))])]);
        assert!(ase::read(&f).is_err(), "{w}x{h}");
    }
    assert_modest_allocations("huge canvas");
}

#[test]
fn canvas_at_the_limit_is_fine_but_lazy() {
    let _g = serial();
    // 8192 x 8192 with an empty cel list: no cel buffers get allocated.
    let f = file(1, 8192, 8192, 32, vec![frame(&[layer_chunk("a")])]);
    let s = ase::read(&f).unwrap();
    assert!(s.cels[0][0].is_none());
    assert_modest_allocations("lazy canvas");
}

#[test]
fn huge_declared_cel_sizes_are_rejected() {
    let _g = serial();
    let tiny_stream = zlib(&[0; 16]);
    for depth in [32, 16, 8] {
        // Compressed and raw cels claiming 65535 x 65535 pixels.
        let f = file(1, 8, 8, depth, vec![frame(&[layer_chunk("a"), cel_chunk(0, 65535, 65535, &tiny_stream)])]);
        assert!(ase::read(&f).is_err(), "compressed, depth {depth}");
        let mut raw = Vec::new();
        for v in [0u16, 0, 0] {
            put16(&mut raw, v);
        }
        raw.push(255);
        put16(&mut raw, 0);
        raw.extend([0; 7]);
        put16(&mut raw, 65535);
        put16(&mut raw, 65535);
        raw.extend([0; 64]);
        let f = file(1, 8, 8, depth, vec![frame(&[layer_chunk("a"), chunk(0x2005, &raw)])]);
        assert!(ase::read(&f).is_err(), "raw, depth {depth}");
    }
    assert_modest_allocations("huge cel");
}

#[test]
fn zlib_bombs_are_bounded() {
    let _g = serial();
    // A real stream that inflates to 64 MiB but is only ~64 KiB, declared exactly right.
    let bomb = zlib(&vec![0u8; 64 << 20]);
    assert!(bomb.len() < 1 << 20);
    let one = cel_chunk(0, 4096, 4096, &bomb);
    let f = file(1, 8, 8, 32, vec![frame(&[layer_chunk("a"), one.clone()])]);
    assert!(ase::read(&f).is_ok());
    // The same stream declared as 1 GiB: it is cut off at the declared size, and the declared
    // size alone is over budget.
    let f = file(1, 8, 8, 32, vec![frame(&[layer_chunk("a"), cel_chunk(0, 16384, 16384, &bomb)])]);
    assert!(ase::read(&f).is_err());
    // Many in-budget bombs add up past the decoded-data budget.
    let mut chunks = vec![layer_chunk("a")];
    chunks.extend((0..12).map(|_| one.clone()));
    let f = file(1, 8, 8, 32, vec![frame(&chunks)]);
    assert!(ase::read(&f).is_err());
    assert_modest_allocations("zlib bomb");
}

#[test]
fn cel_memory_budget_is_enforced() {
    let _g = serial();
    // Each cel is tiny on disk but needs a canvas-sized buffer: thousands of them must be refused.
    let mut chunks = vec![layer_chunk("a")];
    let cel = cel_chunk(0, 1, 1, &zlib(&[1, 2, 3, 4]));
    let frames: Vec<Vec<u8>> = (0..64)
        .map(|f| {
            if f == 0 {
                chunks.push(cel.clone());
                frame(&chunks)
            } else {
                frame(std::slice::from_ref(&cel))
            }
        })
        .collect();
    // 64 cels of a 8192 x 8192 canvas = 16 GiB.
    let f = file(64, 8192, 8192, 32, frames);
    assert!(ase::read(&f).is_err());
    assert_modest_allocations("cel budget");
}

#[test]
fn too_many_layers_or_slots() {
    let _g = serial();
    let layers: Vec<Vec<u8>> = (0..5000).map(|_| layer_chunk("l")).collect();
    assert!(ase::read(&file(1, 4, 4, 32, vec![frame(&layers)])).is_err());
    // 65535 frames (each 16 bytes) times 100 layers is far past the cel-slot limit.
    let layers: Vec<Vec<u8>> = (0..100).map(|_| layer_chunk("l")).collect();
    let mut frames = vec![frame(&layers)];
    frames.extend((1..65535).map(|_| frame(&[])));
    assert!(ase::read(&file(65535, 4, 4, 32, frames)).is_err());
    assert_modest_allocations("slots");
}

#[test]
fn bad_sizes_and_counts_do_not_panic() {
    let _g = serial();
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    // Frame count far beyond the data present.
    cases.push(("frame count", file(65535, 4, 4, 32, vec![frame(&[layer_chunk("a")])])));
    // Palette with absurd size / index range.
    let mut pal = Vec::new();
    for v in [u32::MAX, 0, u32::MAX] {
        put32(&mut pal, v);
    }
    pal.extend([0; 8]);
    cases.push(("palette size", file(1, 4, 4, 32, vec![frame(&[chunk(0x2019, &pal)])])));
    let mut pal = Vec::new();
    for v in [256u32, 0, u32::MAX] {
        put32(&mut pal, v);
    }
    pal.extend([0; 8]);
    pal.extend([0; 60]);
    cases.push(("palette range", file(1, 4, 4, 8, vec![frame(&[chunk(0x2019, &pal)])])));
    // Chunk and frame sizes that lie.
    for size in [0u32, 5, 6, 7, u32::MAX, u32::MAX - 5, 0x8000_0000] {
        let mut c = Vec::new();
        put32(&mut c, size);
        put16(&mut c, 0x2004);
        c.extend([0; 40]);
        cases.push(("chunk size", file(1, 4, 4, 32, vec![frame(&[c])])));
        let mut f = frame(&[layer_chunk("a")]);
        f[..4].copy_from_slice(&size.to_le_bytes());
        cases.push(("frame size", file(1, 4, 4, 32, vec![f.clone(), f])));
    }
    // Tags: count with no data; name longer than the chunk.
    let mut tags = Vec::new();
    put16(&mut tags, 65535);
    tags.extend([0; 8]);
    cases.push(("tag count", file(1, 4, 4, 32, vec![frame(&[chunk(0x2018, &tags)])])));
    // Layer name length beyond the chunk, tilemap layer without tileset id.
    let mut l = layer_chunk("abc");
    let n = l.len();
    l[n - 5] = 0xFF;
    l[n - 4] = 0xFF;
    cases.push(("layer name", file(1, 4, 4, 32, vec![frame(&[l])])));
    // Cel pointing at layers/frames that don't exist, and a link to itself.
    for (layer, src) in [(7u16, 0u16), (0xFFFF, 0xFFFF), (0, 0), (0, 5)] {
        let mut b = Vec::new();
        put16(&mut b, layer);
        b.extend([0; 4]);
        b.push(255);
        put16(&mut b, 1);
        put16(&mut b, 0);
        b.extend([0; 5]);
        put16(&mut b, src);
        cases.push((
            "link",
            file(2, 4, 4, 32, vec![frame(&[layer_chunk("a"), chunk(0x2005, &b)]), frame(&[chunk(0x2005, &b)])]),
        ));
    }
    // Cel offsets far outside the canvas.
    for (x, y) in [(i16::MIN, i16::MIN), (i16::MAX, i16::MAX), (-1, i16::MAX), (i16::MAX, 0)] {
        let mut b = Vec::new();
        put16(&mut b, 0);
        put16(&mut b, x as u16);
        put16(&mut b, y as u16);
        b.push(255);
        put16(&mut b, 2);
        put16(&mut b, 0);
        b.extend([0; 5]);
        put16(&mut b, 2);
        put16(&mut b, 2);
        b.extend(zlib(&[200; 16]));
        cases.push(("cel offset", file(1, 4, 4, 32, vec![frame(&[layer_chunk("a"), chunk(0x2005, &b)])])));
    }
    let mut failures = Vec::new();
    for (name, f) in &cases {
        read_and_check(f, name, &mut failures);
    }
    assert_modest_allocations("bad sizes");
    report(failures);
}

fn tileset_chunk(count: u32, tw: u16, th: u16, flags: u32, pixels: Option<&[u8]>) -> Vec<u8> {
    let mut b = Vec::new();
    put32(&mut b, 0);
    put32(&mut b, flags);
    put32(&mut b, count);
    put16(&mut b, tw);
    put16(&mut b, th);
    put16(&mut b, 1);
    b.extend([0; 14]);
    put16(&mut b, 1);
    b.push(b't');
    if let Some(p) = pixels {
        let z = zlib(p);
        put32(&mut b, z.len() as u32);
        b.extend(z);
    }
    chunk(0x2023, &b)
}

fn tilemap_layer_chunk() -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 3);
    put16(&mut b, 2);
    put16(&mut b, 0);
    put32(&mut b, 0);
    put16(&mut b, 0);
    b.push(255);
    b.extend([0; 3]);
    put16(&mut b, 1);
    b.push(b'm');
    put32(&mut b, 0);
    chunk(0x2004, &b)
}

fn tilemap_cel_chunk(w: u16, h: u16, bits: u16, tiles: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 0);
    put16(&mut b, 0);
    put16(&mut b, 0);
    b.push(255);
    put16(&mut b, 3);
    put16(&mut b, 0);
    b.extend([0; 5]);
    put16(&mut b, w);
    put16(&mut b, h);
    put16(&mut b, bits);
    for m in [0x1FFF_FFFFu32, 0x8000_0000, 0x4000_0000, 0x2000_0000] {
        put32(&mut b, m);
    }
    b.extend([0; 10]);
    b.extend(zlib(tiles));
    chunk(0x2005, &b)
}

#[test]
fn hostile_tilesets_and_tilemaps() {
    let _g = serial();
    // Huge tiles x huge count, both embedded (with little data) and not embedded at all.
    for flags in [0u32, 2, 4, 6, 1] {
        let f = file(
            1,
            8,
            8,
            32,
            vec![frame(&[tileset_chunk(1 << 20, 1024, 1024, flags, Some(&[0; 64])), tilemap_layer_chunk()])],
        );
        assert!(ase::read(&f).is_err(), "flags {flags}");
    }
    // Many moderately large tilesets add up.
    let sets: Vec<Vec<u8>> = (0..64).map(|_| tileset_chunk(4096, 128, 128, 0, None)).collect();
    assert!(ase::read(&file(1, 8, 8, 32, vec![frame(&sets)])).is_err());
    // Tilemap cel with an enormous grid, wrong bit depth, and tile ids beyond the tileset.
    let ts = tileset_chunk(2, 4, 4, 2 | 4, Some(&[255; 4 * 4 * 4 * 2]));
    for (w, h, bits) in [(65535u16, 65535u16, 32u16), (4096, 4096, 32), (2, 2, 7), (0, 0, 32), (65535, 1, 16)] {
        let f = file(
            1,
            8,
            8,
            32,
            vec![frame(&[ts.clone(), tilemap_layer_chunk(), tilemap_cel_chunk(w, h, bits, &[0xFF; 16])])],
        );
        let mut failures = Vec::new();
        read_and_check(&f, &format!("tilemap cel {w}x{h} {bits}bit"), &mut failures);
        report(failures);
    }
    let f = file(1, 8, 8, 32, vec![frame(&[ts, tilemap_layer_chunk(), tilemap_cel_chunk(2, 2, 32, &[0xFF; 16])])]);
    let mut failures = Vec::new();
    read_and_check(&f, "tile ids beyond tileset", &mut failures);
    report(failures);
    assert_modest_allocations("tilesets");
}

fn layer_chunk_ex(name: &str, kind: u16, level: u16, blend: u16, opacity: u8) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 3);
    put16(&mut b, kind);
    put16(&mut b, level);
    put32(&mut b, 0);
    put16(&mut b, blend);
    b.push(opacity);
    b.extend([0; 3]);
    put16(&mut b, name.len() as u16);
    b.extend(name.as_bytes());
    chunk(0x2004, &b)
}

#[test]
fn nested_groups_in_files_are_normalized() {
    let _g = serial();
    // 4000 groups nested one inside the other, then an image layer with a pixel at the bottom.
    let mut chunks: Vec<Vec<u8>> =
        (0..4000u16).map(|i| layer_chunk_ex("g", 1, i, if i % 3 == 0 { 1 } else { 0 }, 200)).collect();
    chunks.push(layer_chunk_ex("leaf", 0, 4000, 0, 255));
    let leaf = chunks.len() as u16 - 1;
    chunks.push(cel_chunk(leaf, 1, 1, &zlib(&[200, 100, 50, 255])));
    let f = file(1, 4, 4, 32, vec![frame(&chunks)]);
    let s = ase::read(&f).unwrap();
    assert_eq!(s.layers.len(), 4001);
    let max = s.layers.iter().map(|l| l.child_level).max().unwrap();
    assert_eq!(max, pixmith_core::document::layers::MAX_GROUP_DEPTH);
    // The result is a well-formed tree: no level jumps by more than one, only groups have children.
    for w in s.layers.windows(2) {
        assert!(w[1].child_level <= w[0].child_level + w[0].is_group() as u16);
    }
    assert_eq!(s.render(0).len(), 4 * 4 * 4);
    check_sprite(&s);
    assert_modest_allocations("nested groups");
}

#[test]
fn ill_formed_levels_are_repaired() {
    let _g = serial();
    // Levels 0, 5, 9 under a plain layer, then a group with a child that skips a level.
    let chunks = vec![
        layer_chunk_ex("a", 0, 0, 0, 255),
        layer_chunk_ex("b", 0, 5, 0, 255),
        layer_chunk_ex("c", 0, 9, 0, 255),
        layer_chunk_ex("g", 1, 3, 0, 255),
        layer_chunk_ex("d", 0, 7, 0, 255),
        layer_chunk_ex("e", 0, 1, 0, 255),
    ];
    let s = ase::read(&file(1, 2, 2, 32, vec![frame(&chunks)])).unwrap();
    let levels: Vec<u16> = s.layers.iter().map(|l| l.child_level).collect();
    assert_eq!(levels, vec![0, 0, 0, 0, 1, 1]);
}

#[test]
fn rendering_extremely_nested_groups_is_safe() {
    let _g = serial();
    // Built in memory, bypassing the loader's normalization: 6000 nested groups with blend
    // modes and opacities. Rendering must neither overflow the stack nor allocate a canvas per
    // level.
    let depth = 6000usize;
    let mut s = pixmith_core::document::Sprite::new(8, 8);
    s.layers.clear();
    for i in 0..depth {
        let mut l = pixmith_core::document::Layer::new("g");
        l.kind = 1;
        l.child_level = i as u16;
        l.opacity = if i % 1500 == 1 { 128 } else { 255 };
        l.blend_mode = [0u16, 1, 3, 10][i % 4];
        s.layers.push(l);
    }
    let mut leaf = pixmith_core::document::Layer::new("leaf");
    leaf.child_level = depth as u16;
    s.layers.push(leaf);
    let mut px = vec![0u8; 8 * 8 * 4];
    px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.copy_from_slice(&[200, 100, 50, 255]));
    s.cels = vec![vec![None; depth + 1]];
    s.cels[0][depth] =
        Some(std::sync::Arc::new(pixmith_core::document::Cel { opacity: 255, pixels: px, ..Default::default() }));
    let handle = std::thread::Builder::new().stack_size(1 << 20).spawn(move || s.render(0)).unwrap();
    let out = handle.join().expect("render must not overflow the stack");
    assert_eq!(out.len(), 8 * 8 * 4);
    assert!(out[3] > 0, "the leaf is visible");
    assert_modest_allocations("nested group render");
}
