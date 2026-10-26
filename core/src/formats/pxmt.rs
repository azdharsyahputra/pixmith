//! Pixmith's own document format, `.pxmt` (specification: `docs/PXMT_FORMAT.md`).
//!
//! A file is an 8-byte magic, a `u16` version and a list of checksummed chunks:
//! `META` (JSON: generator, timestamps and the canvas size), `THMB` (PNG thumbnail), `SPRT` (the whole sprite as
//! an ordinary `.aseprite` file, so every feature that round-trips through `.aseprite` round-trips here) and
//! the `PEND` end marker. Unknown chunks are skipped. Readers can fetch `META` and `THMB` without decoding `SPRT`.

use crate::document::Sprite;
use crate::formats::json::{self, Value};
use crate::formats::sheet::json_str;
use crate::formats::{ase, durable, export, import};
use flate2::Crc;
use std::fmt::Write as _;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

/// Like PNG's: a non-ASCII byte, a DOS and a Unix line ending and Ctrl-Z catch text-mode transfers.
pub const MAGIC: [u8; 8] = *b"PXMT\r\n\x1a\n";
/// The newest version this code reads.
pub const VERSION: u16 = 2;
/// The version of files that need nothing newer. Version 2 adds isometric tilemaps (`document::isoext`); a file
/// that has none is still written as version 1, so older Pixmith versions keep reading it, and a file that has
/// them is refused by older versions instead of being shown wrongly.
pub const BASE_VERSION: u16 = 1;
const HEADER_LEN: u64 = 10;

pub const TAG_META: [u8; 4] = *b"META";
pub const TAG_THUMB: [u8; 4] = *b"THMB";
pub const TAG_SPRITE: [u8; 4] = *b"SPRT";
pub const TAG_END: [u8; 4] = *b"PEND";

/// Longest side of the embedded thumbnail.
pub const THUMB_MAX_SIDE: u32 = 256;
/// What a reader accepts: a thumbnail larger than this is ignored as implausible.
const THUMB_ACCEPT_SIDE: u32 = 1024;
const MAX_META_BYTES: u32 = 1 << 20;
const MAX_THUMB_BYTES: u32 = 1 << 24;

// MARK: Metadata

/// The `META` chunk: facts about the document that are cheap to read without decoding the sprite.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    /// Program and version that wrote the file, for example `pixmith-core 0.1.0`.
    pub generator: String,
    /// Seconds since the Unix epoch.
    pub created: u64,
    pub modified: u64,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub layers: u32,
    /// Members this version does not know; kept when a file is saved again.
    pub extra: Vec<(String, Value)>,
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Meta {
    pub fn for_sprite(s: &Sprite, previous: Option<&Meta>) -> Meta {
        let now = now_secs();
        Meta {
            generator: format!("pixmith-core {}", env!("CARGO_PKG_VERSION")),
            created: previous.map_or(now, |p| if p.created == 0 { now } else { p.created }),
            modified: now,
            width: s.width,
            height: s.height,
            frames: s.frame_count() as u32,
            layers: s.layers.len() as u32,
            extra: previous.map(|p| p.extra.clone()).unwrap_or_default(),
        }
    }

    pub fn to_json(&self) -> String {
        let mut o = String::new();
        write!(
            o,
            "{{\"v\":1,\"generator\":{},\"created\":{},\"modified\":{},\"width\":{},\"height\":{},\"frames\":{},\"layers\":{}",
            json_str(&self.generator),
            self.created,
            self.modified,
            self.width,
            self.height,
            self.frames,
            self.layers
        )
        .unwrap();
        for (k, v) in &self.extra {
            write!(o, ",{}:{}", json_str(k), json::to_string(v)).unwrap();
        }
        o.push('}');
        o
    }

    pub fn from_json(text: &str) -> Result<Meta> {
        let v = json::parse(text)?;
        if v.as_object().is_none() {
            return Err("META is not a JSON object".into());
        }
        let num = |k: &str| match v.get(k) {
            Some(Value::Num(n)) if *n >= 0.0 => *n as u64,
            _ => 0,
        };
        const KNOWN: [&str; 8] = ["v", "generator", "created", "modified", "width", "height", "frames", "layers"];
        Ok(Meta {
            generator: v.get("generator").and_then(Value::as_str).unwrap_or_default().to_string(),
            created: num("created"),
            modified: num("modified"),
            width: num("width").min(u32::MAX as u64) as u32,
            height: num("height").min(u32::MAX as u64) as u32,
            frames: num("frames").min(u32::MAX as u64) as u32,
            layers: num("layers").min(u32::MAX as u64) as u32,
            extra: v
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
                .map(|(k, x)| (k.clone(), x.clone()))
                .collect(),
        })
    }

    /// True when the size fields are filled in (an older or foreign writer may have left them out).
    pub fn has_size(&self) -> bool {
        self.width > 0 && self.height > 0 && self.frames > 0 && self.layers > 0
    }
}

// MARK: Thumbnails

/// Straight-alpha RGBA pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The size of a `sw` x `sh` image fitted inside `max_w` x `max_h` keeping the aspect ratio. Larger images are
/// scaled down exactly; smaller ones only when `allow_up`, by the largest whole factor that fits (pixel art
/// stays crisp). `(0, 0)` when any argument is zero.
pub fn fit_size(sw: u32, sh: u32, max_w: u32, max_h: u32, allow_up: bool) -> (u32, u32) {
    if sw == 0 || sh == 0 || max_w == 0 || max_h == 0 {
        return (0, 0);
    }
    let (swl, shl, mw, mh) = (sw as u64, sh as u64, max_w as u64, max_h as u64);
    if sw <= max_w && sh <= max_h {
        if !allow_up {
            return (sw, sh);
        }
        let k = (mw / swl).min(mh / shl).max(1);
        return ((swl * k) as u32, (shl * k) as u32);
    }
    if mw * shl <= mh * swl {
        (max_w, ((shl * mw / swl).max(1)) as u32)
    } else {
        (((swl * mh / shl).max(1)) as u32, max_h)
    }
}

/// Nearest-neighbour resampling (sampling at pixel centers) into `dst`, which holds `dw * dh * 4` bytes.
fn scale_nearest(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], dw: usize, dh: usize) {
    for y in 0..dh {
        let sy = (((2 * y as u64 + 1) * sh as u64) / (2 * dh as u64)).min(sh as u64 - 1) as usize;
        for x in 0..dw {
            let sx = (((2 * x as u64 + 1) * sw as u64) / (2 * dw as u64)).min(sw as u64 - 1) as usize;
            let (s, d) = ((sy * sw + sx) * 4, (y * dw + x) * 4);
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

impl Thumbnail {
    /// This image fitted inside `max_w` x `max_h` (see [`fit_size`]; it may be scaled up by a whole factor).
    pub fn fitted(&self, max_w: u32, max_h: u32) -> Option<Thumbnail> {
        let (w, h) = fit_size(self.width, self.height, max_w, max_h, true);
        if w == 0 || self.rgba.len() != self.width as usize * self.height as usize * 4 {
            return None;
        }
        let mut rgba = vec![0; w as usize * h as usize * 4];
        scale_nearest(&self.rgba, self.width as usize, self.height as usize, &mut rgba, w as usize, h as usize);
        Some(Thumbnail { width: w, height: h, rgba })
    }
}

/// Frame 1 of a sprite, composited and fitted inside `max_w` x `max_h`.
pub fn render_thumbnail(s: &Sprite, max_w: u32, max_h: u32, allow_up: bool) -> Option<Thumbnail> {
    let (sw, sh) = (s.width as usize, s.height as usize);
    let (w, h) = fit_size(s.width, s.height, max_w, max_h, allow_up);
    let px = s.render(0);
    if w == 0 || px.len() != sw * sh * 4 {
        return None;
    }
    let mut rgba = vec![0; w as usize * h as usize * 4];
    scale_nearest(&px, sw, sh, &mut rgba, w as usize, h as usize);
    Some(Thumbnail { width: w, height: h, rgba })
}

fn decode_thumbnail(png: &[u8]) -> Result<Thumbnail> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if png.len() < 24 || png[..8] != SIGNATURE {
        return Err("the thumbnail is not a PNG image".into());
    }
    let be = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
    if be(16) == 0 || be(20) == 0 || be(16) > THUMB_ACCEPT_SIDE || be(20) > THUMB_ACCEPT_SIDE {
        return Err("the thumbnail has an implausible size".into());
    }
    let (w, h, rgba) = import::decode_png(png)?;
    Ok(Thumbnail { width: w as u32, height: h as u32, rgba })
}

// MARK: Writing

fn put_chunk(out: &mut Vec<u8>, tag: [u8; 4], payload: &[u8]) -> Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| "the document is too large for a .pxmt file".to_string())?;
    out.extend(tag);
    out.extend(len.to_le_bytes());
    out.extend(payload);
    let mut crc = Crc::new();
    crc.update(&tag);
    crc.update(payload);
    out.extend(crc.sum().to_le_bytes());
    Ok(())
}

/// Encodes a document as a new `.pxmt` file.
pub fn write(s: &Sprite) -> Result<Vec<u8>> {
    write_with(s, None)
}

/// Like [`write`], carrying over the creation time and unknown metadata of the file being replaced.
pub fn write_with(s: &Sprite, previous: Option<&Meta>) -> Result<Vec<u8>> {
    let meta = Meta::for_sprite(s, previous);
    let sprite = ase::write(s);
    let thumb = render_thumbnail(s, THUMB_MAX_SIDE, THUMB_MAX_SIDE, false)
        .and_then(|t| export::encode_png(&t.rgba, t.width as usize, t.height as usize).ok());
    let mut out = Vec::with_capacity(sprite.len() + thumb.as_ref().map_or(0, Vec::len) + 512);
    out.extend(MAGIC);
    let version = if crate::document::isoext::needed(s) { VERSION } else { BASE_VERSION };
    out.extend(version.to_le_bytes());
    put_chunk(&mut out, TAG_META, meta.to_json().as_bytes())?;
    if let Some(t) = &thumb {
        put_chunk(&mut out, TAG_THUMB, t)?;
    }
    put_chunk(&mut out, TAG_SPRITE, &sprite)?;
    put_chunk(&mut out, TAG_END, &[])?;
    Ok(out)
}

// MARK: Reading

/// True when the bytes start with the `.pxmt` magic.
pub fn is_pxmt(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC)
}

fn tag_name(tag: &[u8; 4]) -> String {
    tag.iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '?' }).collect()
}

fn io_err(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        "the file is truncated".into()
    } else {
        e.to_string()
    }
}

/// Walks the chunk list of a stream, reading payloads only on request.
struct Chunks<R> {
    r: R,
    /// Bytes of the file after what has been consumed.
    left: u64,
}

impl<R: Read + Seek> Chunks<R> {
    fn new(mut r: R) -> Result<Self> {
        let len = r.seek(SeekFrom::End(0)).map_err(io_err)?;
        r.seek(SeekFrom::Start(0)).map_err(io_err)?;
        let mut head = [0u8; HEADER_LEN as usize];
        if len < HEADER_LEN || r.read_exact(&mut head).is_err() || head[..8] != MAGIC {
            return Err("not a Pixmith document (.pxmt)".into());
        }
        let version = u16::from_le_bytes([head[8], head[9]]);
        if version == 0 || version > VERSION {
            return Err(format!(
                "this file is .pxmt version {version}, newer than this Pixmith reads (version {VERSION}); update Pixmith"
            ));
        }
        Ok(Chunks { r, left: len - HEADER_LEN })
    }

    /// The next chunk's tag and payload length; `None` at the end marker.
    fn next(&mut self) -> Result<Option<([u8; 4], u32)>> {
        if self.left < 8 {
            return Err("the file is truncated (no end marker)".into());
        }
        let mut h = [0u8; 8];
        self.r.read_exact(&mut h).map_err(io_err)?;
        self.left -= 8;
        let tag: [u8; 4] = h[..4].try_into().unwrap();
        let len = u32::from_le_bytes(h[4..].try_into().unwrap());
        if tag == TAG_END {
            return Ok(None);
        }
        if len as u64 + 4 > self.left {
            return Err(format!("the file is truncated (in the {} chunk)", tag_name(&tag)));
        }
        Ok(Some((tag, len)))
    }

    fn skip(&mut self, len: u32) -> Result<()> {
        self.r.seek(SeekFrom::Current(len as i64 + 4)).map_err(io_err)?;
        self.left -= len as u64 + 4;
        Ok(())
    }

    /// Reads and verifies the payload of the chunk `next` just returned.
    fn payload(&mut self, tag: [u8; 4], len: u32) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len as usize];
        self.r.read_exact(&mut buf).map_err(io_err)?;
        let mut crc = [0u8; 4];
        self.r.read_exact(&mut crc).map_err(io_err)?;
        self.left -= len as u64 + 4;
        let mut sum = Crc::new();
        sum.update(&tag);
        sum.update(&buf);
        if sum.sum() != u32::from_le_bytes(crc) {
            return Err(format!("the {} chunk is corrupt (checksum mismatch)", tag_name(&tag)));
        }
        Ok(buf)
    }
}

/// Reads a `.pxmt` document. Only `SPRT` has to be intact; damage to `META` or `THMB` does not matter here.
pub fn read(bytes: &[u8]) -> Result<Sprite> {
    let mut c = Chunks::new(Cursor::new(bytes))?;
    while let Some((tag, len)) = c.next()? {
        if tag != TAG_SPRITE {
            c.skip(len)?;
            continue;
        }
        let payload = c.payload(tag, len)?;
        let s = ase::read(&payload).map_err(|e| format!("sprite data: {e}"))?;
        s.validate()?;
        return Ok(s);
    }
    Err("the file has no sprite data (SPRT chunk)".into())
}

/// The `META` chunk of a stream (`None` if the file has none), without reading the sprite.
pub fn read_meta<R: Read + Seek>(r: R) -> Result<Option<Meta>> {
    let mut c = Chunks::new(r)?;
    while let Some((tag, len)) = c.next()? {
        if tag != TAG_META {
            c.skip(len)?;
            continue;
        }
        if len > MAX_META_BYTES {
            return Err("the META chunk is implausibly large".into());
        }
        let text = c.payload(tag, len)?;
        let text = std::str::from_utf8(&text).map_err(|_| "the META chunk is not UTF-8".to_string())?;
        return Meta::from_json(text).map(Some);
    }
    Ok(None)
}

/// The embedded thumbnail of a stream (`None` if the file has none), without reading the sprite.
pub fn read_thumbnail<R: Read + Seek>(r: R) -> Result<Option<Thumbnail>> {
    let mut c = Chunks::new(r)?;
    while let Some((tag, len)) = c.next()? {
        if tag != TAG_THUMB {
            c.skip(len)?;
            continue;
        }
        if len > MAX_THUMB_BYTES {
            return Err("the THMB chunk is implausibly large".into());
        }
        return decode_thumbnail(&c.payload(tag, len)?).map(Some);
    }
    Ok(None)
}

/// [`read_meta`] on a file; only the chunk headers and `META` are read.
pub fn read_meta_file(path: impl AsRef<Path>) -> Result<Option<Meta>> {
    let path = path.as_ref();
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    read_meta(f).map_err(|e| format!("{}: {e}", path.display()))
}

/// [`read_thumbnail`] on a file; the sprite data is never read.
pub fn read_thumbnail_file(path: impl AsRef<Path>) -> Result<Option<Thumbnail>> {
    let path = path.as_ref();
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    read_thumbnail(f).map_err(|e| format!("{}: {e}", path.display()))
}

// MARK: Any document file

/// Decodes a document: `.pxmt` by its magic bytes, anything else as `.aseprite`. The sprite is validated.
pub fn read_document(bytes: &[u8]) -> Result<Sprite> {
    if is_pxmt(bytes) {
        read(bytes)
    } else {
        let s = ase::read(bytes)?;
        s.validate()?;
        Ok(s)
    }
}

/// [`read_document`] on a file; errors name the path.
pub fn open_file(path: impl AsRef<Path>) -> Result<Sprite> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    read_document(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Like [`open_file`], also accepting PNG images (one layer, one frame). Detection is by content.
pub fn open_any_file(path: impl AsRef<Path>) -> Result<Sprite> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let result = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        import::sprite_from_png(&bytes).and_then(|s| s.validate().map(|_| s))
    } else {
        read_document(&bytes)
    };
    result.map_err(|e| format!("{}: {e}", path.display()))
}

/// True when the path names a `.pxmt` document (case-insensitive).
pub fn is_pxmt_path(path: &str) -> bool {
    path.len() >= 5 && path.as_bytes()[path.len() - 5..].eq_ignore_ascii_case(b".pxmt")
}

/// What `Save` writes for `path`: `.pxmt` when the path ends in `.pxmt`, `.aseprite` otherwise.
pub fn encode_for_path(path: &str, s: &Sprite) -> Result<Vec<u8>> {
    if is_pxmt_path(path) {
        // A file being replaced keeps its creation time.
        let previous = read_meta_file(path).ok().flatten();
        write_with(s, previous.as_ref())
    } else {
        Ok(ase::write(s))
    }
}

/// Saves a document atomically in the format the path asks for (see [`encode_for_path`]).
pub fn save_file(path: &str, s: &Sprite) -> Result<()> {
    durable::write_file(path, &encode_for_path(path, s)?)
}

/// A preview of a document file fitted inside `max_w` x `max_h` (see [`fit_size`]): a `.pxmt` file's embedded
/// thumbnail (decoding the sprite only when the file has none), otherwise frame 1 of the opened document.
pub fn file_thumbnail(path: impl AsRef<Path>, max_w: u32, max_h: u32) -> Result<Thumbnail> {
    let path = path.as_ref();
    if max_w == 0 || max_h == 0 {
        return Err("the thumbnail size is empty".into());
    }
    let mut head = [0u8; 8];
    let is_pxmt =
        std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).map(|_| head == MAGIC).unwrap_or(false);
    let thumb = if is_pxmt {
        match read_thumbnail_file(path) {
            Ok(Some(t)) => Some(t),
            _ => None,
        }
    } else {
        None
    };
    let thumb = match thumb {
        Some(t) => t,
        None => {
            let s = open_any_file(path)?;
            // Render at the final size straight away; there is no embedded copy to scale.
            return render_thumbnail(&s, max_w, max_h, true).ok_or_else(|| "the document has no pixels".to_string());
        }
    };
    thumb.fitted(max_w, max_h).ok_or_else(|| "the thumbnail is damaged".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sprite {
        let mut s = Sprite::new(8, 6);
        s.draw_line(0, 0, 0, 0, 7, 5, [255, 0, 0, 255], 1);
        s.add_frame(0, true);
        s
    }

    #[test]
    fn round_trips_and_has_the_documented_layout() {
        let s = sample();
        let bytes = write(&s).unwrap();
        assert_eq!(&bytes[..8], b"PXMT\r\n\x1a\n");
        assert_eq!(&bytes[8..10], &[1, 0]);
        assert_eq!(&bytes[10..14], b"META");
        assert_eq!(&bytes[bytes.len() - 12..bytes.len() - 8], b"PEND");
        assert_eq!(read(&bytes).unwrap(), s);
        assert!(is_pxmt(&bytes) && !is_pxmt(b"PXMT"));
    }

    #[test]
    fn meta_and_thumbnail_come_without_the_sprite() {
        let s = sample();
        let mut bytes = write(&s).unwrap();
        // Destroy the sprite chunk: META and THMB are still readable.
        let at = bytes.windows(4).position(|w| w == b"SPRT").unwrap();
        bytes[at + 20] ^= 0xff;
        assert!(read(&bytes).is_err());
        let meta = read_meta(Cursor::new(&bytes)).unwrap().unwrap();
        assert_eq!((meta.width, meta.height, meta.frames, meta.layers), (8, 6, 2, 1));
        assert!(meta.created > 0 && meta.generator.starts_with("pixmith-core "));
        let t = read_thumbnail(Cursor::new(&bytes)).unwrap().unwrap();
        assert_eq!((t.width, t.height), (8, 6));
        assert_eq!(&t.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn unknown_chunks_and_unknown_meta_members_are_kept_out_of_the_way() {
        let s = sample();
        let bytes = write(&s).unwrap();
        // Insert an unknown chunk right after the header.
        let mut extra = Vec::new();
        put_chunk(&mut extra, *b"zzzz", b"future data").unwrap();
        let mut with = bytes[..10].to_vec();
        with.extend(&extra);
        with.extend(&bytes[10..]);
        assert_eq!(read(&with).unwrap(), s);

        let meta =
            Meta::from_json(r#"{"v":1,"width":3,"height":4,"frames":1,"layers":2,"future":{"a":[1,2]}}"#).unwrap();
        assert_eq!(meta.extra.len(), 1);
        assert!(meta.to_json().contains(r#""future":{"a":[1,2]}"#));
        assert_eq!(Meta::from_json(&meta.to_json()).unwrap().extra, meta.extra);
    }

    #[test]
    fn damage_is_reported_never_panics() {
        let bytes = write(&sample()).unwrap();
        // Every truncation that cuts into the sprite data fails (the end marker itself is optional).
        for n in 0..bytes.len() - 12 {
            assert!(read(&bytes[..n]).is_err(), "prefix of {n} bytes read");
        }
        // Every single flipped byte either fails or still decodes (flips in the unchecked parts), without panicking.
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0x5a;
            let _ = read(&b);
            let _ = read_meta(Cursor::new(&b));
            let _ = read_thumbnail(Cursor::new(&b));
        }
        assert!(read(b"").unwrap_err().contains("not a Pixmith document"));
        assert!(read(b"PXMT\r\n\x1a\n\x09\x00").unwrap_err().contains("version 9"));
        assert!(read(b"PXMT\r\n\x1a\n\x00\x00").unwrap_err().contains("version 0"));
    }

    #[test]
    fn fitting() {
        assert_eq!(fit_size(16, 16, 64, 64, true), (64, 64));
        assert_eq!(fit_size(16, 16, 70, 100, true), (64, 64));
        assert_eq!(fit_size(16, 16, 64, 64, false), (16, 16));
        assert_eq!(fit_size(1000, 500, 256, 256, false), (256, 128));
        assert_eq!(fit_size(500, 1000, 100, 100, true), (50, 100));
        assert_eq!(fit_size(1000, 1, 10, 10, true), (10, 1));
        assert_eq!(fit_size(0, 5, 10, 10, true), (0, 0));
        assert_eq!(fit_size(5, 5, 0, 10, true), (0, 0));
    }

    #[test]
    fn path_extension_check() {
        assert!(is_pxmt_path("a/b.pxmt") && is_pxmt_path("B.PXMT") && is_pxmt_path(".pxmt") && is_pxmt_path("c.PxMt"));
        assert!(
            !is_pxmt_path("a.aseprite") && !is_pxmt_path("pxmt") && !is_pxmt_path("a.pxmt.bak") && !is_pxmt_path("")
        );
    }
}
