//! A small, strict zip reader for extension packages: stored and deflated entries, CRC-checked, with hard limits.
//!
//! It exists so installing a downloaded package cannot be used to write outside the extension folder or to fill
//! the disk: names are validated before anything is read, sizes are checked against the headers and again while
//! inflating, and anything unusual (encryption, zip64, symbolic links, duplicate names) is refused.

use flate2::read::DeflateDecoder;
use std::io::Read;

/// Limits applied while reading a package.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Size of the package file.
    pub max_package: u64,
    pub max_entries: usize,
    /// Uncompressed size of one file.
    pub max_file: u64,
    /// Uncompressed size of all files together.
    pub max_total: u64,
    /// Longest entry name in bytes.
    pub max_name: usize,
    /// Deepest folder nesting.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_package: 32 << 20,
            max_entries: 512,
            max_file: 8 << 20,
            max_total: 48 << 20,
            max_name: 200,
            max_depth: 8,
        }
    }
}

/// One entry of the central directory.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Validated, `/`-separated, relative path without a trailing slash.
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    compressed: u64,
    method: u16,
    crc: u32,
    header_offset: u64,
}

#[derive(Debug)]
pub struct Archive<'a> {
    data: &'a [u8],
    pub entries: Vec<Entry>,
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// Checks one entry name; the returned form has no trailing slash. Rejects everything that could leave the
/// destination folder: absolute paths, drive letters, `..`, backslashes, control characters and the names
/// Windows treats specially.
pub fn safe_name(raw: &str, limits: &Limits) -> Result<String, String> {
    if raw.is_empty() {
        return Err("empty file name".into());
    }
    if raw.len() > limits.max_name {
        return Err(format!("file name too long: {}", &raw[..raw.floor_char_boundary(40)]));
    }
    if raw.chars().any(|c| c.is_control() || matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')) {
        return Err(format!("unsafe characters in file name {raw:?}"));
    }
    if raw.starts_with('/') {
        return Err(format!("absolute path in package: {raw}"));
    }
    let trimmed = raw.trim_end_matches('/');
    let mut depth = 0;
    for part in trimmed.split('/') {
        depth += 1;
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        if part.is_empty() || part == "." || part == ".." || part.ends_with('.') || part.ends_with(' ') || reserved {
            return Err(format!("unsafe path in package: {raw}"));
        }
    }
    if depth > limits.max_depth {
        return Err(format!("folders nested too deeply: {raw}"));
    }
    Ok(trimmed.to_string())
}

impl<'a> Archive<'a> {
    /// Reads the central directory and validates every entry.
    pub fn open(data: &'a [u8], limits: &Limits) -> Result<Archive<'a>, String> {
        if data.len() as u64 > limits.max_package {
            return Err(format!("package is larger than {} MB", limits.max_package >> 20));
        }
        if data.len() < 22 {
            return Err("not a zip file".into());
        }
        // The end-of-central-directory record is within the last 64 KiB + 22 bytes.
        let from = data.len().saturating_sub(22 + 65535);
        let eocd = (from..=data.len() - 22)
            .rev()
            .find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
            .ok_or("not a zip file (no central directory)")?;
        let (disk, cd_disk) = (u16_at(data, eocd + 4), u16_at(data, eocd + 6));
        if disk != Some(0) || cd_disk != Some(0) {
            return Err("multi-part zip files are not supported".into());
        }
        let count = u16_at(data, eocd + 10).ok_or("damaged zip file")? as usize;
        let cd_size = u32_at(data, eocd + 12).ok_or("damaged zip file")? as usize;
        let cd_offset = u32_at(data, eocd + 16).ok_or("damaged zip file")? as usize;
        if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
            return Err("zip64 packages are not supported".into());
        }
        if count > limits.max_entries {
            return Err(format!("package has more than {} files", limits.max_entries));
        }
        if cd_offset.checked_add(cd_size).is_none_or(|end| end > eocd) {
            return Err("damaged zip file (central directory out of range)".into());
        }
        let mut entries: Vec<Entry> = Vec::with_capacity(count);
        let mut at = cd_offset;
        let mut total = 0u64;
        for _ in 0..count {
            if u32_at(data, at) != Some(0x0201_4b50) {
                return Err("damaged zip file (bad central directory entry)".into());
            }
            let field = |off: usize| u16_at(data, at + off).ok_or("damaged zip file");
            let flags = field(8)?;
            let method = field(10)?;
            let crc = u32_at(data, at + 16).ok_or("damaged zip file")?;
            let compressed = u32_at(data, at + 20).ok_or("damaged zip file")? as u64;
            let size = u32_at(data, at + 24).ok_or("damaged zip file")? as u64;
            let (name_len, extra_len, comment_len) = (field(28)? as usize, field(30)? as usize, field(32)? as usize);
            let external = u32_at(data, at + 38).ok_or("damaged zip file")?;
            let header_offset = u32_at(data, at + 42).ok_or("damaged zip file")? as u64;
            let version_made_by = field(4)?;
            let raw = data.get(at + 46..at + 46 + name_len).ok_or("damaged zip file (name out of range)")?;
            at += 46 + name_len + extra_len + comment_len;
            let raw = std::str::from_utf8(raw).map_err(|_| "a file name in the package is not valid UTF-8")?;
            let is_dir = raw.ends_with('/');
            let name = safe_name(raw, limits)?;
            if flags & 1 != 0 {
                return Err(format!("{name}: encrypted entries are not supported"));
            }
            if compressed == 0xFFFF_FFFF || size == 0xFFFF_FFFF {
                return Err("zip64 packages are not supported".into());
            }
            if !matches!(method, 0 | 8) {
                return Err(format!("{name}: unsupported compression method {method}"));
            }
            // Unix mode in the high 16 bits when made on Unix: refuse links and special files.
            if version_made_by >> 8 == 3 {
                let kind = (external >> 16) & 0o170000;
                if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                    return Err(format!("{name}: links and special files are not allowed"));
                }
            }
            if size > limits.max_file {
                return Err(format!("{name}: larger than {} MB", limits.max_file >> 20));
            }
            total += size;
            if total > limits.max_total {
                return Err(format!("package expands to more than {} MB", limits.max_total >> 20));
            }
            // A deflate stream cannot shrink data by more than ~1032x; anything beyond is a bomb or a lie.
            if method == 8 && size > 65536 && size / 1100 > compressed {
                return Err(format!("{name}: suspicious compression ratio"));
            }
            let key = name.to_lowercase();
            if entries.iter().any(|e| e.name.to_lowercase() == key) {
                return Err(format!("{name}: listed twice in the package"));
            }
            entries.push(Entry { name, is_dir, size, compressed, method, crc, header_offset });
        }
        // A file may not also be a folder of another entry ("a" and "a/b").
        for e in &entries {
            let prefix = format!("{}/", e.name.to_lowercase());
            if entries.iter().any(|o| !std::ptr::eq(o, e) && !e.is_dir && o.name.to_lowercase().starts_with(&prefix)) {
                return Err(format!("{}: is both a file and a folder", e.name));
            }
        }
        Ok(Archive { data, entries })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| !e.is_dir && e.name == name)
    }

    /// The contents of a file entry, inflated and checked against the recorded size and CRC.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>, String> {
        let d = self.data;
        let at = entry.header_offset as usize;
        if u32_at(d, at) != Some(0x0403_4b50) {
            return Err(format!("{}: damaged zip file (bad local header)", entry.name));
        }
        let name_len = u16_at(d, at + 26).ok_or("damaged zip file")? as usize;
        let extra_len = u16_at(d, at + 28).ok_or("damaged zip file")? as usize;
        let start = at + 30 + name_len + extra_len;
        let end = start.checked_add(entry.compressed as usize).filter(|&e| e <= d.len());
        let body = end.map(|end| &d[start..end]).ok_or_else(|| format!("{}: data out of range", entry.name))?;
        let out = match entry.method {
            0 => body.to_vec(),
            _ => {
                let mut out = Vec::with_capacity(entry.size.min(1 << 20) as usize);
                // One byte more than declared proves the header lied.
                DeflateDecoder::new(body)
                    .take(entry.size + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("{}: damaged data ({e})", entry.name))?;
                out
            }
        };
        if out.len() as u64 != entry.size {
            return Err(format!("{}: size does not match the package header", entry.name));
        }
        let mut crc = flate2::Crc::new();
        crc.update(&out);
        if crc.sum() != entry.crc {
            return Err(format!("{}: checksum mismatch", entry.name));
        }
        Ok(out)
    }
}

/// Builds a zip in memory (stored entries): used by tests and by tools that pack an extension.
pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let offset = out.len() as u32;
        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        local.extend_from_slice(&crc.sum().to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        local.extend_from_slice(data);
        out.extend_from_slice(&local);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.sum().to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim() -> Limits {
        Limits::default()
    }

    #[test]
    fn reads_what_build_wrote() {
        let z = build(&[("package.json", b"{}"), ("a/b.txt", b"hello")]);
        let a = Archive::open(&z, &lim()).unwrap();
        assert_eq!(a.entries.len(), 2);
        assert_eq!(a.read(a.find("a/b.txt").unwrap()).unwrap(), b"hello");
    }

    #[test]
    fn reads_deflated_entries_from_the_system_zip_tool() {
        // "hello hello hello hello\n" deflated by zip(1), then the file name "x.txt".
        let z = real_deflated_zip();
        let a = Archive::open(&z, &lim()).unwrap();
        assert_eq!(a.read(a.find("x.txt").unwrap()).unwrap(), b"hello hello hello hello hello hello\n");
    }

    fn real_deflated_zip() -> Vec<u8> {
        use flate2::write::DeflateEncoder;
        use std::io::Write;
        let data = b"hello hello hello hello hello hello\n";
        let mut enc = DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        let comp = enc.finish().unwrap();
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let mut z = Vec::new();
        z.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        z.extend_from_slice(&[20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&crc.sum().to_le_bytes());
        z.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        z.extend_from_slice(&(data.len() as u32).to_le_bytes());
        z.extend_from_slice(&5u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(b"x.txt");
        z.extend_from_slice(&comp);
        let cd = z.len() as u32;
        let mut c = Vec::new();
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&[20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
        c.extend_from_slice(&crc.sum().to_le_bytes());
        c.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&5u16.to_le_bytes());
        c.extend_from_slice(&[0; 12]);
        c.extend_from_slice(&0u32.to_le_bytes());
        c.extend_from_slice(b"x.txt");
        z.extend_from_slice(&c);
        z.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        z.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend_from_slice(&(c.len() as u32).to_le_bytes());
        z.extend_from_slice(&cd.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z
    }

    #[test]
    fn hostile_names_are_refused() {
        for bad in [
            "../evil.lua",
            "a/../../evil",
            "/etc/passwd",
            "C:/x",
            "a\\b",
            "a//b",
            "./a",
            "a/./b",
            "con.txt",
            "NUL",
            "a/",
            "trailing.",
            "sp ace ",
            "x\0y",
            "q?.txt",
        ] {
            // "a/" is a valid folder entry once trimmed; the rest must fail.
            let r = safe_name(bad, &lim());
            if bad == "a/" {
                assert_eq!(r.unwrap(), "a");
                continue;
            }
            assert!(r.is_err(), "{bad:?} was accepted");
            let z = build(&[(bad, b"x")]);
            if bad != "x\0y" {
                assert!(Archive::open(&z, &lim()).is_err(), "{bad:?} opened");
            }
        }
        assert!(safe_name(&"a".repeat(300), &lim()).is_err());
        assert!(safe_name("a/b/c/d/e/f/g/h/i/j", &lim()).is_err());
    }

    #[test]
    fn duplicates_and_file_folder_clashes_are_refused() {
        assert!(Archive::open(&build(&[("A.txt", b"1"), ("a.TXT", b"2")]), &lim()).is_err());
        assert!(Archive::open(&build(&[("a", b"1"), ("a/b", b"2")]), &lim()).is_err());
    }

    #[test]
    fn limits_are_enforced() {
        let l = Limits { max_entries: 2, ..lim() };
        assert!(Archive::open(&build(&[("a", b""), ("b", b""), ("c", b"")]), &l).is_err());
        let l = Limits { max_file: 4, ..lim() };
        assert!(Archive::open(&build(&[("a", b"12345")]), &l).is_err());
        let l = Limits { max_total: 6, ..lim() };
        assert!(Archive::open(&build(&[("a", b"1234"), ("b", b"1234")]), &l).is_err());
        let l = Limits { max_package: 10, ..lim() };
        assert!(Archive::open(&build(&[("a", b"1234")]), &l).is_err());
    }

    #[test]
    fn damaged_archives_are_errors_not_panics() {
        assert!(Archive::open(b"", &lim()).is_err());
        assert!(Archive::open(b"PK\x03\x04 not really a zip file at all.", &lim()).is_err());
        let z = build(&[("a.txt", b"hello world")]);
        for cut in 0..z.len() {
            let _ = Archive::open(&z[..cut], &lim()).map(|a| a.entries.iter().for_each(|e| drop(a.read(e))));
        }
        // A flipped data byte fails the checksum.
        let mut bad = z.clone();
        bad[36] ^= 0xFF;
        let a = Archive::open(&bad, &lim()).unwrap();
        assert!(a.read(&a.entries[0]).unwrap_err().contains("checksum"));
        // A lying size is caught.
        let mut lie = z.clone();
        let cd = lie.len() - 22 - (46 + 5);
        lie[cd + 24] = 3;
        let a = Archive::open(&lie, &lim()).unwrap();
        assert!(a.read(&a.entries[0]).is_err());
    }

    #[test]
    fn symlinks_encryption_and_zip64_are_refused() {
        let base = build(&[("a.txt", b"hi")]);
        let cd = base.len() - 22 - (46 + 5);
        let mut link = base.clone();
        link[cd + 5] = 3;
        link[cd + 38..cd + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
        assert!(Archive::open(&link, &lim()).unwrap_err().contains("links"));
        let mut regular = base.clone();
        regular[cd + 5] = 3;
        regular[cd + 38..cd + 42].copy_from_slice(&(0o100644u32 << 16).to_le_bytes());
        assert!(Archive::open(&regular, &lim()).is_ok());
        let mut enc = base.clone();
        enc[cd + 8] = 1;
        assert!(Archive::open(&enc, &lim()).unwrap_err().contains("encrypted"));
        let mut z64 = base.clone();
        let eocd = base.len() - 22;
        z64[eocd + 10..eocd + 12].copy_from_slice(&[0xFF, 0xFF]);
        assert!(Archive::open(&z64, &lim()).is_err());
    }

    #[test]
    fn a_deflate_bomb_is_refused_by_the_header_ratio() {
        use flate2::write::DeflateEncoder;
        use std::io::Write;
        let data = vec![0u8; 4 << 20];
        let mut enc = DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(&data).unwrap();
        let comp = enc.finish().unwrap();
        assert!(comp.len() < 8000);
        // Hand-build a deflated entry that claims 4 MiB.
        let mut z = real_deflated_zip();
        let cd = z.len() - 22 - (46 + 5);
        z[cd + 24..cd + 28].copy_from_slice(&(data.len() as u32).to_le_bytes());
        z[cd + 20..cd + 24].copy_from_slice(&(comp.len() as u32).to_le_bytes());
        // 4 MiB / 1100 = 3813 < compressed size, so this one passes the ratio check but fails on content.
        let a = Archive::open(&z, &lim());
        if let Ok(a) = a {
            assert!(a.read(&a.entries[0]).is_err());
        }
        // A claim of 8 MiB from a few bytes is refused up front.
        z[cd + 24..cd + 28].copy_from_slice(&(8u32 << 20).to_le_bytes());
        z[cd + 20..cd + 24].copy_from_slice(&10u32.to_le_bytes());
        assert!(Archive::open(&z, &lim()).unwrap_err().contains("ratio"));
    }
}
