//! The `.lsf` files inside a save: a tree of named nodes that carry typed attributes.
//!
//! Layout, as found in every sample save (format version 7):
//!
//! ```text
//! 0   "LSOF"
//! 4   u32  format version (7)
//! 8   u64  engine version
//! 16  five pairs of u32 (uncompressed size, size on disk):
//!     strings, keys, nodes, attributes, values
//! 56  u8   compression flags (0x22: LZ4, default level)
//! 57  u8, u16  unknown, zero
//! 60  u32  metadata format (0)
//! 64  the sections, in this order: strings, nodes, attributes, values, keys
//! ```
//!
//! The strings section is one LZ4 block; the others are LZ4 frames with linked 64 KB blocks and
//! no checksums. A section with a size on disk of zero is stored uncompressed.
//!
//! * **Strings**: the name table. A bucket count, then for each bucket a count and that many
//!   length-prefixed names. A name is referred to as `bucket << 16 | index`.
//! * **Nodes**: 12 bytes each: name, index of the node's first attribute (or -1), index of its
//!   parent (or -1). Parents always come before their children.
//! * **Attributes**: 12 bytes each: name, type in the low 6 bits and length in the rest, node
//!   index. Their order is the order the game wrote them in, which is *not* node order: a
//!   node's attributes can come after its children's and be interleaved with them.
//! * **Values**: the attributes' data, back to back in attribute order.
//! * **Keys**: 8 bytes each: node index, name of the attribute that keys that node in a map.
//!
//! [`Lsf`] keeps nodes and attributes in file order and every value as its original bytes, so
//! writing an unchanged document reproduces each section exactly.

use crate::value::{self, Value};
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::io::{Read, Write};

pub const MAGIC: &[u8; 4] = b"LSOF";
/// The format version the game writes, from Patch 4 to Patch 8.
pub const VERSION: u32 = 7;
/// Version 6 has the same layout. One file of that version was found, a level cache carried
/// over from an older game version inside a Patch 5 save.
const OLDEST_VERSION: u32 = 6;

const HEADER_LEN: usize = 64;
const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4d, 0x18];
/// Compression method in the low four bits of the flags: LZ4.
const METHOD_LZ4: u8 = 2;
/// A section larger than this is taken to be a damaged size field.
const MAX_SECTION_LEN: usize = 1 << 30;

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}
fn i32_at(d: &[u8], at: usize) -> i32 {
    u32_at(d, at) as i32
}
fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

/// The fixed fields at the start of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub engine_version: u64,
    pub compression_flags: u8,
    pub unknown2: u8,
    pub unknown3: u16,
    pub metadata_format: u32,
}

impl Header {
    /// The engine version as `major.minor.revision.build`. Patch 8 saves say 4.8.
    pub fn engine_version_text(&self) -> String {
        let v = self.engine_version;
        format!(
            "{}.{}.{}.{}",
            (v >> 55) & 0x7f,
            (v >> 47) & 0xff,
            (v >> 31) & 0xffff,
            v & 0x7fff_ffff
        )
    }

    /// The engine's major and minor version.
    pub fn engine_major_minor(&self) -> (u32, u32) {
        let v = self.engine_version;
        (((v >> 55) & 0x7f) as u32, ((v >> 47) & 0xff) as u32)
    }
}

/// Which section, in the order their sizes appear in the header.
#[derive(Clone, Copy)]
enum Section {
    Strings = 0,
    Keys = 1,
    Nodes = 2,
    Attributes = 3,
    Values = 4,
}

/// The order the sections sit in the file.
const DISK_ORDER: [Section; 5] = [
    Section::Strings,
    Section::Nodes,
    Section::Attributes,
    Section::Values,
    Section::Keys,
];

/// An `.lsf` file split into its header and uncompressed sections.
#[derive(Clone, PartialEq, Eq)]
pub struct Sections {
    pub header: Header,
    pub strings: Vec<u8>,
    pub nodes: Vec<u8>,
    pub attributes: Vec<u8>,
    pub values: Vec<u8>,
    pub keys: Vec<u8>,
    /// Sections that were stored uncompressed, indexed in header order.
    stored_raw: [bool; 5],
}

fn decode_frame(raw: &[u8], expected: usize, what: &str) -> Result<Vec<u8>> {
    ensure!(
        raw.len() >= 4 && raw[..4] == LZ4_FRAME_MAGIC,
        "{what} section is not an LZ4 frame"
    );
    let mut out = Vec::with_capacity(expected);
    lz4_flex::frame::FrameDecoder::new(raw)
        .take(expected as u64 + 1)
        .read_to_end(&mut out)
        .with_context(|| format!("{what} section"))?;
    Ok(out)
}

fn encode_frame(data: &[u8]) -> Result<Vec<u8>> {
    let mut info = lz4_flex::frame::FrameInfo::new();
    info.block_size = lz4_flex::frame::BlockSize::Max64KB;
    info.block_mode = lz4_flex::frame::BlockMode::Linked;
    let mut encoder = lz4_flex::frame::FrameEncoder::with_frame_info(info, Vec::new());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

impl Sections {
    /// Splits an `.lsf` file and decompresses its sections.
    pub fn read(data: &[u8]) -> Result<Sections> {
        ensure!(data.len() >= HEADER_LEN, "too short to be an LSF file");
        ensure!(&data[0..4] == MAGIC, "not an LSF file (no LSOF signature)");
        let version = u32_at(data, 4);
        ensure!(
            (OLDEST_VERSION..=VERSION).contains(&version),
            "LSF format version {version} is not supported (expected {VERSION})"
        );
        let header = Header {
            version,
            engine_version: u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8])),
            compression_flags: data[56],
            unknown2: data[57],
            unknown3: u16_at(data, 58),
            metadata_format: u32_at(data, 60),
        };
        ensure!(
            header.metadata_format == 0,
            "LSF metadata format {} is not supported",
            header.metadata_format
        );

        let mut out: [Vec<u8>; 5] = Default::default();
        let mut stored_raw = [false; 5];
        let mut pos = HEADER_LEN;
        for section in DISK_ORDER {
            let i = section as usize;
            let what = ["strings", "keys", "nodes", "attributes", "values"][i];
            let uncompressed = u32_at(data, 16 + i * 8) as usize;
            let on_disk = u32_at(data, 20 + i * 8) as usize;
            ensure!(
                uncompressed <= MAX_SECTION_LEN,
                "{what} section: implausible size"
            );
            let len = if on_disk == 0 { uncompressed } else { on_disk };
            ensure!(pos + len <= data.len(), "{what} section runs past the end");
            let raw = &data[pos..pos + len];
            pos += len;
            if uncompressed == 0 {
                ensure!(on_disk == 0, "{what} section: empty but stored");
                continue;
            }
            out[i] = if on_disk == 0 {
                stored_raw[i] = true;
                raw.to_vec()
            } else {
                ensure!(
                    header.compression_flags & 0x0f == METHOD_LZ4,
                    "LSF compression method {} is not supported",
                    header.compression_flags & 0x0f
                );
                match section {
                    Section::Strings => lz4_flex::block::decompress(raw, uncompressed)
                        .map_err(|e| anyhow!("{what} section: {e}"))?,
                    _ => decode_frame(raw, uncompressed, what)?,
                }
            };
            ensure!(
                out[i].len() == uncompressed,
                "{what} section: unpacked to {} bytes, expected {uncompressed}",
                out[i].len()
            );
        }
        ensure!(pos == data.len(), "data after the last section");

        let [strings, keys, nodes, attributes, values] = out;
        Ok(Sections {
            header,
            strings,
            nodes,
            attributes,
            values,
            keys,
            stored_raw,
        })
    }

    /// Compresses the sections and assembles the file.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut out = vec![0u8; HEADER_LEN];
        for section in DISK_ORDER {
            let i = section as usize;
            let data: &[u8] = match section {
                Section::Strings => &self.strings,
                Section::Keys => &self.keys,
                Section::Nodes => &self.nodes,
                Section::Attributes => &self.attributes,
                Section::Values => &self.values,
            };
            ensure!(data.len() <= u32::MAX as usize, "section too large");
            let on_disk = if data.is_empty() {
                0
            } else if self.stored_raw[i] {
                out.extend_from_slice(data);
                0
            } else {
                ensure!(
                    self.header.compression_flags & 0x0f == METHOD_LZ4,
                    "LSF compression method {} is not supported",
                    self.header.compression_flags & 0x0f
                );
                let packed = match section {
                    Section::Strings => lz4_flex::block::compress(data),
                    _ => encode_frame(data)?,
                };
                ensure!(packed.len() <= u32::MAX as usize, "section too large");
                out.extend_from_slice(&packed);
                packed.len()
            };
            out[16 + i * 8..20 + i * 8].copy_from_slice(&(data.len() as u32).to_le_bytes());
            out[20 + i * 8..24 + i * 8].copy_from_slice(&(on_disk as u32).to_le_bytes());
        }
        out[0..4].copy_from_slice(MAGIC);
        out[4..8].copy_from_slice(&self.header.version.to_le_bytes());
        out[8..16].copy_from_slice(&self.header.engine_version.to_le_bytes());
        out[56] = self.header.compression_flags;
        out[57] = self.header.unknown2;
        out[58..60].copy_from_slice(&self.header.unknown3.to_le_bytes());
        out[60..64].copy_from_slice(&self.header.metadata_format.to_le_bytes());
        Ok(out)
    }
}

/// Index of a node, in file order.
pub type NodeId = u32;
/// Index of an attribute, in file order.
pub type AttrId = u32;

/// A reference into the name table: `bucket << 16 | index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NameRef(u32);

#[derive(Clone)]
struct NodeRec {
    name: NameRef,
    /// -1 for a root.
    parent: i32,
}

#[derive(Clone)]
struct AttrRec {
    name: NameRef,
    type_id: u8,
    node: NodeId,
    /// Where the value's bytes are in `Lsf::values`.
    offset: u32,
    len: u32,
}

#[derive(Clone)]
struct KeyRec {
    node: NodeId,
    name: NameRef,
}

/// Lookup tables derived from the node and attribute lists.
#[derive(Clone, Default)]
struct Index {
    roots: Vec<NodeId>,
    /// `children[child_start[n]..child_start[n + 1]]` are node `n`'s children, in file order.
    child_start: Vec<u32>,
    children: Vec<NodeId>,
    /// `node_attrs[attr_start[n]..attr_start[n + 1]]` are node `n`'s attributes, in file order.
    attr_start: Vec<u32>,
    node_attrs: Vec<AttrId>,
}

/// A parsed `.lsf` document.
#[derive(Clone)]
pub struct Lsf {
    pub header: Header,
    stored_raw: [bool; 5],
    /// The name table as read: buckets of names.
    names: Vec<Vec<Box<str>>>,
    nodes: Vec<NodeRec>,
    attrs: Vec<AttrRec>,
    /// Value bytes. Starts as the file's values section; a value that changes length is
    /// appended and its attribute pointed at the new bytes.
    values: Vec<u8>,
    keys: Vec<KeyRec>,
    index: Index,
}

impl Lsf {
    /// Parses an `.lsf` file.
    pub fn parse(data: &[u8]) -> Result<Lsf> {
        Lsf::from_sections(Sections::read(data)?)
    }

    pub fn from_sections(sections: Sections) -> Result<Lsf> {
        let names = parse_names(&sections.strings)?;
        let name_ref = |raw: u32| -> Result<NameRef> {
            let bucket = (raw >> 16) as usize;
            let index = (raw & 0xffff) as usize;
            ensure!(
                names.get(bucket).is_some_and(|b| index < b.len()),
                "reference to a name that isn't in the name table"
            );
            Ok(NameRef(raw))
        };

        ensure!(
            sections.nodes.len().is_multiple_of(12),
            "nodes section has a partial entry"
        );
        let node_count = sections.nodes.len() / 12;
        ensure!(node_count <= i32::MAX as usize, "too many nodes");
        let mut nodes = Vec::with_capacity(node_count);
        let mut first_attrs = Vec::with_capacity(node_count);
        for (i, raw) in sections.nodes.chunks_exact(12).enumerate() {
            let parent = i32_at(raw, 8);
            ensure!(
                parent == -1 || (parent >= 0 && (parent as usize) < i),
                "node {i} comes before its parent"
            );
            nodes.push(NodeRec {
                name: name_ref(u32_at(raw, 0))?,
                parent,
            });
            first_attrs.push(i32_at(raw, 4));
        }

        ensure!(
            sections.attributes.len().is_multiple_of(12),
            "attributes section has a partial entry"
        );
        let attr_count = sections.attributes.len() / 12;
        ensure!(attr_count <= i32::MAX as usize, "too many attributes");
        let mut attrs = Vec::with_capacity(attr_count);
        let mut offset = 0usize;
        for (i, raw) in sections.attributes.chunks_exact(12).enumerate() {
            let type_and_len = u32_at(raw, 4);
            let len = (type_and_len >> 6) as usize;
            let node = i32_at(raw, 8);
            ensure!(
                node >= 0 && (node as usize) < node_count,
                "attribute {i} belongs to a node that doesn't exist"
            );
            ensure!(
                offset + len <= sections.values.len(),
                "attribute {i} runs past the values section"
            );
            attrs.push(AttrRec {
                name: name_ref(u32_at(raw, 0))?,
                type_id: (type_and_len & 0x3f) as u8,
                node: node as NodeId,
                offset: offset as u32,
                len: len as u32,
            });
            offset += len;
        }
        ensure!(
            offset == sections.values.len(),
            "values section has {} bytes that no attribute uses",
            sections.values.len() - offset
        );

        ensure!(
            sections.keys.len().is_multiple_of(8),
            "keys section has a partial entry"
        );
        let mut keys = Vec::with_capacity(sections.keys.len() / 8);
        for raw in sections.keys.chunks_exact(8) {
            let node = u32_at(raw, 0);
            ensure!(
                (node as usize) < node_count,
                "key for a node that doesn't exist"
            );
            keys.push(KeyRec {
                node,
                name: name_ref(u32_at(raw, 4))?,
            });
        }

        let mut lsf = Lsf {
            header: sections.header,
            stored_raw: sections.stored_raw,
            names,
            nodes,
            attrs,
            values: sections.values,
            keys,
            index: Index::default(),
        };
        lsf.rebuild_index();
        // The writer derives each node's first attribute; it must agree with what was read.
        for (i, &first) in first_attrs.iter().enumerate() {
            let derived = lsf.attrs(i as NodeId).first().map_or(-1, |&a| a as i32);
            ensure!(
                first == derived,
                "node {i} points at attribute {first}, expected {derived}"
            );
        }
        Ok(lsf)
    }

    fn rebuild_index(&mut self) {
        let n = self.nodes.len();
        let mut index = Index {
            child_start: vec![0; n + 1],
            attr_start: vec![0; n + 1],
            ..Index::default()
        };
        for (i, node) in self.nodes.iter().enumerate() {
            if node.parent < 0 {
                index.roots.push(i as NodeId);
            } else {
                index.child_start[node.parent as usize + 1] += 1;
            }
        }
        for attr in &self.attrs {
            index.attr_start[attr.node as usize + 1] += 1;
        }
        for i in 0..n {
            index.child_start[i + 1] += index.child_start[i];
            index.attr_start[i + 1] += index.attr_start[i];
        }
        index.children = vec![0; index.child_start[n] as usize];
        index.node_attrs = vec![0; index.attr_start[n] as usize];
        let mut next_child = index.child_start.clone();
        for (i, node) in self.nodes.iter().enumerate() {
            if node.parent >= 0 {
                let slot = &mut next_child[node.parent as usize];
                index.children[*slot as usize] = i as NodeId;
                *slot += 1;
            }
        }
        let mut next_attr = index.attr_start.clone();
        for (i, attr) in self.attrs.iter().enumerate() {
            let slot = &mut next_attr[attr.node as usize];
            index.node_attrs[*slot as usize] = i as AttrId;
            *slot += 1;
        }
        self.index = index;
    }

    /// The document as uncompressed sections.
    pub fn to_sections(&self) -> Result<Sections> {
        let mut strings = Vec::new();
        strings.extend_from_slice(&(self.names.len() as u32).to_le_bytes());
        for bucket in &self.names {
            strings.extend_from_slice(&(bucket.len() as u16).to_le_bytes());
            for name in bucket {
                strings.extend_from_slice(&(name.len() as u16).to_le_bytes());
                strings.extend_from_slice(name.as_bytes());
            }
        }

        let mut nodes = Vec::with_capacity(self.nodes.len() * 12);
        for (i, node) in self.nodes.iter().enumerate() {
            let first = self.attrs(i as NodeId).first().map_or(-1, |&a| a as i32);
            nodes.extend_from_slice(&node.name.0.to_le_bytes());
            nodes.extend_from_slice(&first.to_le_bytes());
            nodes.extend_from_slice(&node.parent.to_le_bytes());
        }

        let mut attributes = Vec::with_capacity(self.attrs.len() * 12);
        let mut values = Vec::with_capacity(self.values.len());
        for attr in &self.attrs {
            ensure!(attr.len < 1 << 26, "attribute value too long");
            attributes.extend_from_slice(&attr.name.0.to_le_bytes());
            attributes.extend_from_slice(&(attr.type_id as u32 | attr.len << 6).to_le_bytes());
            attributes.extend_from_slice(&(attr.node as i32).to_le_bytes());
            let start = attr.offset as usize;
            values.extend_from_slice(&self.values[start..start + attr.len as usize]);
        }
        ensure!(
            values.len() <= u32::MAX as usize,
            "values section too large"
        );

        let mut keys = Vec::with_capacity(self.keys.len() * 8);
        for key in &self.keys {
            keys.extend_from_slice(&key.node.to_le_bytes());
            keys.extend_from_slice(&key.name.0.to_le_bytes());
        }

        Ok(Sections {
            header: self.header.clone(),
            strings,
            nodes,
            attributes,
            values,
            keys,
            stored_raw: self.stored_raw,
        })
    }

    /// Serializes the document as an `.lsf` file.
    pub fn write(&self) -> Result<Vec<u8>> {
        self.to_sections()?.write()
    }

    fn name(&self, name: NameRef) -> &str {
        &self.names[(name.0 >> 16) as usize][(name.0 & 0xffff) as usize]
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn attr_count(&self) -> usize {
        self.attrs.len()
    }

    /// Number of distinct names in the name table.
    pub fn name_count(&self) -> usize {
        self.names.iter().map(Vec::len).sum()
    }

    /// The top-level nodes (regions), in file order.
    pub fn roots(&self) -> &[NodeId] {
        &self.index.roots
    }

    /// The first top-level node called `name`.
    pub fn root(&self, name: &str) -> Option<NodeId> {
        self.roots()
            .iter()
            .copied()
            .find(|&n| self.node_name(n) == name)
    }

    pub fn node_name(&self, node: NodeId) -> &str {
        self.name(self.nodes[node as usize].name)
    }

    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        let parent = self.nodes[node as usize].parent;
        (parent >= 0).then_some(parent as NodeId)
    }

    /// The node's children, in file order.
    pub fn children(&self, node: NodeId) -> &[NodeId] {
        let i = node as usize;
        &self.index.children
            [self.index.child_start[i] as usize..self.index.child_start[i + 1] as usize]
    }

    /// The node's children called `name`, in file order.
    pub fn children_named<'a>(
        &'a self,
        node: NodeId,
        name: &'a str,
    ) -> impl Iterator<Item = NodeId> + 'a {
        self.children(node)
            .iter()
            .copied()
            .filter(move |&c| self.node_name(c) == name)
    }

    /// The node's first child called `name`.
    pub fn child(&self, node: NodeId, name: &str) -> Option<NodeId> {
        self.children_named(node, name).next()
    }

    /// Follows a chain of first-children by name, starting at a root.
    pub fn find(&self, path: &[&str]) -> Option<NodeId> {
        let (first, rest) = path.split_first()?;
        let mut node = self.root(first)?;
        for name in rest {
            node = self.child(node, name)?;
        }
        Some(node)
    }

    /// The node's attributes, in file order.
    pub fn attrs(&self, node: NodeId) -> &[AttrId] {
        let i = node as usize;
        &self.index.node_attrs
            [self.index.attr_start[i] as usize..self.index.attr_start[i + 1] as usize]
    }

    /// The node's attribute called `name`.
    pub fn attr(&self, node: NodeId, name: &str) -> Option<AttrId> {
        self.attrs(node)
            .iter()
            .copied()
            .find(|&a| self.attr_name(a) == name)
    }

    pub fn attr_name(&self, attr: AttrId) -> &str {
        self.name(self.attrs[attr as usize].name)
    }

    /// The attribute's type tag; see [`value::DataType`].
    pub fn attr_type(&self, attr: AttrId) -> u8 {
        self.attrs[attr as usize].type_id
    }

    /// The node the attribute belongs to.
    pub fn attr_node(&self, attr: AttrId) -> NodeId {
        self.attrs[attr as usize].node
    }

    /// The attribute's value as stored.
    pub fn attr_bytes(&self, attr: AttrId) -> &[u8] {
        let rec = &self.attrs[attr as usize];
        &self.values[rec.offset as usize..(rec.offset + rec.len) as usize]
    }

    /// The attribute's decoded value.
    pub fn value(&self, attr: AttrId) -> Result<Value> {
        value::decode(self.attr_type(attr), self.attr_bytes(attr))
    }

    /// The decoded value of the node's attribute called `name`, if it has one that decodes.
    pub fn get(&self, node: NodeId, name: &str) -> Option<Value> {
        self.value(self.attr(node, name)?).ok()
    }

    /// The name of the attribute that keys this node in a map, if the file records one.
    pub fn key_attribute(&self, node: NodeId) -> Option<&str> {
        // Keys are few (a handful to a few thousand) and sorted by node.
        let i = self.keys.binary_search_by_key(&node, |k| k.node).ok()?;
        Some(self.name(self.keys[i].name))
    }

    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Replaces the attribute's value with raw bytes. The type stays as it is.
    pub fn set_bytes(&mut self, attr: AttrId, bytes: &[u8]) -> Result<()> {
        ensure!(bytes.len() < 1 << 26, "attribute value too long");
        let rec = &mut self.attrs[attr as usize];
        if bytes.len() == rec.len as usize {
            let start = rec.offset as usize;
            self.values[start..start + bytes.len()].copy_from_slice(bytes);
        } else {
            ensure!(
                self.values.len() + bytes.len() <= u32::MAX as usize,
                "values section too large"
            );
            rec.offset = self.values.len() as u32;
            rec.len = bytes.len() as u32;
            self.values.extend_from_slice(bytes);
        }
        Ok(())
    }

    /// Replaces the attribute's value. The value must be of the attribute's type.
    pub fn set_value(&mut self, attr: AttrId, value: &Value) -> Result<()> {
        let bytes = value::encode(self.attr_type(attr), value)
            .with_context(|| self.attr_name(attr).to_owned())?;
        self.set_bytes(attr, &bytes)
    }

    /// Removes the given nodes, with everything under them and all their attributes. The
    /// remaining nodes keep their order and are numbered again, so `NodeId`s held from before
    /// no longer apply. Names that are no longer used stay in the name table, as harmless as
    /// value bytes no attribute points at.
    pub fn remove_nodes(&mut self, nodes: &[NodeId]) -> Result<()> {
        let count = self.nodes.len();
        let mut removed = vec![false; count];
        for &node in nodes {
            ensure!((node as usize) < count, "there is no node {node}");
            removed[node as usize] = true;
        }
        // Parents come before their children, so one pass reaches every descendant.
        for i in 0..count {
            let parent = self.nodes[i].parent;
            if parent >= 0 && removed[parent as usize] {
                removed[i] = true;
            }
        }
        let mut new_index = vec![u32::MAX; count];
        let mut next = 0u32;
        for i in 0..count {
            if !removed[i] {
                new_index[i] = next;
                next += 1;
            }
        }
        let mut kept = Vec::with_capacity(next as usize);
        for (i, node) in self.nodes.iter().enumerate() {
            if !removed[i] {
                let mut node = node.clone();
                if node.parent >= 0 {
                    node.parent = new_index[node.parent as usize] as i32;
                }
                kept.push(node);
            }
        }
        self.nodes = kept;
        self.attrs.retain(|a| !removed[a.node as usize]);
        for attr in &mut self.attrs {
            attr.node = new_index[attr.node as usize];
        }
        self.keys.retain(|k| !removed[k.node as usize]);
        for key in &mut self.keys {
            key.node = new_index[key.node as usize];
        }
        self.rebuild_index();
        Ok(())
    }

    /// Where the node is, for people: `Root/Child[2]/Name`, with an index wherever a node has
    /// siblings of the same name.
    pub fn path(&self, node: NodeId) -> String {
        let mut parts = Vec::new();
        let mut current = Some(node);
        while let Some(n) = current {
            let name = self.node_name(n);
            let siblings = match self.parent(n) {
                Some(p) => self.children(p),
                None => self.roots(),
            };
            let mut same = siblings.iter().filter(|&&s| self.node_name(s) == name);
            let position = same.clone().position(|&s| s == n).unwrap_or(0);
            parts.push(if same.nth(1).is_some() {
                format!("{name}[{position}]")
            } else {
                name.to_owned()
            });
            current = self.parent(n);
        }
        parts.reverse();
        parts.join("/")
    }
}

fn parse_names(strings: &[u8]) -> Result<Vec<Vec<Box<str>>>> {
    let mut pos = 0usize;
    let mut take = |len: usize| -> Result<&[u8]> {
        ensure!(pos + len <= strings.len(), "name table is truncated");
        let slice = &strings[pos..pos + len];
        pos += len;
        Ok(slice)
    };
    let bucket_count = u32_at(take(4)?, 0) as usize;
    ensure!(bucket_count <= 0x1_0000, "name table has too many buckets");
    let mut names = Vec::with_capacity(bucket_count);
    for _ in 0..bucket_count {
        let count = u16_at(take(2)?, 0) as usize;
        let mut bucket = Vec::with_capacity(count);
        for _ in 0..count {
            let len = u16_at(take(2)?, 0) as usize;
            let name = std::str::from_utf8(take(len)?).context("a name is not UTF-8")?;
            bucket.push(Box::<str>::from(name));
        }
        names.push(bucket);
    }
    if pos != strings.len() {
        bail!("data after the name table");
    }
    Ok(names)
}

/// Builds documents from scratch. Used by tests, which have no real save to start from.
pub mod build {
    use super::*;
    use crate::value::DataType;

    /// A node to be written: its name, attributes and children.
    #[derive(Clone, Default)]
    pub struct Node {
        pub name: String,
        pub attrs: Vec<(String, DataType, Value)>,
        pub children: Vec<Node>,
    }

    impl Node {
        pub fn new(name: &str) -> Node {
            Node {
                name: name.to_owned(),
                ..Node::default()
            }
        }

        pub fn attr(mut self, name: &str, data_type: DataType, value: Value) -> Node {
            self.attrs.push((name.to_owned(), data_type, value));
            self
        }

        pub fn child(mut self, child: Node) -> Node {
            self.children.push(child);
            self
        }
    }

    struct Builder {
        names: Vec<Vec<Box<str>>>,
        nodes: Vec<u8>,
        attributes: Vec<u8>,
        values: Vec<u8>,
        node_count: i32,
        attr_count: i32,
    }

    impl Builder {
        fn name(&mut self, name: &str) -> u32 {
            let bucket = name
                .bytes()
                .fold(0usize, |h, b| (h * 31 + b as usize) % 512);
            let index = match self.names[bucket].iter().position(|n| &**n == name) {
                Some(index) => index,
                None => {
                    self.names[bucket].push(name.into());
                    self.names[bucket].len() - 1
                }
            };
            (bucket as u32) << 16 | index as u32
        }

        fn node(&mut self, node: &Node, parent: i32) -> Result<()> {
            let id = self.node_count;
            self.node_count += 1;
            let name = self.name(&node.name);
            let first = if node.attrs.is_empty() {
                -1
            } else {
                self.attr_count
            };
            self.nodes.extend_from_slice(&name.to_le_bytes());
            self.nodes.extend_from_slice(&first.to_le_bytes());
            self.nodes.extend_from_slice(&parent.to_le_bytes());
            for (attr_name, data_type, value) in &node.attrs {
                let bytes = value::encode(data_type.id(), value)?;
                let name = self.name(attr_name);
                self.attributes.extend_from_slice(&name.to_le_bytes());
                self.attributes.extend_from_slice(
                    &(data_type.id() as u32 | (bytes.len() as u32) << 6).to_le_bytes(),
                );
                self.attributes.extend_from_slice(&id.to_le_bytes());
                self.values.extend_from_slice(&bytes);
                self.attr_count += 1;
            }
            for child in &node.children {
                self.node(child, id)?;
            }
            Ok(())
        }
    }

    /// Serializes `roots` as an `.lsf` file with the header of a Patch 8 save.
    pub fn lsf(roots: &[Node]) -> Result<Vec<u8>> {
        let mut builder = Builder {
            names: vec![Vec::new(); 512],
            nodes: Vec::new(),
            attributes: Vec::new(),
            values: Vec::new(),
            node_count: 0,
            attr_count: 0,
        };
        for root in roots {
            builder.node(root, -1)?;
        }
        let mut strings = Vec::new();
        strings.extend_from_slice(&(builder.names.len() as u32).to_le_bytes());
        for bucket in &builder.names {
            strings.extend_from_slice(&(bucket.len() as u16).to_le_bytes());
            for name in bucket {
                strings.extend_from_slice(&(name.len() as u16).to_le_bytes());
                strings.extend_from_slice(name.as_bytes());
            }
        }
        Sections {
            header: Header {
                version: VERSION,
                // 4.8.0.700, as written by game build 4.1.1.7631656.
                engine_version: 0x0204_0000_0000_02bc,
                compression_flags: 0x22,
                unknown2: 0,
                unknown3: 0,
                metadata_format: 0,
            },
            strings,
            nodes: builder.nodes,
            attributes: builder.attributes,
            values: builder.values,
            keys: Vec::new(),
            stored_raw: [false; 5],
        }
        .write()
    }
}

#[cfg(test)]
mod tests {
    use super::build::Node;
    use super::*;
    use crate::value::DataType;

    fn sample() -> Vec<u8> {
        let mods = Node::new("Mods")
            .child(
                Node::new("ModuleShortDesc")
                    .attr("Folder", DataType::LSString, Value::Text("GustavX".into()))
                    .attr("Version64", DataType::Int64, Value::Int64(1 << 40)),
            )
            .child(
                Node::new("ModuleShortDesc")
                    .attr("Folder", DataType::LSString, Value::Text("Honour".into()))
                    .attr("Version64", DataType::Int64, Value::Int64(7)),
            );
        let meta = Node::new("MetaData")
            .attr("Difficulty", DataType::UInt, Value::UInt(2))
            .attr(
                "LeaderName",
                DataType::LSString,
                Value::Text("Nelthryiis".into()),
            )
            .child(Node::new("ModuleSettings").child(mods));
        build::lsf(&[meta, Node::new("Empty")]).unwrap()
    }

    #[test]
    fn frames_use_the_games_settings() {
        // Magic, then FLG 0x40 (linked blocks, no checksums or content size), BD 0x40 (64 KB).
        let frame = encode_frame(&[1, 2, 3, 4]).unwrap();
        assert_eq!(frame[..6], [0x04, 0x22, 0x4d, 0x18, 0x40, 0x40]);
        let big: Vec<u8> = (0..300_000u32)
            .flat_map(|i| (i / 7).to_le_bytes())
            .collect();
        let frame = encode_frame(&big).unwrap();
        assert_eq!(decode_frame(&frame, big.len(), "test").unwrap(), big);
    }

    #[test]
    fn reads_what_was_built() {
        let lsf = Lsf::parse(&sample()).unwrap();
        assert_eq!(lsf.header.engine_version_text(), "4.8.0.700");
        assert_eq!(lsf.header.engine_major_minor(), (4, 8));
        assert_eq!(lsf.node_count(), 6);
        assert_eq!(lsf.attr_count(), 6);
        assert_eq!(lsf.roots().len(), 2);

        let meta = lsf.root("MetaData").unwrap();
        assert_eq!(lsf.get(meta, "Difficulty"), Some(Value::UInt(2)));
        assert_eq!(lsf.get(meta, "Missing"), None);
        let mods = lsf.find(&["MetaData", "ModuleSettings", "Mods"]).unwrap();
        let folders: Vec<_> = lsf
            .children_named(mods, "ModuleShortDesc")
            .map(|m| lsf.get(m, "Folder").unwrap().as_str().unwrap().to_owned())
            .collect();
        assert_eq!(folders, ["GustavX", "Honour"]);
        let second = lsf.children(mods)[1];
        assert_eq!(
            lsf.path(second),
            "MetaData/ModuleSettings/Mods/ModuleShortDesc[1]"
        );
        assert_eq!(lsf.path(mods), "MetaData/ModuleSettings/Mods");
        assert_eq!(lsf.parent(second), Some(mods));
        assert_eq!(lsf.parent(meta), None);
        assert!(lsf.children(lsf.root("Empty").unwrap()).is_empty());
    }

    #[test]
    fn unchanged_document_writes_identical_sections() {
        let bytes = sample();
        let original = Sections::read(&bytes).unwrap();
        let lsf = Lsf::parse(&bytes).unwrap();
        assert!(lsf.to_sections().unwrap() == original);
        // And the file it writes reads back the same.
        assert!(Sections::read(&lsf.write().unwrap()).unwrap() == original);
    }

    #[test]
    fn edits_survive_a_write() {
        let mut lsf = Lsf::parse(&sample()).unwrap();
        let meta = lsf.root("MetaData").unwrap();
        let difficulty = lsf.attr(meta, "Difficulty").unwrap();
        let leader = lsf.attr(meta, "LeaderName").unwrap();
        lsf.set_value(difficulty, &Value::UInt(1)).unwrap();
        // A longer and then a shorter string: both change the value's length.
        lsf.set_value(leader, &Value::Text("A much longer leader name".into()))
            .unwrap();
        lsf.set_value(leader, &Value::Text("Tav".into())).unwrap();
        assert!(lsf.set_value(difficulty, &Value::Int(1)).is_err());

        let again = Lsf::parse(&lsf.write().unwrap()).unwrap();
        let meta = again.root("MetaData").unwrap();
        assert_eq!(again.get(meta, "Difficulty"), Some(Value::UInt(1)));
        assert_eq!(
            again.get(meta, "LeaderName"),
            Some(Value::Text("Tav".into()))
        );
        // Everything else is untouched.
        let mods = again.find(&["MetaData", "ModuleSettings", "Mods"]).unwrap();
        let last = again.children(mods)[1];
        assert_eq!(again.get(last, "Version64"), Some(Value::Int64(7)));
        assert_eq!(
            again.get(last, "Folder"),
            Some(Value::Text("Honour".into()))
        );
    }

    #[test]
    fn removed_nodes_go_with_what_is_under_them() {
        let mut lsf = Lsf::parse(&sample()).unwrap();
        let mods = lsf.find(&["MetaData", "ModuleSettings", "Mods"]).unwrap();
        let first = lsf.children(mods)[0];
        lsf.remove_nodes(&[first]).unwrap();

        let again = Lsf::parse(&lsf.write().unwrap()).unwrap();
        assert_eq!(again.node_count(), 5);
        assert_eq!(again.attr_count(), 4);
        let mods = again.find(&["MetaData", "ModuleSettings", "Mods"]).unwrap();
        let left: Vec<_> = again
            .children(mods)
            .iter()
            .map(|&m| again.get(m, "Folder").unwrap())
            .collect();
        assert_eq!(left, [Value::Text("Honour".into())]);
        let meta = again.root("MetaData").unwrap();
        assert_eq!(
            again.get(meta, "LeaderName"),
            Some(Value::Text("Nelthryiis".into()))
        );
        assert!(again.root("Empty").is_some());

        // Removing a node removes everything below it.
        let settings = again.find(&["MetaData", "ModuleSettings"]).unwrap();
        let mut again = again;
        again.remove_nodes(&[settings]).unwrap();
        let mut again = Lsf::parse(&again.write().unwrap()).unwrap();
        assert_eq!(again.node_count(), 2);
        assert_eq!(again.attr_count(), 2);
        assert!(again.remove_nodes(&[9]).is_err());
    }

    #[test]
    fn rejects_damaged_files() {
        let bytes = sample();
        assert!(Lsf::parse(&bytes[..40]).is_err());
        let mut wrong_magic = bytes.clone();
        wrong_magic[0] = b'X';
        assert!(Lsf::parse(&wrong_magic).is_err());
        let mut wrong_version = bytes.clone();
        wrong_version[4] = 8;
        let error = Lsf::parse(&wrong_version).err().unwrap().to_string();
        assert!(error.contains("version 8"), "{error}");
        let mut truncated = bytes.clone();
        truncated.truncate(bytes.len() - 3);
        assert!(Lsf::parse(&truncated).is_err());

        // A node that names a parent after itself.
        let mut sections = Sections::read(&bytes).unwrap();
        sections.nodes[8..12].copy_from_slice(&5i32.to_le_bytes());
        assert!(Lsf::from_sections(sections).is_err());
        // An attribute whose length runs past the values.
        let mut sections = Sections::read(&bytes).unwrap();
        sections.values.truncate(sections.values.len() - 1);
        assert!(Lsf::from_sections(sections).is_err());
        // A name reference outside the table.
        let mut sections = Sections::read(&bytes).unwrap();
        sections.nodes[0..4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        assert!(Lsf::from_sections(sections).is_err());
    }
}
