//! The `.lsv` save package: a Larian package (LSPK), version 18.
//!
//! Layout, as found in every sample save:
//!
//! ```text
//! 0   "LSPK"
//! 4   u32  version (18)
//! 8   u64  offset of the file list
//! 16  u32  size of the file list (8 + its compressed size)
//! 20  u8   flags
//! 21  u8   priority
//! 22  [16] MD5 of every file's uncompressed contents in on-disk order, each byte plus one
//! 38  u16  number of parts (1)
//! 40  file data; after each file, 0xAD bytes up to the next multiple of 64 counted from 40
//! ..  file list: u32 count, u32 compressed size, LZ4 block of 272-byte entries
//! ```
//!
//! A file list entry is a 256-byte name, a 48-bit offset, a part number, a flags byte whose low
//! four bits give the compression method, the size on disk and the uncompressed size.
//!
//! Files that are not replaced are written back with the exact bytes they were read with, so a
//! package that is read and written without changes comes out identical.

use anyhow::{anyhow, bail, ensure, Context, Result};
use md5::{Digest, Md5};
use std::io::Read;
use std::sync::Arc;

pub const MAGIC: &[u8; 4] = b"LSPK";
/// The only package version the game writes for saves, from Patch 4 to Patch 8.
pub const VERSION: u32 = 18;

const HEADER_LEN: usize = 40;
const ENTRY_LEN: usize = 272;
const NAME_LEN: usize = 256;
const ALIGN: usize = 64;
const PAD: u8 = 0xAD;
/// Flags byte the game writes for a Zstandard file: method 3, default level.
const ZSTD_FLAGS: u8 = 0x23;
/// Larger than any save seen (the biggest file is 76 MB uncompressed); stops a corrupt size
/// field from asking for gigabytes.
const MAX_FILE_LEN: usize = 1 << 30;

/// How a file is stored in the package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    None,
    Zlib,
    Lz4,
    Zstd,
}

impl Method {
    fn from_flags(flags: u8) -> Result<Method> {
        Ok(match flags & 0x0f {
            0 => Method::None,
            1 => Method::Zlib,
            2 => Method::Lz4,
            3 => Method::Zstd,
            other => bail!("unknown compression method {other}"),
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Method::None => "none",
            Method::Zlib => "zlib",
            Method::Lz4 => "lz4",
            Method::Zstd => "zstd",
        }
    }
}

#[derive(Clone)]
enum Payload {
    /// Bytes at this range of the file the package was read from.
    Original { offset: usize, len: usize },
    /// Bytes produced by `Package::replace`.
    Replaced(Arc<Vec<u8>>),
}

/// One file in the package.
#[derive(Clone)]
pub struct Entry {
    /// The 256-byte name field exactly as stored.
    raw_name: [u8; NAME_LEN],
    name: String,
    part: u8,
    flags: u8,
    uncompressed_len: u32,
    payload: Payload,
}

impl Entry {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn flags(&self) -> u8 {
        self.flags
    }

    pub fn method(&self) -> Method {
        // Checked when the entry was read or created.
        Method::from_flags(self.flags).unwrap_or(Method::None)
    }

    pub fn stored_len(&self) -> usize {
        match &self.payload {
            Payload::Original { len, .. } => *len,
            Payload::Replaced(bytes) => bytes.len(),
        }
    }

    /// Length of the file's contents once unpacked.
    pub fn len(&self) -> usize {
        if self.method() == Method::None {
            self.stored_len()
        } else {
            self.uncompressed_len as usize
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_replaced(&self) -> bool {
        matches!(self.payload, Payload::Replaced(_))
    }
}

/// A save package held in memory.
#[derive(Clone)]
pub struct Package {
    /// The file the package was read from. Unreplaced entries point into it.
    data: Arc<Vec<u8>>,
    pub flags: u8,
    pub priority: u8,
    md5: [u8; 16],
    /// In file list order.
    entries: Vec<Entry>,
    /// Indices into `entries`, in the order the files sit on disk.
    disk_order: Vec<usize>,
    /// The file list as read: uncompressed and compressed. Reused on write when unchanged.
    original_table: (Vec<u8>, Vec<u8>),
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}
fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}
fn u64_at(d: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[at..at + 8]);
    u64::from_le_bytes(b)
}

/// Bytes of padding that follow a file ending at `end`.
fn padding_after(end: usize) -> usize {
    (ALIGN - (end - HEADER_LEN) % ALIGN) % ALIGN
}

impl Package {
    /// Reads a package from the bytes of a `.lsv` file.
    pub fn read(data: Vec<u8>) -> Result<Package> {
        ensure!(data.len() >= HEADER_LEN, "too short to be a save package");
        ensure!(
            &data[0..4] == MAGIC,
            "not a save package (no LSPK signature)"
        );
        let version = u32_at(&data, 4);
        ensure!(
            version == VERSION,
            "package version {version} is not supported (expected {VERSION})"
        );
        let list_offset = usize::try_from(u64_at(&data, 8)).context("file list offset")?;
        let list_size = u32_at(&data, 16) as usize;
        let flags = data[20];
        let priority = data[21];
        let mut md5 = [0u8; 16];
        md5.copy_from_slice(&data[22..38]);
        let parts = u16_at(&data, 38);
        ensure!(
            parts == 1,
            "packages split into {parts} parts are not supported"
        );

        ensure!(
            list_offset >= HEADER_LEN && list_offset + 8 <= data.len(),
            "file list offset is outside the file"
        );
        let count = u32_at(&data, list_offset) as usize;
        let compressed_len = u32_at(&data, list_offset + 4) as usize;
        ensure!(
            list_size == compressed_len + 8,
            "file list size does not match its header"
        );
        let list_end = list_offset + 8 + compressed_len;
        ensure!(list_end == data.len(), "data after the file list");
        ensure!(count <= 1 << 20, "implausible file count {count}");
        let compressed = data[list_offset + 8..list_end].to_vec();
        let table = lz4_flex::block::decompress(&compressed, count * ENTRY_LEN)
            .map_err(|e| anyhow!("file list: {e}"))?;
        ensure!(table.len() == count * ENTRY_LEN, "file list is truncated");

        let mut entries = Vec::with_capacity(count);
        for raw in table.chunks_exact(ENTRY_LEN) {
            let mut raw_name = [0u8; NAME_LEN];
            raw_name.copy_from_slice(&raw[..NAME_LEN]);
            let name_len = raw_name.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
            let name = std::str::from_utf8(&raw_name[..name_len])
                .context("file name is not UTF-8")?
                .to_owned();
            let offset = u32_at(raw, 256) as u64 | (u16_at(raw, 260) as u64) << 32;
            let offset = usize::try_from(offset).context("file offset")?;
            let part = raw[262];
            let entry_flags = raw[263];
            let stored_len = u32_at(raw, 264) as usize;
            let uncompressed_len = u32_at(raw, 268);
            ensure!(part == 0, "{name}: stored in part {part}");
            Method::from_flags(entry_flags).with_context(|| name.clone())?;
            ensure!(
                offset >= HEADER_LEN && offset + stored_len <= list_offset,
                "{name}: data is outside the package"
            );
            entries.push(Entry {
                raw_name,
                name,
                part,
                flags: entry_flags,
                uncompressed_len,
                payload: Payload::Original {
                    offset,
                    len: stored_len,
                },
            });
        }

        let offset_of = |e: &Entry| match e.payload {
            Payload::Original { offset, .. } => offset,
            Payload::Replaced(_) => unreachable!("entries were just read"),
        };
        let mut disk_order: Vec<usize> = (0..entries.len()).collect();
        disk_order.sort_by_key(|&i| offset_of(&entries[i]));

        // The layout the writer produces must be the layout that was read, or an unchanged
        // package would not come out identical.
        let mut pos = HEADER_LEN;
        for &i in &disk_order {
            let e = &entries[i];
            ensure!(
                offset_of(e) == pos,
                "{}: unexpected position in the package",
                e.name
            );
            let end = pos + e.stored_len();
            let pad = padding_after(end);
            ensure!(
                end + pad <= list_offset && data[end..end + pad].iter().all(|&b| b == PAD),
                "{}: unexpected padding",
                e.name
            );
            pos = end + pad;
        }
        ensure!(pos == list_offset, "unexpected gap before the file list");

        Ok(Package {
            data: Arc::new(data),
            flags,
            priority,
            md5,
            entries,
            disk_order,
            original_table: (table, compressed),
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Entries in the order their data sits in the package.
    pub fn entries_on_disk(&self) -> impl Iterator<Item = &Entry> {
        self.disk_order.iter().map(|&i| &self.entries[i])
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// The MD5 stored in the header.
    pub fn stored_md5(&self) -> [u8; 16] {
        self.md5
    }

    /// The file's bytes as they sit in the package (compressed).
    pub fn stored<'a>(&'a self, entry: &'a Entry) -> &'a [u8] {
        match &entry.payload {
            Payload::Original { offset, len } => &self.data[*offset..*offset + *len],
            Payload::Replaced(bytes) => bytes,
        }
    }

    /// The file's uncompressed contents.
    pub fn unpack(&self, entry: &Entry) -> Result<Vec<u8>> {
        unpack_stored(&entry.name, entry.method(), self.stored(entry), entry.len())
    }

    /// The uncompressed contents of the file called `name`.
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self
            .find(name)
            .ok_or_else(|| anyhow!("the save has no file called {name}"))?;
        self.unpack(entry)
    }

    /// Replaces the contents of an existing file. The new contents are stored with Zstandard,
    /// as the game stores every file in a Patch 8 save.
    pub fn replace(&mut self, name: &str, contents: &[u8]) -> Result<()> {
        let index = self
            .entries
            .iter()
            .position(|e| e.name == name)
            .ok_or_else(|| anyhow!("the save has no file called {name}"))?;
        ensure!(
            contents.len() <= u32::MAX as usize,
            "{name}: too large for a save package"
        );
        ensure!(
            self.entries[index].method() == Method::Zstd,
            "{name}: only Zstandard files can be replaced"
        );
        let stored = zstd::bulk::compress(contents, zstd::DEFAULT_COMPRESSION_LEVEL)
            .with_context(|| format!("{name}: zstd"))?;
        ensure!(
            stored.len() <= u32::MAX as usize,
            "{name}: too large for a save package"
        );
        let entry = &mut self.entries[index];
        entry.flags = ZSTD_FLAGS;
        entry.uncompressed_len = contents.len() as u32;
        entry.payload = Payload::Replaced(Arc::new(stored));
        Ok(())
    }

    /// The header checksum for the package's current contents: MD5 over every file's
    /// uncompressed contents in on-disk order, with one added to each byte of the digest.
    pub fn compute_md5(&self) -> Result<[u8; 16]> {
        let mut hasher = Md5::new();
        for entry in self.entries_on_disk() {
            hasher.update(self.unpack(entry)?);
        }
        let mut digest: [u8; 16] = hasher.finalize().into();
        for byte in &mut digest {
            *byte = byte.wrapping_add(1);
        }
        Ok(digest)
    }

    /// Serializes the package. With no file replaced the result is identical to what was read.
    pub fn write(&self) -> Result<Vec<u8>> {
        let changed = self.entries.iter().any(Entry::is_replaced);
        let md5 = if changed {
            self.compute_md5()?
        } else {
            self.md5
        };

        let mut out = Vec::with_capacity(self.data.len());
        out.resize(HEADER_LEN, 0);
        let mut offsets = vec![0usize; self.entries.len()];
        for &i in &self.disk_order {
            let entry = &self.entries[i];
            offsets[i] = out.len();
            out.extend_from_slice(self.stored(entry));
            let pad = padding_after(out.len());
            out.resize(out.len() + pad, PAD);
        }
        let list_offset = out.len();

        let mut table = Vec::with_capacity(self.entries.len() * ENTRY_LEN);
        for (entry, &offset) in self.entries.iter().zip(&offsets) {
            ensure!(
                (offset as u64) < 1 << 48,
                "{}: offset too large for a save package",
                entry.name
            );
            table.extend_from_slice(&entry.raw_name);
            table.extend_from_slice(&(offset as u32).to_le_bytes());
            table.extend_from_slice(&((offset as u64 >> 32) as u16).to_le_bytes());
            table.push(entry.part);
            table.push(entry.flags);
            table.extend_from_slice(&(entry.stored_len() as u32).to_le_bytes());
            table.extend_from_slice(&entry.uncompressed_len.to_le_bytes());
        }
        // Another encoder's LZ4 output differs from the game's, so the game's own bytes are
        // kept whenever the list they encode is unchanged.
        let compressed = if table == self.original_table.0 {
            self.original_table.1.clone()
        } else {
            lz4_flex::block::compress(&table)
        };
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        out.extend_from_slice(&compressed);

        out[0..4].copy_from_slice(MAGIC);
        out[4..8].copy_from_slice(&VERSION.to_le_bytes());
        out[8..16].copy_from_slice(&(list_offset as u64).to_le_bytes());
        out[16..20].copy_from_slice(&(compressed.len() as u32 + 8).to_le_bytes());
        out[20] = self.flags;
        out[21] = self.priority;
        out[22..38].copy_from_slice(&md5);
        out[38..40].copy_from_slice(&1u16.to_le_bytes());
        Ok(out)
    }
}

fn unpack_stored(name: &str, method: Method, stored: &[u8], expected: usize) -> Result<Vec<u8>> {
    ensure!(
        expected <= MAX_FILE_LEN,
        "{name}: implausible size {expected}"
    );
    let out = match method {
        Method::None => stored.to_vec(),
        Method::Zlib => {
            let mut out = Vec::with_capacity(expected);
            flate2::read::ZlibDecoder::new(stored)
                .take(expected as u64 + 1)
                .read_to_end(&mut out)
                .with_context(|| format!("{name}: zlib"))?;
            out
        }
        Method::Lz4 => lz4_flex::block::decompress(stored, expected)
            .map_err(|e| anyhow!("{name}: lz4: {e}"))?,
        Method::Zstd => {
            let mut out = Vec::with_capacity(expected);
            zstd::stream::read::Decoder::new(stored)
                .with_context(|| format!("{name}: zstd"))?
                .take(expected as u64 + 1)
                .read_to_end(&mut out)
                .with_context(|| format!("{name}: zstd"))?;
            out
        }
    };
    ensure!(
        out.len() == expected,
        "{name}: unpacked to {} bytes, expected {expected}",
        out.len()
    );
    Ok(out)
}

/// Reads a few small files out of a package on disk without loading the rest: the header,
/// the file list at the end, then only the files asked for. A save is tens of megabytes and
/// a saves folder holds hundreds, so listing them must not read each one whole.
///
/// Returns the contents in the order of `names`, with `None` for a file the package lacks.
pub fn peek(path: &std::path::Path, names: &[&str]) -> Result<Vec<Option<Vec<u8>>>> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)
        .context("too short to be a save package")?;
    ensure!(
        &header[0..4] == MAGIC,
        "not a save package (no LSPK signature)"
    );
    let version = u32_at(&header, 4);
    ensure!(
        version == VERSION,
        "package version {version} is not supported (expected {VERSION})"
    );
    let list_offset = u64_at(&header, 8);
    let list_size = u32_at(&header, 16) as u64;
    ensure!(
        list_size >= 8 && list_offset >= HEADER_LEN as u64 && list_offset + list_size <= file_len,
        "file list is outside the file"
    );
    let mut list = vec![0u8; list_size as usize];
    file.seek(SeekFrom::Start(list_offset))?;
    file.read_exact(&mut list)?;
    let count = u32_at(&list, 0) as usize;
    ensure!(count <= 1 << 20, "implausible file count {count}");
    let table = lz4_flex::block::decompress(&list[8..], count * ENTRY_LEN)
        .map_err(|e| anyhow!("file list: {e}"))?;
    ensure!(table.len() == count * ENTRY_LEN, "file list is truncated");

    let mut found = vec![None; names.len()];
    for raw in table.chunks_exact(ENTRY_LEN) {
        let name_len = raw[..NAME_LEN]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(NAME_LEN);
        let Some(wanted) = names.iter().position(|n| n.as_bytes() == &raw[..name_len]) else {
            continue;
        };
        let name = names[wanted];
        let offset = u32_at(raw, 256) as u64 | (u16_at(raw, 260) as u64) << 32;
        let flags = raw[263];
        let stored_len = u32_at(raw, 264) as usize;
        let uncompressed_len = u32_at(raw, 268) as usize;
        let method = Method::from_flags(flags).with_context(|| name.to_owned())?;
        ensure!(
            offset >= HEADER_LEN as u64 && offset + stored_len as u64 <= list_offset,
            "{name}: data is outside the package"
        );
        ensure!(stored_len <= MAX_FILE_LEN, "{name}: implausible size");
        let mut stored = vec![0u8; stored_len];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut stored)?;
        let expected = if method == Method::None {
            stored_len
        } else {
            uncompressed_len
        };
        found[wanted] = Some(unpack_stored(name, method, &stored, expected)?);
    }
    Ok(found)
}

/// One file in a package, as its file list describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub part: u8,
    pub method: Option<Method>,
    pub stored_len: u64,
    pub len: u64,
}

/// What a package on disk says about itself, read without loading its contents: for the
/// game's own packages, which run to gigabytes.
#[derive(Clone, Debug)]
pub struct Listing {
    pub version: u32,
    pub parts: u16,
    pub flags: u8,
    pub files: Vec<Listed>,
}

/// Reads a package's header and file list, and nothing else. A version this code doesn't
/// read is reported with no files.
pub fn list(path: &std::path::Path) -> Result<Listing> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)
        .context("too short to be a package")?;
    ensure!(&header[0..4] == MAGIC, "not a package (no LSPK signature)");
    let version = u32_at(&header, 4);
    let mut listing = Listing {
        version,
        parts: u16_at(&header, 38),
        flags: header[20],
        files: Vec::new(),
    };
    if version != VERSION {
        return Ok(listing);
    }
    let list_offset = u64_at(&header, 8);
    let list_size = u32_at(&header, 16) as u64;
    ensure!(
        list_size >= 8 && list_offset >= HEADER_LEN as u64 && list_offset + list_size <= file_len,
        "file list is outside the file"
    );
    let mut list = vec![0u8; list_size as usize];
    file.seek(SeekFrom::Start(list_offset))?;
    file.read_exact(&mut list)?;
    let count = u32_at(&list, 0) as usize;
    ensure!(count <= 1 << 22, "implausible file count {count}");
    let table = lz4_flex::block::decompress(&list[8..], count * ENTRY_LEN)
        .map_err(|e| anyhow!("file list: {e}"))?;
    ensure!(table.len() == count * ENTRY_LEN, "file list is truncated");
    for raw in table.chunks_exact(ENTRY_LEN) {
        let name_len = raw[..NAME_LEN]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(NAME_LEN);
        listing.files.push(Listed {
            name: String::from_utf8_lossy(&raw[..name_len]).into_owned(),
            part: raw[262],
            method: Method::from_flags(raw[263]).ok(),
            stored_len: u32_at(raw, 264) as u64,
            len: u32_at(raw, 268) as u64,
        });
    }
    Ok(listing)
}

/// Builds a package from scratch. Used by tests, which have no real save to start from.
pub fn build(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    let mut out = vec![0u8; HEADER_LEN];
    let mut table = Vec::new();
    let mut hasher = Md5::new();
    for (name, contents) in files {
        ensure!(name.len() < NAME_LEN, "{name}: name too long");
        let stored = zstd::bulk::compress(contents, zstd::DEFAULT_COMPRESSION_LEVEL)?;
        let offset = out.len();
        out.extend_from_slice(&stored);
        let pad = padding_after(out.len());
        out.resize(out.len() + pad, PAD);
        hasher.update(contents);

        let mut raw_name = [0u8; NAME_LEN];
        raw_name[..name.len()].copy_from_slice(name.as_bytes());
        table.extend_from_slice(&raw_name);
        table.extend_from_slice(&(offset as u32).to_le_bytes());
        table.extend_from_slice(&((offset as u64 >> 32) as u16).to_le_bytes());
        table.push(0);
        table.push(ZSTD_FLAGS);
        table.extend_from_slice(&(stored.len() as u32).to_le_bytes());
        table.extend_from_slice(&(contents.len() as u32).to_le_bytes());
    }
    let mut md5: [u8; 16] = hasher.finalize().into();
    for byte in &mut md5 {
        *byte = byte.wrapping_add(1);
    }
    let list_offset = out.len();
    let compressed = lz4_flex::block::compress(&table);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);

    out[0..4].copy_from_slice(MAGIC);
    out[4..8].copy_from_slice(&VERSION.to_le_bytes());
    out[8..16].copy_from_slice(&(list_offset as u64).to_le_bytes());
    out[16..20].copy_from_slice(&(compressed.len() as u32 + 8).to_le_bytes());
    out[22..38].copy_from_slice(&md5);
    out[38..40].copy_from_slice(&1u16.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_a_package_without_loading_it() {
        let bytes = build(&[("a.lsf", b"first"), ("dir/b.txt", b"second file")]).unwrap();
        let path = std::env::temp_dir().join(format!("nautiloid-list-{}.pak", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let listing = list(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(listing.version, 18);
        assert_eq!(listing.parts, 1);
        let names: Vec<&str> = listing.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["a.lsf", "dir/b.txt"]);
        assert_eq!(listing.files[1].len, 11);
        assert_eq!(listing.files[0].part, 0);
    }

    use super::*;

    fn sample() -> Vec<u8> {
        let big: Vec<u8> = (0..200_000u32)
            .flat_map(|i| (i % 251).to_le_bytes())
            .collect();
        build(&[
            ("meta.lsf", b"first file".as_slice()),
            ("LevelCache/WLD_Main_A.lsf", &big),
            ("empty.bin", b"".as_slice()),
        ])
        .unwrap()
    }

    #[test]
    fn unchanged_package_writes_identical_bytes() {
        let bytes = sample();
        let package = Package::read(bytes.clone()).unwrap();
        assert_eq!(package.entries().len(), 3);
        assert_eq!(package.read_file("meta.lsf").unwrap(), b"first file");
        assert_eq!(package.compute_md5().unwrap(), package.stored_md5());
        assert_eq!(package.write().unwrap(), bytes);
    }

    #[test]
    fn replacing_a_file_updates_contents_offsets_and_checksum() {
        let mut package = Package::read(sample()).unwrap();
        let before = package.stored_md5();
        let big = package.read_file("LevelCache/WLD_Main_A.lsf").unwrap();
        package.replace("meta.lsf", &vec![7u8; 5000]).unwrap();

        let written = Package::read(package.write().unwrap()).unwrap();
        assert_eq!(written.read_file("meta.lsf").unwrap(), vec![7u8; 5000]);
        assert_eq!(written.read_file("LevelCache/WLD_Main_A.lsf").unwrap(), big);
        assert_eq!(written.read_file("empty.bin").unwrap(), b"");
        assert_ne!(written.stored_md5(), before);
        assert_eq!(written.compute_md5().unwrap(), written.stored_md5());
    }

    #[test]
    fn peeks_at_named_files_on_disk() {
        let bytes = sample();
        let path = std::env::temp_dir().join(format!("nautiloid-peek-{}.lsv", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let found = peek(&path, &["empty.bin", "missing.lsf", "meta.lsf"]).unwrap();
        assert_eq!(found[0].as_deref(), Some(b"".as_slice()));
        assert_eq!(found[1], None);
        assert_eq!(found[2].as_deref(), Some(b"first file".as_slice()));

        // Not a package, and a package cut short.
        std::fs::write(
            &path,
            b"not a package at all, but long enough to have a header",
        )
        .unwrap();
        assert!(peek(&path, &["meta.lsf"]).is_err());
        std::fs::write(&path, &bytes[..bytes.len() - 9]).unwrap();
        assert!(peek(&path, &["meta.lsf"]).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(peek(&path, &["meta.lsf"]).is_err());
    }

    #[test]
    fn rejects_damaged_packages() {
        let bytes = sample();
        assert!(Package::read(bytes[..30].to_vec()).is_err());

        let mut wrong_magic = bytes.clone();
        wrong_magic[0] = b'X';
        assert!(Package::read(wrong_magic).is_err());

        let mut wrong_version = bytes.clone();
        wrong_version[4] = 15;
        let error = Package::read(wrong_version).err().unwrap().to_string();
        assert!(error.contains("version 15"), "{error}");

        let mut truncated = bytes.clone();
        truncated.truncate(bytes.len() - 5);
        assert!(Package::read(truncated).is_err());

        let mut package = Package::read(bytes).unwrap();
        assert!(package.replace("missing.lsf", b"x").is_err());
    }
}
