//! Autodesk Animator FLI and FLC animations (8 bit, palette based).
//!
//! Reading follows the public file description: the 128 byte header, frames of chunks, and the chunk types
//! COLOR_256, COLOR_64, DELTA_FLC (word oriented), DELTA_FLI (byte oriented), BLACK, BYTE_RUN and COPY; unknown
//! chunks are skipped. Writing makes an FLC with one 256 color palette, a BYTE_RUN (or COPY) first frame, DELTA_FLC
//! frames after it and the ring frame that makes the animation loop. FLC has no alpha: frames are flattened onto
//! the background color of the options, and a palette of more than 256 colors is reduced by median cut.

use super::{check_dims, check_frame_budget, flatten, indexed, le16, le32, Decoded, ImageOptions};

const FRAME_MAGIC: u16 = 0xF1FA;

const COLOR_256: u16 = 4;
const DELTA_FLC: u16 = 7;
const COLOR_64: u16 = 11;
const DELTA_FLI: u16 = 12;
const BLACK: u16 = 13;
const BYTE_RUN: u16 = 15;
const COPY: u16 = 16;

pub fn decode(b: &[u8]) -> Result<Decoded, String> {
    if b.len() < 128 {
        return Err("not a FLI/FLC file".into());
    }
    let magic = le16(b, 4).unwrap_or(0);
    let is_fli = match magic {
        0xAF11 => true,
        0xAF12 => false,
        _ => return Err("not a FLI/FLC file".into()),
    };
    let declared = le16(b, 6).unwrap_or(0) as usize;
    let (mut w, mut h) = (le16(b, 8).unwrap_or(0) as usize, le16(b, 10).unwrap_or(0) as usize);
    if is_fli && (w == 0 || h == 0) {
        (w, h) = (320, 200);
    }
    check_dims(w, h)?;
    if le16(b, 12).unwrap_or(8) != 8 {
        return Err("only 8 bit FLI/FLC animations are supported".into());
    }
    let speed_ms =
        if is_fli { (le16(b, 16).unwrap_or(0) as u32 * 1000 / 70).max(14) } else { le32(b, 16).unwrap_or(0).max(1) }
            .min(u16::MAX as u32) as u16;

    let first = le32(b, 80).unwrap_or(0) as usize;
    let mut pos = if !is_fli && first >= 128 && first < b.len() { first } else { 128 };
    let wanted = if declared == 0 { u16::MAX as usize } else { declared };

    let mut indices = vec![0u8; w * h];
    let mut palette = [[0u8; 3]; 256];
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut durations = Vec::new();
    while frames.len() < wanted && pos + 16 <= b.len() {
        let size = le32(b, pos).unwrap_or(0) as usize;
        let kind = le16(b, pos + 4).unwrap_or(0);
        if size < 16 {
            break;
        }
        let end = pos.saturating_add(size).min(b.len());
        if kind == FRAME_MAGIC {
            check_frame_budget(frames.len(), w, h)?;
            let chunks = le16(b, pos + 6).unwrap_or(0) as usize;
            let delay = le16(b, pos + 8).unwrap_or(0);
            let mut at = pos + 16;
            for _ in 0..chunks {
                let (Some(csize), Some(ctype)) = (le32(b, at), le16(b, at + 4)) else { break };
                let csize = csize as usize;
                if csize < 6 || at + csize > end {
                    break;
                }
                apply_chunk(ctype, &b[at + 6..at + csize], w, h, &mut indices, &mut palette, is_fli)?;
                at += csize;
            }
            let mut rgba = Vec::with_capacity(w * h * 4);
            for &i in &indices {
                let c = palette[i as usize];
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
            frames.push(rgba);
            durations.push(if delay > 0 { delay } else { speed_ms });
        }
        if end >= b.len() && size > end - pos {
            break; // the last frame was cut off; it was applied as far as it goes
        }
        pos += size;
    }
    if frames.is_empty() {
        return Err("the FLI/FLC has no frames".into());
    }
    let mut last_palette: Vec<[u8; 4]> = palette.iter().map(|c| [c[0], c[1], c[2], 255]).collect();
    last_palette.truncate(256);
    Ok(Decoded { width: w, height: h, frames, durations, palette: Some(last_palette) })
}

fn apply_chunk(
    kind: u16,
    d: &[u8],
    w: usize,
    h: usize,
    idx: &mut [u8],
    pal: &mut [[u8; 3]; 256],
    is_fli: bool,
) -> Result<(), String> {
    let bad = || "the FLI/FLC is damaged".to_string();
    match kind {
        COLOR_256 | COLOR_64 => {
            let packets = le16(d, 0).ok_or_else(bad)? as usize;
            let (mut p, mut at) = (2, 0usize);
            // Old files store 6 bit values even in COLOR_256 chunks.
            let six = kind == COLOR_64 || (is_fli && d.get(2..).is_some_and(|v| v.iter().all(|&x| x < 64)));
            for _ in 0..packets {
                let (Some(&skip), Some(&count)) = (d.get(p), d.get(p + 1)) else { return Err(bad()) };
                p += 2;
                at += skip as usize;
                let count = if count == 0 { 256 } else { count as usize };
                for k in 0..count {
                    let c = d.get(p..p + 3).ok_or_else(bad)?;
                    p += 3;
                    if let Some(e) = pal.get_mut(at + k) {
                        *e = if six {
                            [c[0] << 2 | c[0] >> 4, c[1] << 2 | c[1] >> 4, c[2] << 2 | c[2] >> 4]
                        } else {
                            [c[0], c[1], c[2]]
                        };
                    }
                }
                at += count;
            }
        }
        BLACK => idx.fill(0),
        COPY => {
            let src = d.get(..w * h).ok_or_else(bad)?;
            idx.copy_from_slice(src);
        }
        BYTE_RUN => {
            let mut p = 0;
            for y in 0..h {
                p += 1; // packet count: ignored, the line ends at the width
                let mut x = 0;
                while x < w {
                    let c = *d.get(p).ok_or_else(bad)? as i8;
                    p += 1;
                    if c >= 0 {
                        let v = *d.get(p).ok_or_else(bad)?;
                        p += 1;
                        let n = (c as usize).min(w - x);
                        if n == 0 {
                            break;
                        }
                        idx[y * w + x..y * w + x + n].fill(v);
                        x += n;
                    } else {
                        let n = (-(c as i32)) as usize;
                        let src = d.get(p..p + n).ok_or_else(bad)?;
                        p += n;
                        let m = n.min(w - x);
                        idx[y * w + x..y * w + x + m].copy_from_slice(&src[..m]);
                        x += n;
                    }
                }
            }
        }
        DELTA_FLI => {
            let (start, lines) = (le16(d, 0).ok_or_else(bad)? as usize, le16(d, 2).ok_or_else(bad)? as usize);
            let mut p = 4;
            for y in start..start.saturating_add(lines) {
                let packets = *d.get(p).ok_or_else(bad)?;
                p += 1;
                let mut x = 0usize;
                for _ in 0..packets {
                    let (Some(&skip), Some(&count)) = (d.get(p), d.get(p + 1)) else { return Err(bad()) };
                    p += 2;
                    x += skip as usize;
                    let count = count as i8;
                    if count > 0 {
                        let n = count as usize;
                        let src = d.get(p..p + n).ok_or_else(bad)?;
                        p += n;
                        for (k, &v) in src.iter().enumerate() {
                            if y < h && x + k < w {
                                idx[y * w + x + k] = v;
                            }
                        }
                        x += n;
                    } else if count < 0 {
                        let n = (-(count as i32)) as usize;
                        let v = *d.get(p).ok_or_else(bad)?;
                        p += 1;
                        for k in 0..n {
                            if y < h && x + k < w {
                                idx[y * w + x + k] = v;
                            }
                        }
                        x += n;
                    }
                }
            }
        }
        DELTA_FLC => {
            let mut lines = le16(d, 0).ok_or_else(bad)? as usize;
            let (mut p, mut y) = (2, 0usize);
            while lines > 0 {
                let mut last = None;
                let packets = loop {
                    let op = le16(d, p).ok_or_else(bad)?;
                    p += 2;
                    match op >> 14 {
                        3 => y = y.saturating_add((op as i16).unsigned_abs() as usize),
                        2 => last = Some(op as u8),
                        1 => return Err(bad()),
                        _ => break op as usize,
                    }
                };
                if y >= h {
                    break;
                }
                if let Some(v) = last {
                    idx[y * w + w - 1] = v;
                }
                let mut x = 0usize;
                for _ in 0..packets {
                    let (Some(&skip), Some(&count)) = (d.get(p), d.get(p + 1)) else { return Err(bad()) };
                    p += 2;
                    x += skip as usize;
                    let count = count as i8;
                    if count >= 0 {
                        let n = count as usize * 2;
                        let src = d.get(p..p + n).ok_or_else(bad)?;
                        p += n;
                        for (k, &v) in src.iter().enumerate() {
                            if x + k < w {
                                idx[y * w + x + k] = v;
                            }
                        }
                        x += n;
                    } else {
                        let word = d.get(p..p + 2).ok_or_else(bad)?;
                        p += 2;
                        let n = (-(count as i32)) as usize;
                        for k in 0..n * 2 {
                            if x + k < w {
                                idx[y * w + x + k] = word[k & 1];
                            }
                        }
                        x += n * 2;
                    }
                }
                y += 1;
                lines -= 1;
            }
        }
        _ => {} // PSTAMP, prefix chunks and anything newer
    }
    Ok(())
}

// ---- Encoder -----------------------------------------------------------------------------------

fn chunk(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
    let padded = data.len() + (data.len() & 1);
    out.extend_from_slice(&((6 + padded) as u32).to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(data);
    if data.len() & 1 == 1 {
        out.push(0);
    }
}

fn byte_run(idx: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..h {
        let line = &idx[y * w..(y + 1) * w];
        let count_at = out.len();
        out.push(0);
        let (mut packets, mut x) = (0u32, 0);
        while x < w {
            let mut run = 1;
            while x + run < w && run < 127 && line[x + run] == line[x] {
                run += 1;
            }
            if run >= 3 || (run == 2 && x + run >= w) {
                out.push(run as u8);
                out.push(line[x]);
                x += run;
            } else {
                let start = x;
                x += run;
                while x < w && x - start < 127 {
                    let r = (1..3).take_while(|k| x + k < w && line[x + k] == line[x]).count();
                    if r >= 2 {
                        break;
                    }
                    x += 1;
                }
                out.push((-((x - start) as i32)) as i8 as u8);
                out.extend_from_slice(&line[start..x]);
            }
            packets += 1;
        }
        out[count_at] = packets.min(255) as u8;
    }
    out
}

/// DELTA_FLC data turning `prev` into `cur`, or `None` when they are equal.
fn delta_flc(prev: &[u8], cur: &[u8], w: usize, h: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    let (mut lines, mut gap) = (0u16, 0usize);
    let flush_gap = |body: &mut Vec<u8>, gap: &mut usize| {
        while *gap > 0 {
            let n = (*gap).min(16000);
            body.extend_from_slice(&((-(n as i32)) as i16 as u16).to_le_bytes());
            *gap -= n;
        }
    };
    for y in 0..h {
        let (a, b) = (&prev[y * w..(y + 1) * w], &cur[y * w..(y + 1) * w]);
        if a == b {
            gap += 1;
            continue;
        }
        flush_gap(&mut body, &mut gap);
        let words = w / 2;
        let changed = |i: usize| a[2 * i..2 * i + 2] != b[2 * i..2 * i + 2];
        let mut packets: Vec<u8> = Vec::new();
        let mut count = 0u16;
        let (mut i, mut skip) = (0usize, 0usize);
        while i < words {
            if !changed(i) {
                skip += 2;
                i += 1;
                continue;
            }
            let mut end = i;
            while end < words && changed(end) {
                end += 1;
            }
            let word = |k: usize| &b[2 * k..2 * k + 2];
            let mut k = i;
            while k < end {
                while skip > 255 {
                    packets.extend_from_slice(&[255, 0]);
                    count += 1;
                    skip -= 255;
                }
                let mut run = 1;
                while k + run < end && run < 127 && word(k + run) == word(k) {
                    run += 1;
                }
                if run >= 2 {
                    packets.extend_from_slice(&[skip as u8, (-(run as i32)) as i8 as u8]);
                    packets.extend_from_slice(word(k));
                    k += run;
                } else {
                    let start = k;
                    k += 1;
                    while k < end && k - start < 127 && !(k + 1 < end && word(k + 1) == word(k)) {
                        k += 1;
                    }
                    packets.extend_from_slice(&[skip as u8, (k - start) as u8]);
                    packets.extend_from_slice(&b[2 * start..2 * k]);
                }
                count += 1;
                skip = 0;
            }
            i = end;
        }
        if w % 2 == 1 && a[w - 1] != b[w - 1] {
            body.extend_from_slice(&(0x8000u16 | b[w - 1] as u16).to_le_bytes());
        }
        body.extend_from_slice(&count.to_le_bytes());
        body.extend_from_slice(&packets);
        lines += 1;
    }
    (lines > 0).then(|| {
        let mut data = lines.to_le_bytes().to_vec();
        data.extend_from_slice(&body);
        data
    })
}

fn frame_header(out: &mut Vec<u8>, frame: &[u8], chunks: usize, delay: u16) {
    out.extend_from_slice(&((16 + frame.len()) as u32).to_le_bytes());
    out.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
    out.extend_from_slice(&(chunks as u16).to_le_bytes());
    out.extend_from_slice(&delay.to_le_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(frame);
}

/// Encodes frames `(rgba, duration in ms)` as an FLC. `repeat` is not stored: the format always loops.
pub fn encode(frames: &[(&[u8], u16)], w: usize, h: usize, _repeat: u16, o: &ImageOptions) -> Result<Vec<u8>, String> {
    if w == 0 || h == 0 || w > u16::MAX as usize || h > u16::MAX as usize {
        return Err("the image is too large for an FLC (65535 pixels at most per side)".into());
    }
    if frames.is_empty() || frames.len() > u16::MAX as usize {
        return Err("an FLC needs between 1 and 65535 frames".into());
    }
    let flat: Vec<Vec<u8>> = frames
        .iter()
        .map(|(px, _)| {
            let mut v = px.to_vec();
            flatten(&mut v, o.background);
            v
        })
        .collect();
    let refs: Vec<&[u8]> = flat.iter().map(Vec::as_slice).collect();
    let (palette, planes) = indexed(&refs, w, h, 256);

    // Chunk 1: the palette, as one packet of 256 colors.
    let mut pal = vec![1, 0, 0, 0];
    for i in 0..256 {
        pal.extend_from_slice(&palette.get(i).copied().unwrap_or([0, 0, 0]));
    }
    let mut body = Vec::new();
    let mut offsets = Vec::new();
    for (n, plane) in planes.iter().enumerate() {
        offsets.push(128 + body.len());
        let delay = frames[n].1.max(1);
        let mut data = Vec::new();
        let mut chunks = 0;
        if n == 0 {
            chunk(&mut data, COLOR_256, &pal);
            let brun = byte_run(plane, w, h);
            if brun.len() < plane.len() {
                chunk(&mut data, BYTE_RUN, &brun);
            } else {
                chunk(&mut data, COPY, plane);
            }
            chunks = 2;
        } else if let Some(delta) = delta_flc(&planes[n - 1], plane, w, h) {
            chunk(&mut data, DELTA_FLC, &delta);
            chunks = 1;
        }
        frame_header(&mut body, &data, chunks, delay);
    }
    // The ring frame leads from the last frame back to the first.
    let ring_at = 128 + body.len();
    let mut data = Vec::new();
    let mut chunks = 0;
    if let Some(delta) = delta_flc(&planes[planes.len() - 1], &planes[0], w, h) {
        chunk(&mut data, DELTA_FLC, &delta);
        chunks = 1;
    }
    frame_header(&mut body, &data, chunks, frames[0].1.max(1));

    let total = 128 + body.len();
    if total > u32::MAX as usize {
        return Err("the animation is too large for an FLC".into());
    }
    let mut out = vec![0u8; 128];
    out[0..4].copy_from_slice(&(total as u32).to_le_bytes());
    out[4..6].copy_from_slice(&0xAF12u16.to_le_bytes());
    out[6..8].copy_from_slice(&(frames.len() as u16).to_le_bytes());
    out[8..10].copy_from_slice(&(w as u16).to_le_bytes());
    out[10..12].copy_from_slice(&(h as u16).to_le_bytes());
    out[12..14].copy_from_slice(&8u16.to_le_bytes());
    out[14..16].copy_from_slice(&3u16.to_le_bytes());
    out[16..20].copy_from_slice(&(frames[0].1.max(1) as u32).to_le_bytes());
    out[50..52].copy_from_slice(&1u16.to_le_bytes());
    out[52..54].copy_from_slice(&1u16.to_le_bytes());
    out[80..84].copy_from_slice(&128u32.to_le_bytes());
    out[84..88].copy_from_slice(&(offsets.get(1).copied().unwrap_or(ring_at) as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::picture;
    use super::*;

    fn animation(w: usize, h: usize, n: usize) -> Vec<Vec<u8>> {
        // A small set of colors with something moving, so the deltas have runs, literals and skips.
        (0..n)
            .map(|f| {
                let mut v = Vec::new();
                for y in 0..h {
                    for x in 0..w {
                        let c: [u8; 4] = if (x + f * 3) % 11 == y % 5 {
                            [250, 20, 20, 255]
                        } else if x > w / 2 && (y + f) % 7 == 0 {
                            [20, 20, 250, 255]
                        } else {
                            [(x % 3 * 40) as u8, 90, 160, 255]
                        };
                        v.extend_from_slice(&c);
                    }
                }
                v
            })
            .collect()
    }

    #[test]
    fn frames_durations_and_pixels_round_trip() {
        for (w, h) in [(16, 8), (17, 9), (1, 4), (300, 20), (2, 1)] {
            let imgs = animation(w, h, 5);
            let timed: Vec<(&[u8], u16)> = imgs.iter().map(|v| v.as_slice()).zip([40, 90, 120, 30, 500]).collect();
            let bytes = encode(&timed, w, h, 0, &ImageOptions::default()).unwrap();
            assert_eq!(super::super::detect(&bytes), Some(super::super::ImageFormat::Flc));
            let d = decode(&bytes).unwrap();
            assert_eq!((d.width, d.height, d.frames.len()), (w, h, 5), "{w}x{h}");
            assert_eq!(d.durations, vec![40, 90, 120, 30, 500]);
            for (a, b) in d.frames.iter().zip(&imgs) {
                assert_eq!(a, b, "{w}x{h}");
            }
        }
    }

    #[test]
    fn identical_frames_cost_nothing_and_stay_identical() {
        let a = animation(20, 10, 1).remove(0);
        let timed = [(&a[..], 50u16), (&a[..], 60), (&a[..], 70)];
        let bytes = encode(&timed, 20, 10, 0, &ImageOptions::default()).unwrap();
        let d = decode(&bytes).unwrap();
        assert_eq!(d.frames.len(), 3);
        assert!(d.frames.iter().all(|f| *f == a));
        assert_eq!(d.durations, vec![50, 60, 70]);
    }

    #[test]
    fn many_colors_are_reduced_to_256() {
        let src = picture(40, 40, false);
        let bytes = encode(&[(&src[..], 100)], 40, 40, 0, &ImageOptions::default()).unwrap();
        let d = decode(&bytes).unwrap();
        let mut colors: Vec<[u8; 4]> = d.frames[0].chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        colors.sort();
        colors.dedup();
        assert!(colors.len() <= 256);
        let worst = d.frames[0].iter().zip(&src).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
        assert!(worst < 40, "worst {worst}");
    }

    #[test]
    fn transparent_pixels_become_the_background() {
        let px = [0, 0, 0, 0].repeat(6);
        let o = ImageOptions { background: [200, 100, 50], ..Default::default() };
        let d = decode(&encode(&[(&px[..], 100)], 3, 2, 0, &o).unwrap()).unwrap();
        assert_eq!(&d.frames[0][0..4], &[200, 100, 50, 255]);
    }

    #[test]
    fn reads_a_classic_fli_with_byte_oriented_chunks() {
        // 4x2 FLI: COLOR_64, BYTE_RUN, then a DELTA_FLI frame changing the second line.
        let mut file = vec![0u8; 128];
        file[4..6].copy_from_slice(&0xAF11u16.to_le_bytes());
        file[6..8].copy_from_slice(&2u16.to_le_bytes());
        file[8..10].copy_from_slice(&4u16.to_le_bytes());
        file[10..12].copy_from_slice(&2u16.to_le_bytes());
        file[12..14].copy_from_slice(&8u16.to_le_bytes());
        file[16..18].copy_from_slice(&7u16.to_le_bytes()); // 7 / 70 s = 100 ms
        let mut body = Vec::new();
        let mut data = Vec::new();
        chunk(&mut data, COLOR_64, &[1, 0, 0, 2, 0, 0, 0, 63, 0, 0]);
        chunk(&mut data, BYTE_RUN, &[1, 4, 0, 1, 4, 1]);
        frame_header(&mut body, &data, 2, 0);
        let mut data = Vec::new();
        chunk(&mut data, DELTA_FLI, &[1, 0, 1, 0, 1, 1, 0xFE, 0, 0, 0]);
        frame_header(&mut body, &data, 1, 0);
        file.extend_from_slice(&body);
        let d = decode(&file).unwrap();
        assert_eq!(d.frames.len(), 2);
        assert_eq!(d.durations, vec![100, 100]);
        // COLOR_64 value 63 scales to 255.
        assert_eq!(&d.frames[0][0..4], &[0, 0, 0, 255]);
        assert_eq!(&d.frames[0][4 * 4..4 * 4 + 4], &[255, 0, 0, 255]);
        // The delta paints indexes 0 over x = 1 and 2 of the second line.
        let row: Vec<[u8; 4]> = d.frames[1][16..].chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        assert_eq!(row, vec![[255, 0, 0, 255], [0, 0, 0, 255], [0, 0, 0, 255], [255, 0, 0, 255]]);
    }

    #[test]
    fn corrupt_files_are_errors_and_never_panic() {
        let imgs = animation(21, 9, 4);
        let timed: Vec<(&[u8], u16)> = imgs.iter().map(|v| (v.as_slice(), 80)).collect();
        let good = encode(&timed, 21, 9, 0, &ImageOptions::default()).unwrap();
        assert!(decode(&good[..100]).is_err());
        let _ = decode(&good[..good.len() / 2]);
        assert!(decode(&[0u8; 200]).is_err());
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&[255, 255, 255, 255]);
        assert!(decode(&huge).is_err());
        for i in 100..good.len().min(900) {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = decode(&bad);
        }
    }
}
