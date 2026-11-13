//! The `NewAge` blob: where the game keeps its entity data.
//!
//! `Globals.lsf` and every level cache have a top-level `NewAge` node whose one attribute is a
//! byte buffer of several megabytes. Party members' experience, level and ability scores,
//! approval ratings, camp supplies and inventory stacks are in there, not in the node tree.
//!
//! The buffer is a memory image: tables of fixed-size rows, with 64-bit offsets where the game
//! had pointers. Layout, as found in every sample save:
//!
//! ```text
//! 0   "LSMF"
//! 4   01 01 00 08
//! 8   u64  checksum of everything after the header
//! 16  u64  size of the data section
//! 24  u64  size of the directory (names, then table records)
//! 32  u32  size of the names
//! 36  u16  number of tables
//! 38  u16  32
//! 40  u64  0
//! 48  the data section; every offset below is relative to its start
//! ..  the names, then one 48-byte record per table
//! ```
//!
//! The checksum is the first half of MurmurHash3 x64_128 over bytes 48 to the end, with both
//! halves of the state seeded with `0xf0015eed5eedf001`. The game checks it: with one bit of
//! the checksum or one unused byte of the data or directory changed, a save loads to 100% and
//! then drops to the title screen with "Server shutting down". The function and seed are from
//! the SaveForge project's notes; they reproduce the stored value of every blob in the samples.
//!
//! A table record is: name offset (u64) and length (u32), four unused bytes, a hash that is
//! the same for a given table in every save (u64), row length (u32), version (u32, matching
//! the `vN` in the name), row count (u64) and offset (u64). Unused bytes here and in the data
//! hold whatever was in memory, and they count towards the checksum like any others.
//!
//! Table 0, `core.v0.Level`, has one row of four u64: where the entity list starts, where it
//! ends, the offset of the owner map and the offset of the data that follows the map.
//! Table 1, `core.v0.EntityId`, is 16 bytes per row. It starts with the entity list, one row
//! per entity: the same identifier the node tree stores as `Entity` under
//! `Characters/CharacterFactory/Creators/Creator` (and the item equivalent), which is how a
//! character is tied to its rows here. The rest of the table is identifiers that other rows
//! point at when they refer to an entity, so the same identifier appears there many times.
//!
//! The owner map is 514 slots of 32 bytes: begin, end, table index, count. A slot whose begin
//! is all ones is empty. A used slot says that `begin..end` holds `count` u32 entity indices,
//! one per row of that table: row `i` belongs to entity `owners[i]`. Tables without a slot are
//! not components; their rows are pointed at from other rows.
//!
//! Only edits that keep a row's size are supported: they overwrite bytes in place and leave
//! every offset valid. The checksum is recomputed whenever the blob is written out.

use crate::lsf::{AttrId, Lsf};
use crate::value::Uuid;
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::collections::HashMap;

pub const MAGIC: &[u8; 4] = b"LSMF";
/// The bytes after the magic in every sample, Patch 4 to Patch 8.
const VERSION: [u8; 4] = [1, 1, 0, 8];
const HEADER_LEN: usize = 48;
const RECORD_LEN: usize = 48;
const SLOT_LEN: usize = 32;
const LEVEL_TABLE: &str = "core.v0.Level";
const ENTITY_TABLE: &str = "core.v0.EntityId";
/// Name of the node and of its attribute that hold the blob.
pub const NODE: &str = "NewAge";
/// Seed for both halves of the checksum's state.
const CHECKSUM_SEED: u64 = 0xf001_5eed_5eed_f001;

/// MurmurHash3 x64_128, with one 64-bit seed for both halves of the state. Returns `(h1, h2)`.
/// With a seed that fits in 32 bits this is the reference function.
fn murmur3_x64_128(data: &[u8], seed: u64) -> (u64, u64) {
    const C1: u64 = 0x87c3_7b91_1142_53d5;
    const C2: u64 = 0x4cf5_ad43_2745_937f;
    fn mix1(k: u64) -> u64 {
        k.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2)
    }
    fn mix2(k: u64) -> u64 {
        k.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1)
    }
    fn finish(mut k: u64) -> u64 {
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        k ^ (k >> 33)
    }
    /// Up to eight bytes as a little-endian number.
    fn partial(bytes: &[u8]) -> u64 {
        let mut word = [0u8; 8];
        word[..bytes.len()].copy_from_slice(bytes);
        u64::from_le_bytes(word)
    }

    let (mut h1, mut h2) = (seed, seed);
    let mut blocks = data.chunks_exact(16);
    for block in &mut blocks {
        h1 ^= mix1(u64_at(block, 0));
        h1 = h1
            .rotate_left(27)
            .wrapping_add(h2)
            .wrapping_mul(5)
            .wrapping_add(0x52dc_e729);
        h2 ^= mix2(u64_at(block, 8));
        h2 = h2
            .rotate_left(31)
            .wrapping_add(h1)
            .wrapping_mul(5)
            .wrapping_add(0x3849_5ab5);
    }
    let tail = blocks.remainder();
    if tail.len() > 8 {
        h2 ^= mix2(partial(&tail[8..]));
    }
    if !tail.is_empty() {
        h1 ^= mix1(partial(&tail[..tail.len().min(8)]));
    }
    h1 ^= data.len() as u64;
    h2 ^= data.len() as u64;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = finish(h1);
    h2 = finish(h2);
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    (h1, h2)
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}
fn u64_at(d: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[at..at + 8]);
    u64::from_le_bytes(b)
}

/// Index of a table in the blob's directory.
pub type TableId = usize;

/// One table: an array of fixed-size rows.
#[derive(Clone, Debug)]
pub struct Table {
    /// For example `game.experience.v0.ExperienceComponent`.
    pub name: String,
    /// Identifies the table's layout; the same in every save for a given name.
    pub schema: u64,
    pub version: u32,
    /// Bytes per row.
    pub row_len: usize,
    pub rows: usize,
    /// Where the first row is in the blob.
    start: usize,
    /// Where the row owners' entity indices are in the blob, for a component table.
    owners: Option<usize>,
}

impl Table {
    /// Whether rows belong to entities. Other tables hold data that rows point at.
    pub fn is_component(&self) -> bool {
        self.owners.is_some()
    }
}

/// A parsed blob. It keeps the bytes it was read from; edits change them in place.
#[derive(Clone)]
pub struct NewAge {
    /// The blob. After an edit the checksum in it is out of date until it is written out.
    bytes: Vec<u8>,
    tables: Vec<Table>,
    by_name: HashMap<String, TableId>,
    entities: TableId,
    /// Rows at the start of the entity table that are the entity list.
    entity_count: usize,
    by_entity: HashMap<[u8; 16], u32>,
}

impl NewAge {
    /// The attribute that holds the blob in a document, if it has one.
    pub fn attribute(lsf: &Lsf) -> Option<AttrId> {
        lsf.attr(lsf.root(NODE)?, NODE)
    }

    /// Parses the blob stored in `lsf`.
    pub fn from_lsf(lsf: &Lsf) -> Result<NewAge> {
        let attr =
            NewAge::attribute(lsf).ok_or_else(|| anyhow!("the document has no NewAge data"))?;
        NewAge::parse(lsf.attr_bytes(attr).to_vec())
    }

    pub fn parse(bytes: Vec<u8>) -> Result<NewAge> {
        ensure!(bytes.len() >= HEADER_LEN, "too short to be NewAge data");
        ensure!(&bytes[0..4] == MAGIC, "not NewAge data (no LSMF signature)");
        ensure!(
            bytes[4..8] == VERSION,
            "NewAge version {:02x?} is not supported",
            &bytes[4..8]
        );
        let data_len = usize::try_from(u64_at(&bytes, 16)).context("data size")?;
        let directory_len = usize::try_from(u64_at(&bytes, 24)).context("directory size")?;
        let names_len = u32_at(&bytes, 32) as usize;
        let table_count = u16::from_le_bytes([bytes[36], bytes[37]]) as usize;
        ensure!(
            data_len
                .checked_add(directory_len)
                .and_then(|n| n.checked_add(HEADER_LEN))
                == Some(bytes.len()),
            "NewAge sizes don't add up to the buffer's length"
        );
        ensure!(
            names_len + table_count * RECORD_LEN == directory_len,
            "NewAge directory size doesn't match its table count"
        );
        let data = &bytes[HEADER_LEN..HEADER_LEN + data_len];
        let names = &bytes[HEADER_LEN + data_len..HEADER_LEN + data_len + names_len];
        let records = &bytes[HEADER_LEN + data_len + names_len..];

        let mut tables = Vec::with_capacity(table_count);
        let mut by_name = HashMap::with_capacity(table_count);
        for (i, record) in records.chunks_exact(RECORD_LEN).enumerate() {
            let name_at = usize::try_from(u64_at(record, 0)).context("name offset")?;
            let name_len = u32_at(record, 8) as usize;
            ensure!(
                name_at
                    .checked_add(name_len)
                    .is_some_and(|end| end <= names.len()),
                "table {i}: name is outside the name list"
            );
            let name = std::str::from_utf8(&names[name_at..name_at + name_len])
                .with_context(|| format!("table {i}: name"))?
                .to_owned();
            let row_len = u32_at(record, 24) as usize;
            let rows = usize::try_from(u64_at(record, 32)).context("row count")?;
            let offset = usize::try_from(u64_at(record, 40)).context("table offset")?;
            ensure!(row_len > 0, "{name}: rows of no length");
            ensure!(
                rows.checked_mul(row_len)
                    .and_then(|len| len.checked_add(offset))
                    .is_some_and(|end| end <= data.len()),
                "{name}: runs past the data section"
            );
            ensure!(
                by_name.insert(name.clone(), i).is_none(),
                "{name}: listed twice"
            );
            tables.push(Table {
                name,
                schema: u64_at(record, 16),
                version: u32_at(record, 28),
                row_len,
                rows,
                start: HEADER_LEN + offset,
                owners: None,
            });
        }

        let level = *by_name
            .get(LEVEL_TABLE)
            .ok_or_else(|| anyhow!("NewAge data has no {LEVEL_TABLE} table"))?;
        let entities = *by_name
            .get(ENTITY_TABLE)
            .ok_or_else(|| anyhow!("NewAge data has no {ENTITY_TABLE} table"))?;
        ensure!(
            tables[level].rows == 1 && tables[level].row_len == 32,
            "{LEVEL_TABLE}: unexpected shape"
        );
        ensure!(
            tables[entities].row_len == 16,
            "{ENTITY_TABLE}: unexpected shape"
        );
        let level_row = tables[level].start;
        let entity_offset = u64_at(&bytes, level_row) as usize;
        let entity_end = usize::try_from(u64_at(&bytes, level_row + 8)).context("entity list")?;
        let map_offset = usize::try_from(u64_at(&bytes, level_row + 16)).context("owner map")?;
        let map_end = usize::try_from(u64_at(&bytes, level_row + 24)).context("owner map")?;
        ensure!(
            HEADER_LEN + entity_offset == tables[entities].start,
            "{LEVEL_TABLE} doesn't point at {ENTITY_TABLE}"
        );
        ensure!(
            entity_end >= entity_offset
                && (entity_end - entity_offset).is_multiple_of(16)
                && (entity_end - entity_offset) / 16 <= tables[entities].rows,
            "the entity list is not where {LEVEL_TABLE} says"
        );
        ensure!(
            map_offset <= map_end
                && map_end <= data.len()
                && (map_end - map_offset) % SLOT_LEN == 0,
            "the owner map is not where {LEVEL_TABLE} says"
        );

        let entity_count = (entity_end - entity_offset) / 16;
        for slot in data[map_offset..map_end].chunks_exact(SLOT_LEN) {
            let begin = u64_at(slot, 0);
            if begin == u64::MAX {
                continue;
            }
            let (begin, end) = (begin as usize, u64_at(slot, 8) as usize);
            let table = usize::try_from(u64_at(slot, 16)).context("owner map")?;
            let count = usize::try_from(u64_at(slot, 24)).context("owner map")?;
            ensure!(table < tables.len(), "the owner map names table {table}");
            let entry = &mut tables[table];
            ensure!(
                count == entry.rows
                    && begin <= end
                    && end <= data.len()
                    && end - begin == count * 4,
                "{}: its owner list doesn't match its rows",
                entry.name
            );
            ensure!(entry.owners.is_none(), "{}: two owner lists", entry.name);
            for at in (begin..end).step_by(4) {
                ensure!(
                    (u32_at(data, at) as usize) < entity_count,
                    "{}: a row is owned by an entity that doesn't exist",
                    entry.name
                );
            }
            entry.owners = Some(HEADER_LEN + begin);
        }

        let mut by_entity = HashMap::with_capacity(entity_count);
        let start = tables[entities].start;
        for i in 0..entity_count {
            let mut id = [0u8; 16];
            id.copy_from_slice(&bytes[start + i * 16..start + i * 16 + 16]);
            ensure!(
                by_entity.insert(id, i as u32).is_none(),
                "the entity list has the same identifier twice"
            );
        }

        // The game refuses a blob whose checksum is wrong, and a mismatch here would mean
        // this code's idea of the checksum is wrong, so nothing is built on such a blob.
        let blob = NewAge {
            bytes,
            tables,
            by_name,
            entities,
            entity_count,
            by_entity,
        };
        ensure!(
            blob.checksum() == u64_at(&blob.bytes, 8),
            "the entity data fails its checksum"
        );
        Ok(blob)
    }

    /// The checksum of the blob's current contents.
    pub fn checksum(&self) -> u64 {
        murmur3_x64_128(&self.bytes[HEADER_LEN..], CHECKSUM_SEED).0
    }

    /// The blob with any edits made and its checksum brought up to date.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.bytes.clone();
        out[8..16].copy_from_slice(&self.checksum().to_le_bytes());
        out
    }

    /// Where `row` of `table` starts in the blob.
    pub fn row_offset(&self, table: TableId, row: usize) -> usize {
        let t = &self.tables[table];
        assert!(row < t.rows, "{} has {} rows", t.name, t.rows);
        t.start + row * t.row_len
    }

    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    pub fn table(&self, name: &str) -> Option<TableId> {
        self.by_name.get(name).copied()
    }

    fn table_ref(&self, name: &str) -> Result<(TableId, &Table)> {
        let id = self
            .table(name)
            .ok_or_else(|| anyhow!("the save has no {name} table"))?;
        Ok((id, &self.tables[id]))
    }

    pub fn entity_count(&self) -> usize {
        self.entity_count
    }

    /// The identifier of the entity at `index`.
    pub fn entity_id(&self, index: u32) -> Uuid {
        assert!((index as usize) < self.entity_count, "no entity {index}");
        let mut id = [0u8; 16];
        id.copy_from_slice(self.row(self.entities, index as usize));
        Uuid(id)
    }

    /// The index of the entity with this identifier.
    pub fn entity_index(&self, id: Uuid) -> Option<u32> {
        self.by_entity.get(&id.0).copied()
    }

    pub fn row(&self, table: TableId, row: usize) -> &[u8] {
        let t = &self.tables[table];
        assert!(row < t.rows, "{} has {} rows", t.name, t.rows);
        &self.bytes[t.start + row * t.row_len..t.start + (row + 1) * t.row_len]
    }

    /// The entity that owns `row` of a component table.
    pub fn owner(&self, table: TableId, row: usize) -> Option<u32> {
        let t = &self.tables[table];
        (row < t.rows).then_some(u32_at(&self.bytes, t.owners? + row * 4))
    }

    /// The row of a component table that belongs to `entity`.
    pub fn row_of(&self, table: TableId, entity: u32) -> Option<usize> {
        let t = &self.tables[table];
        let owners = t.owners?;
        (0..t.rows).find(|&row| u32_at(&self.bytes, owners + row * 4) == entity)
    }

    /// Reads a u32 from the row of table `name` that belongs to `entity`.
    pub fn get_u32(&self, name: &str, entity: u32, offset: usize) -> Result<u32> {
        let (id, table) = self.table_ref(name)?;
        ensure!(offset + 4 <= table.row_len, "{name}: no field at {offset}");
        let row = self
            .row_of(id, entity)
            .ok_or_else(|| anyhow!("{name}: the entity has no row"))?;
        Ok(u32_at(self.row(id, row), offset))
    }

    /// Overwrites a u32 in the row of table `name` that belongs to `entity`.
    pub fn set_u32(&mut self, name: &str, entity: u32, offset: usize, value: u32) -> Result<()> {
        let (id, table) = self.table_ref(name)?;
        ensure!(offset + 4 <= table.row_len, "{name}: no field at {offset}");
        let row = self
            .row_of(id, entity)
            .ok_or_else(|| anyhow!("{name}: the entity has no row"))?;
        let at = table.start + row * table.row_len + offset;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Where the array is that a row points at. Many rows hold a pair of offsets, begin and
    /// end, to data elsewhere in the blob; `offset` is where that pair sits in the row.
    fn pointed_array(
        &self,
        name: &str,
        entity: u32,
        offset: usize,
    ) -> Result<std::ops::Range<usize>> {
        let (id, table) = self.table_ref(name)?;
        ensure!(
            offset + 16 <= table.row_len,
            "{name}: no pair of offsets at {offset}"
        );
        let row = self
            .row_of(id, entity)
            .ok_or_else(|| anyhow!("{name}: the entity has no row"))?;
        let row = self.row(id, row);
        let (begin, end) = (u64_at(row, offset), u64_at(row, offset + 8));
        ensure!(
            begin <= end && end <= u64_at(&self.bytes, 16) && (end - begin).is_multiple_of(4),
            "{name}: the field at {offset} is not a pair of offsets to 32-bit values"
        );
        Ok(HEADER_LEN + begin as usize..HEADER_LEN + end as usize)
    }

    /// The u32 values in the array that the row of table `name` belonging to `entity`
    /// points at with the pair of offsets at `offset`.
    pub fn get_array_u32(&self, name: &str, entity: u32, offset: usize) -> Result<Vec<u32>> {
        let range = self.pointed_array(name, entity, offset)?;
        Ok(range.step_by(4).map(|at| u32_at(&self.bytes, at)).collect())
    }

    /// Overwrites the u32 at `index` of that array.
    pub fn set_array_u32(
        &mut self,
        name: &str,
        entity: u32,
        offset: usize,
        index: usize,
        value: u32,
    ) -> Result<()> {
        let range = self.pointed_array(name, entity, offset)?;
        ensure!(
            index < range.len() / 4,
            "{name}: the array at {offset} has {} values",
            range.len() / 4
        );
        let at = range.start + index * 4;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Writes the blob back into the document it came from, with its checksum up to date.
    pub fn store(&self, lsf: &mut Lsf) -> Result<()> {
        let attr =
            NewAge::attribute(lsf).ok_or_else(|| anyhow!("the document has no NewAge data"))?;
        if lsf.attr_bytes(attr).len() != self.bytes.len() {
            bail!("the NewAge data changed size");
        }
        lsf.set_bytes(attr, &self.to_bytes())
    }
}

/// Builds a blob from scratch. Used by tests, which have no real save to start from.
pub mod build {
    use super::*;

    /// A table to write: name, row length, rows, and the owning entity of each row for a
    /// component table.
    pub struct TableSpec<'a> {
        pub name: &'a str,
        pub row_len: usize,
        pub rows: Vec<Vec<u8>>,
        pub owners: Option<Vec<u32>>,
    }

    /// Serializes `entities` and `tables` in the layout the game uses.
    pub fn blob(entities: &[[u8; 16]], tables: &[TableSpec]) -> Vec<u8> {
        const SLOTS: usize = 514;
        let mut data = vec![0u8; 8];
        let mut names = vec![0u8];
        let mut records = Vec::new();
        let mut record = |name: &str, row_len: usize, rows: usize, offset: usize| {
            records.extend_from_slice(&(names.len() as u64).to_le_bytes());
            records.extend_from_slice(&(name.len() as u32).to_le_bytes());
            // Unused; the game leaves whatever was in memory.
            records.extend_from_slice(&0x8fu32.to_le_bytes());
            records.extend_from_slice(&0u64.to_le_bytes());
            records.extend_from_slice(&(row_len as u32).to_le_bytes());
            records.extend_from_slice(&0u32.to_le_bytes());
            records.extend_from_slice(&(rows as u64).to_le_bytes());
            records.extend_from_slice(&(offset as u64).to_le_bytes());
            names.extend_from_slice(name.as_bytes());
        };

        let level_at = data.len();
        record(LEVEL_TABLE, 32, 1, level_at);
        data.resize(level_at + 32, 0);
        let entities_at = data.len();
        record(
            ENTITY_TABLE,
            16,
            entities.len() + usize::from(!entities.is_empty()),
            entities_at,
        );
        for id in entities {
            data.extend_from_slice(id);
        }
        let entities_end = data.len();
        // After the list, the table holds references to entities; repeat the first one.
        if let Some(first) = entities.first() {
            data.extend_from_slice(first);
        }
        for table in tables {
            record(table.name, table.row_len, table.rows.len(), data.len());
            for row in &table.rows {
                assert_eq!(row.len(), table.row_len);
                data.extend_from_slice(row);
            }
        }
        let mut slots = vec![[u64::MAX, u64::MAX, u64::MAX, 0]; SLOTS];
        for (i, table) in tables.iter().enumerate() {
            if let Some(owners) = &table.owners {
                let begin = data.len();
                for owner in owners {
                    data.extend_from_slice(&owner.to_le_bytes());
                }
                // Table indices 0 and 1 are the level and entity tables.
                slots[(i * 7 + 3) % SLOTS] = [
                    begin as u64,
                    data.len() as u64,
                    i as u64 + 2,
                    owners.len() as u64,
                ];
            }
        }
        while !data.len().is_multiple_of(8) {
            data.push(0);
        }
        let map_at = data.len();
        for slot in &slots {
            for field in slot {
                data.extend_from_slice(&field.to_le_bytes());
            }
        }
        let map_end = data.len();
        data.extend_from_slice(&[0xee; 24]);
        for (i, field) in [entities_at, entities_end, map_at, map_end]
            .iter()
            .enumerate()
        {
            data[level_at + i * 8..level_at + i * 8 + 8]
                .copy_from_slice(&(*field as u64).to_le_bytes());
        }
        while names.len() % 8 != 0 {
            names.push(0);
        }

        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION);
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&((names.len() + records.len()) as u64).to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&((tables.len() + 2) as u16).to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&data);
        out.extend_from_slice(&names);
        out.extend_from_slice(&records);
        let checksum = murmur3_x64_128(&out[HEADER_LEN..], CHECKSUM_SEED).0;
        out[8..16].copy_from_slice(&checksum.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::TableSpec;
    use super::*;

    const EXPERIENCE: &str = "game.experience.v0.ExperienceComponent";

    fn sample() -> Vec<u8> {
        let experience = |xp: u32| [xp.to_le_bytes(), xp.to_le_bytes(), [0; 4]].concat();
        build::blob(
            &[[1; 16], [2; 16], [3; 16]],
            &[
                TableSpec {
                    name: EXPERIENCE,
                    row_len: 12,
                    rows: vec![experience(5655), experience(5595)],
                    // The second and third entities, in that order.
                    owners: Some(vec![2, 1]),
                },
                TableSpec {
                    name: "core.v0.TranslatedString",
                    row_len: 4,
                    rows: vec![vec![9; 4]],
                    owners: None,
                },
            ],
        )
    }

    #[test]
    fn reads_tables_and_owners() {
        let blob = NewAge::parse(sample()).unwrap();
        assert_eq!(blob.tables().len(), 4);
        assert_eq!(blob.entity_count(), 3);
        assert_eq!(blob.entity_index(Uuid([3; 16])), Some(2));
        assert_eq!(blob.entity_index(Uuid([9; 16])), None);
        assert_eq!(blob.entity_id(1), Uuid([2; 16]));

        let table = blob.table(EXPERIENCE).unwrap();
        assert!(blob.tables()[table].is_component());
        assert_eq!(blob.tables()[table].rows, 2);
        assert_eq!(blob.owner(table, 0), Some(2));
        assert_eq!(blob.row_of(table, 1), Some(1));
        assert_eq!(blob.row_of(table, 0), None);
        assert_eq!(blob.get_u32(EXPERIENCE, 2, 0).unwrap(), 5655);
        assert_eq!(blob.get_u32(EXPERIENCE, 1, 4).unwrap(), 5595);

        let strings = blob.table("core.v0.TranslatedString").unwrap();
        assert!(!blob.tables()[strings].is_component());
        assert_eq!(blob.owner(strings, 0), None);
    }

    #[test]
    fn an_edit_changes_only_its_bytes() {
        let original = sample();
        let mut blob = NewAge::parse(original.clone()).unwrap();
        blob.set_u32(EXPERIENCE, 1, 0, 9000).unwrap();
        assert_eq!(blob.get_u32(EXPERIENCE, 1, 0).unwrap(), 9000);
        assert_eq!(blob.get_u32(EXPERIENCE, 2, 0).unwrap(), 5655);

        // Written out, only the edited field and the checksum differ.
        let written = blob.to_bytes();
        let table = blob.table(EXPERIENCE).unwrap();
        let field = blob.row_offset(table, blob.row_of(table, 1).unwrap());
        let changed: Vec<usize> = (0..original.len())
            .filter(|&i| original[i] != written[i])
            .collect();
        assert!(!changed.is_empty());
        assert!(
            changed
                .iter()
                .all(|&i| (8..16).contains(&i) || (field..field + 4).contains(&i)),
            "{changed:?}"
        );
        assert_eq!(written[field..field + 4], 9000u32.to_le_bytes());
        assert_ne!(
            written[8..16],
            original[8..16],
            "the checksum follows the edit"
        );
        // The edited blob is well formed, checksum included, and reads back the edit.
        let again = NewAge::parse(written).unwrap();
        assert_eq!(again.get_u32(EXPERIENCE, 1, 0).unwrap(), 9000);
        // An unedited blob is written out exactly as it was read.
        assert_eq!(
            NewAge::parse(original.clone()).unwrap().to_bytes(),
            original
        );

        assert!(
            blob.set_u32(EXPERIENCE, 0, 0, 1).is_err(),
            "entity 0 has no row"
        );
        assert!(blob.set_u32(EXPERIENCE, 1, 10, 1).is_err(), "past the row");
        assert!(blob.set_u32("game.missing.v0.Component", 1, 0, 1).is_err());
    }

    #[test]
    fn follows_a_row_to_the_array_it_points_at() {
        const POINTERS: &str = "game.test.v0.PointerComponent";
        let blob = build::blob(
            &[[1; 16], [2; 16]],
            &[
                TableSpec {
                    name: "game.test.v0.Values",
                    row_len: 4,
                    rows: vec![vec![7, 0, 0, 0], vec![5, 0, 0, 0], vec![6, 0, 0, 0]],
                    owners: None,
                },
                TableSpec {
                    name: POINTERS,
                    row_len: 24,
                    rows: vec![vec![0; 24]],
                    owners: Some(vec![1]),
                },
            ],
        );
        // Point the row's second and third words at the three values, as the game's rows
        // point at their arrays: offsets from the start of the data section.
        let mut blob = NewAge::parse(blob).unwrap();
        let values = blob.table("game.test.v0.Values").unwrap();
        let begin = (blob.row_offset(values, 0) - 48) as u32;
        blob.set_u32(POINTERS, 1, 8, begin).unwrap();
        blob.set_u32(POINTERS, 1, 16, begin + 12).unwrap();
        let mut blob = NewAge::parse(blob.to_bytes()).unwrap();

        assert_eq!(blob.get_array_u32(POINTERS, 1, 8).unwrap(), [7, 5, 6]);
        blob.set_array_u32(POINTERS, 1, 8, 1, 9).unwrap();
        assert_eq!(blob.get_array_u32(POINTERS, 1, 8).unwrap(), [7, 9, 6]);
        assert_eq!(blob.row(values, 1), [9, 0, 0, 0]);
        assert!(
            blob.set_array_u32(POINTERS, 1, 8, 3, 1).is_err(),
            "past the end"
        );
        assert!(
            blob.get_array_u32(POINTERS, 1, 16).is_err(),
            "no room for a pair"
        );
        // Words that are not a begin and end are refused: here the end comes first.
        blob.set_u32(POINTERS, 1, 0, begin + 100).unwrap();
        assert!(blob.get_array_u32(POINTERS, 1, 0).is_err());
        NewAge::parse(blob.to_bytes()).unwrap();
    }

    #[test]
    fn rejects_damaged_blobs() {
        let bytes = sample();
        assert!(NewAge::parse(bytes[..20].to_vec()).is_err());
        let mut wrong_magic = bytes.clone();
        wrong_magic[3] = b'X';
        assert!(NewAge::parse(wrong_magic).is_err());
        let mut wrong_version = bytes.clone();
        wrong_version[4] = 2;
        assert!(NewAge::parse(wrong_version).is_err());
        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(NewAge::parse(truncated).is_err());
        // A row count that runs a table past the data section.
        let mut too_many = bytes.clone();
        let last = too_many.len() - 16;
        too_many[last..last + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(NewAge::parse(too_many).is_err());
    }

    #[test]
    fn rejects_a_blob_that_fails_its_checksum() {
        let bytes = sample();
        // One bit of the checksum, one unused byte of the data, one unused byte of the
        // directory: each is enough for the game to refuse the save.
        let data_end = 48 + u64_at(&bytes, 16) as usize;
        for at in [8, 48, data_end] {
            let mut changed = bytes.clone();
            changed[at] ^= 1;
            let error = NewAge::parse(changed).err().unwrap().to_string();
            assert!(error.contains("checksum"), "byte {at}: {error}");
        }
    }

    #[test]
    fn checksum_function_matches_the_reference() {
        // Reference values for MurmurHash3 x64_128 with 32-bit seeds, covering an empty input
        // and tails shorter than, equal to and longer than eight bytes.
        let fox = b"The quick brown fox jumps over the lazy dog";
        let counting: Vec<u8> = (0..40).collect();
        let cases: [(&[u8], u64, (u64, u64)); 6] = [
            (b"", 0, (0, 0)),
            (fox, 0, (0xe34b_bc7b_bc07_1b6c, 0x7a43_3ca9_c49a_9347)),
            (b"a", 0, (0x8555_5565_f659_7889, 0xe6b5_3a48_510e_895a)),
            (
                &fox[..16],
                0,
                (0x9d12_44f4_af9b_32c4, 0x3d15_3c8b_2c2a_3aa6),
            ),
            (
                &fox[..25],
                0x9747_b28c,
                (0x785c_7748_7d9c_95a1, 0x0a76_9e50_9f6d_5935),
            ),
            (
                &counting,
                42,
                (0xae8e_6b70_5c22_d1da, 0xa479_4c6f_0ee8_9ef1),
            ),
        ];
        for (data, seed, expected) in cases {
            assert_eq!(
                murmur3_x64_128(data, seed),
                expected,
                "{data:?} seed {seed}"
            );
        }
        // And with the 64-bit seed the game uses.
        assert_eq!(
            murmur3_x64_128(fox, CHECKSUM_SEED),
            (0x9c25_2313_9d76_b285, 0x4a09_cf92_d0c7_b26a)
        );
        assert_eq!(
            murmur3_x64_128(&counting, CHECKSUM_SEED),
            (0x531b_92c2_0850_a404, 0x2357_7ed5_d1c4_2314)
        );
    }
}
