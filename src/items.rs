//! Adding items to a save. **Research**: nothing here has been tried in the game yet.
//!
//! An item is an entity: a `Creator` and an `Item` node in `Globals.lsf`'s item factory
//! (in the same position in both lists), an entry in the entity data's entity list, and a
//! row in each of some forty component tables. An item in an inventory also has:
//!
//! - `game.inventory.v0.MemberComponent`: an offset to a `MemberData` (16 bytes): an offset
//!   to the inventory's entity identifier, then the item's slot number (u32).
//! - On the inventory, `game.inventory.v1.ContainerComponent`: two pairs of offsets, to the
//!   slot numbers (u16 each) and to offsets of `ContainerSlotData` (16 bytes each: an offset
//!   to the item's entity identifier, a u32 position, and four bytes left as they were).
//!
//! Ownership is kept on both sides. The item's `game.v0.OwneeCurrentComponent` points at
//! its owner, and the owner's `IsCurrentOwner`, `IsLatestOwner` and `IsOriginalOwner`
//! components each hold a pair of offsets to a list of offsets to the identifiers of what
//! it owns.
//!
//! A copy is made by appending: a new entity, a copy of every row the original owns, its own
//! `MemberData` and `ContainerSlotData`, the inventory's two arrays with one more entry each,
//! and each ownership list that has the original with one more entry. Nothing already in the
//! save moves.

use crate::lsf::{Lsf, NodeId};
use crate::newage::NewAge;
use crate::value::{Uuid, Value};
use anyhow::{anyhow, ensure, Context, Result};

const MEMBER: &str = "game.inventory.v0.MemberComponent";
const MEMBER_DATA: &str = "game.inventory.v0.MemberData";
const SLOT_DATA: &str = "game.inventory.v0.ContainerSlotData";
const ENTITY_TABLE: &str = "core.v0.EntityId";
const CONTAINER: &str = "game.inventory.v1.ContainerComponent";
/// Unique to each item: sixteen bytes that look like an identifier.
const SIGHT: &str = "game.sight.v0.DataComponent";
/// A stacked item; copying one is not supported yet. The stack itself has a
/// `NewStackComponent`, and every other item in it a `StackMemberComponent`.
const STACKED: &str = "game.inventory.v0.NewStackComponent";
const STACK_MEMBER: &str = "game.inventory.v0.StackMemberComponent";
/// An offset to the item's template identifier as text, then its length.
const TEMPLATE: &str = "game.templates.v0.TemplateComponent";
/// The same, for what it looks like.
const VISUAL: &str = "game.visual.v5.GameObjectVisualComponent";
/// On an item someone is wearing or wielding: an offset to that character's identifier.
const WIELDING: &str = "game.inventory.v0.WieldingComponent";
/// On an item the story depends on.
const STORY: &str = "game.item.v0.IsStoryItemComponent";
/// On an inventory: an offset to its owner, which for a bag is the bag.
const INVENTORY_OWNER: &str = "game.inventory.v1.IsOwnedComponent";
/// On an owner: a pair of offsets to a list of offsets to the identifiers of what it owns.
const OWNER_LISTS: [&str; 3] = [
    "game.v0.IsCurrentOwnerComponent",
    "game.v0.IsLatestOwnerComponent",
    "game.v0.IsOriginalOwnerComponent",
];

fn u64_of(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]))
}
fn u32_of(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4]))
}

/// A new random identifier.
pub fn new_uuid() -> Uuid {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let half = || {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        hasher.finish().to_le_bytes()
    };
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&half());
    id[8..].copy_from_slice(&half());
    Uuid(id)
}

/// The `Creator` and `Item` nodes of an item, found by its entity.
fn item_nodes(globals: &Lsf, item: Uuid) -> Result<(NodeId, NodeId)> {
    let creators = globals
        .find(&["Items", "ItemFactory", "Creators"])
        .ok_or_else(|| anyhow!("the save has no item creators"))?;
    let items = globals
        .find(&["Items", "ItemFactory", "Items"])
        .ok_or_else(|| anyhow!("the save has no item list"))?;
    let position = globals
        .children_named(creators, "Creator")
        .position(|c| globals.get(c, "Entity").and_then(|v| v.as_uuid()) == Some(item))
        .ok_or_else(|| anyhow!("no creator for {item}"))?;
    let creator = globals
        .children_named(creators, "Creator")
        .nth(position)
        .ok_or_else(|| anyhow!("no creator for {item}"))?;
    let node = globals
        .children_named(items, "Item")
        .nth(position)
        .ok_or_else(|| anyhow!("the item list is shorter than the creator list"))?;
    Ok((creator, node))
}

/// Where an item sits in its inventory.
struct Place {
    /// The item's `MemberComponent` row.
    member_row: usize,
    /// Its `MemberData` record.
    member_data: Vec<u8>,
    /// The inventory's `ContainerComponent` row.
    container_row: usize,
    /// Its slot record, and where the inventory's list points at it.
    slot_data: Vec<u8>,
    slot_index: usize,
}

fn table(entities: &NewAge, name: &str) -> Result<usize> {
    entities
        .table(name)
        .ok_or_else(|| anyhow!("the save has no {name} table"))
}

/// The two arrays of an inventory's container: slot numbers and offsets of slot records.
fn container_arrays(entities: &NewAge, row: usize) -> Result<(Vec<u8>, Vec<u8>)> {
    let container = table(entities, CONTAINER)?;
    let fields = entities.row(container, row);
    let (keys, keys_end) = (u64_of(fields, 0), u64_of(fields, 8));
    let (values, values_end) = (u64_of(fields, 16), u64_of(fields, 24));
    ensure!(
        keys <= keys_end && values <= values_end,
        "an inventory's lists are back to front"
    );
    ensure!(
        (values_end - values) / 8 == (keys_end - keys) / 2,
        "an inventory's lists differ in length"
    );
    Ok((
        entities.data(keys, (keys_end - keys) as usize)?.to_vec(),
        entities
            .data(values, (values_end - values) as usize)?
            .to_vec(),
    ))
}

fn place(entities: &NewAge, index: u32) -> Result<Place> {
    let member = table(entities, MEMBER)?;
    let member_row = entities
        .row_of(member, index)
        .ok_or_else(|| anyhow!("the item is not in an inventory"))?;
    let member_data = entities
        .data(u64_of(entities.row(member, member_row), 0), 16)?
        .to_vec();
    let inventory = entities
        .entity_at(u64_of(&member_data, 0))
        .ok_or_else(|| anyhow!("the item's inventory is not an entity"))?;
    let container_row = entities
        .row_of(table(entities, CONTAINER)?, inventory)
        .ok_or_else(|| anyhow!("the item's inventory has no container"))?;
    let (_, values) = container_arrays(entities, container_row)?;
    for (slot_index, pointer) in values.chunks_exact(8).enumerate() {
        let slot_data = entities.data(u64_of(pointer, 0), 16)?.to_vec();
        if entities.entity_at(u64_of(&slot_data, 0)) == Some(index) {
            return Ok(Place {
                member_row,
                member_data,
                container_row,
                slot_data,
                slot_index,
            });
        }
    }
    Err(anyhow!("the item's inventory does not list it"))
}

/// Points an inventory's container at new arrays, appended to the end of the data.
fn set_container_arrays(
    entities: &mut NewAge,
    row: usize,
    keys: &[u8],
    values: &[u8],
) -> Result<()> {
    let container = table(entities, CONTAINER)?;
    let keys_at = entities.append(keys);
    let values_at = entities.append(values);
    let row_at = (entities.row_offset(container, row) - 48) as u64;
    let mut pairs = Vec::with_capacity(32);
    for value in [
        keys_at,
        keys_at + keys.len() as u64,
        values_at,
        values_at + values.len() as u64,
    ] {
        pairs.extend_from_slice(&value.to_le_bytes());
    }
    entities.set_data(row_at, &pairs)
}

/// Adds a record to a table of records that rows point at, keeping every record inside the
/// table: the table is copied to the end with the new record, and every pointer into the
/// old copy is moved by the same distance. `pointers` lists where those pointers are.
/// Returns the new record's offset.
fn add_record(entities: &mut NewAge, name: &str, record: &[u8], pointers: &[u64]) -> Result<u64> {
    let id = table(entities, name)?;
    let (old_start, len) = {
        let t = &entities.tables()[id];
        (
            (entities.row_offset(id, 0) - 48) as u64,
            (t.rows * t.row_len) as u64,
        )
    };
    let row = entities.add_row(id, None, record)?;
    let new_start = (entities.row_offset(id, 0) - 48) as u64;
    for &at in pointers {
        let old = u64_of(entities.data(at, 8)?, 0);
        ensure!(
            (old_start..old_start + len).contains(&old),
            "{name}: a pointer at {at} is not into the table"
        );
        entities.set_data(at, &(old - old_start + new_start).to_le_bytes())?;
    }
    Ok((entities.row_offset(id, row) - 48) as u64)
}

/// Where every pointer to a `MemberData` record is: the start of every `MemberComponent` row.
fn member_data_pointers(entities: &NewAge) -> Result<Vec<u64>> {
    let member = table(entities, MEMBER)?;
    Ok((0..entities.tables()[member].rows)
        .map(|row| (entities.row_offset(member, row) - 48) as u64)
        .collect())
}

/// Where every pointer to a `ContainerSlotData` record is: each entry of every inventory's
/// list of slot records.
fn slot_data_pointers(entities: &NewAge) -> Result<Vec<u64>> {
    let container = table(entities, CONTAINER)?;
    let mut out = Vec::new();
    for row in 0..entities.tables()[container].rows {
        let fields = entities.row(container, row);
        let (values, values_end) = (u64_of(fields, 16), u64_of(fields, 24));
        out.extend((values..values_end).step_by(8));
    }
    Ok(out)
}

/// Adds a `MemberData` record to its table and returns its offset.
pub fn add_member_data(entities: &mut NewAge, record: &[u8]) -> Result<u64> {
    let pointers = member_data_pointers(entities)?;
    add_record(entities, MEMBER_DATA, record, &pointers)
}

/// Adds a `ContainerSlotData` record to its table and returns its offset.
pub fn add_slot_data(entities: &mut NewAge, record: &[u8]) -> Result<u64> {
    let pointers = slot_data_pointers(entities)?;
    add_record(entities, SLOT_DATA, record, &pointers)
}

/// A row of the entity table that nothing points at, filled with `id`, for rows to point at
/// to mean that entity. The game writes the entity list at the head of the entity table, and
/// most of it is pointed at by nothing; once the list has been moved
/// ([`NewAge::add_entities`]), those rows are free.
pub fn add_reference(entities: &mut NewAge, id: Uuid) -> Result<u64> {
    let ids = table(entities, ENTITY_TABLE)?;
    let rows = entities.tables()[ids].rows;
    let start = (entities.row_offset(ids, 0) - 48) as u64;
    let list = entities.entity_list_offset();
    // Rows of the entity table that are not the entity list as it now stands.
    let candidates: Vec<u64> = (0..rows as u64)
        .map(|r| start + r * 16)
        .filter(|&at| !(list.0..list.1).contains(&at))
        .take(4096)
        .collect();
    let free = entities.unreferenced(&candidates);
    let at = *free
        .first()
        .ok_or_else(|| anyhow!("no free row in the entity table"))?;
    entities.set_data(at, &id.0)?;
    Ok(at)
}

/// Why an item can't be copied, if it can't.
pub fn why_not_copied(entities: &NewAge, index: u32) -> Option<&'static str> {
    let has = |name: &str| {
        entities
            .table(name)
            .is_some_and(|t| entities.row_of(t, index).is_some())
    };
    if !has(crate::party::tables::ITEM) {
        return Some("it is not an item");
    }
    if has(STACKED) || has(STACK_MEMBER) {
        return Some("it is a stack; change how many it holds instead");
    }
    if has(WIELDING) {
        return Some("it is equipped; unequip it in the game first");
    }
    if has(STORY) {
        return Some("the story depends on it");
    }
    if let Some(owned) = entities.table(INVENTORY_OWNER) {
        let holds = entities
            .owned_rows(owned)
            .any(|(row, _)| entities.entity_at(u64_of(entities.row(owned, row), 0)) == Some(index));
        if holds {
            return Some("it holds other items");
        }
    }
    if !has(MEMBER) {
        return Some("it is not in an inventory");
    }
    None
}

/// Why an item can't be the pattern a new item is copied from, if it can't: for a copy of
/// the same item, and else for one made into another.
pub fn why_not_pattern(globals: &Lsf, entities: &NewAge, item: Uuid) -> Option<&'static str> {
    let index = entities.entity_index(item)?;
    if let Some(reason) = why_not_copied(entities, index) {
        return Some(reason);
    }
    if place(entities, index).is_err() {
        return Some("its inventory doesn't list it");
    }
    if item_nodes(globals, item).is_err() {
        return Some("it has no creator in the save");
    }
    None
}

/// Whether a copy of `item` can be made into another item: only one made from one of the
/// game's own templates can.
pub fn can_become_another(globals: &Lsf, item: Uuid) -> bool {
    item_nodes(globals, item)
        .is_ok_and(|(creator, _)| globals.get(creator, "TemplateType") == Some(Value::Byte(0)))
}

/// Copies an unstacked item that is in an inventory, into the same inventory. Returns the
/// copy's entity.
pub fn copy_item(globals: &mut Lsf, entities: &mut NewAge, item: Uuid) -> Result<Uuid> {
    let index = entities
        .entity_index(item)
        .ok_or_else(|| anyhow!("no entity {item} in the save"))?;
    let item_table = table(entities, crate::party::tables::ITEM)?;
    ensure!(
        entities.row_of(item_table, index).is_some(),
        "{item} is not an item"
    );
    if let Some(reason) = why_not_copied(entities, index) {
        anyhow::bail!("{item} can't be copied: {reason}");
    }
    let place = place(entities, index)?;

    // Every row the item owns, read before anything is added.
    let owned: Vec<(usize, Vec<u8>)> = (0..entities.tables().len())
        .filter_map(|t| {
            let row = entities.row_of(t, index)?;
            Some((t, entities.row(t, row).to_vec()))
        })
        .collect();

    let copy = new_uuid();
    let copy_index = entities.add_entities(&[copy])?[0];
    let copy_ref = add_reference(entities, copy)?;

    // A slot in the same inventory, after the last.
    let (keys, values) = container_arrays(entities, place.container_row)?;
    let slot = keys
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .max()
        .map_or(0, |s| s + 1);
    let mut position = 0;
    for pointer in values.chunks_exact(8) {
        let record = entities.data(u64_of(pointer, 0), 16)?;
        position = position.max(u32_of(record, 8) + 1);
    }

    let mut member_data = place.member_data.clone();
    member_data[8..12].copy_from_slice(&(slot as u32).to_le_bytes());
    let member_data_at = add_member_data(entities, &member_data)?;

    let mut slot_data = place.slot_data.clone();
    slot_data[0..8].copy_from_slice(&copy_ref.to_le_bytes());
    slot_data[8..12].copy_from_slice(&position.to_le_bytes());
    let slot_data_at = add_slot_data(entities, &slot_data)?;

    // The inventory's lists, read again now that the pointers in them have moved.
    let (mut keys, mut values) = container_arrays(entities, place.container_row)?;
    keys.extend_from_slice(&slot.to_le_bytes());
    values.extend_from_slice(&slot_data_at.to_le_bytes());
    set_container_arrays(entities, place.container_row, &keys, &values)?;

    // Wherever the original is listed as owned, the copy is too.
    for name in OWNER_LISTS {
        let Some(table) = entities.table(name) else {
            continue;
        };
        for row in 0..entities.tables()[table].rows {
            let fields = entities.row(table, row).to_vec();
            let (begin, end) = (u64_of(&fields, 0), u64_of(&fields, 8));
            if begin == u64::MAX || end < begin || !(end - begin).is_multiple_of(8) {
                continue;
            }
            let mut list = entities.data(begin, (end - begin) as usize)?.to_vec();
            let listed = list
                .chunks_exact(8)
                .any(|p| entities.entity_at(u64_of(p, 0)) == Some(index));
            if listed {
                list.extend_from_slice(&copy_ref.to_le_bytes());
                let at = entities.append(&list);
                let row_at = (entities.row_offset(table, row) - 48) as u64;
                let mut pair = at.to_le_bytes().to_vec();
                pair.extend_from_slice(&(at + list.len() as u64).to_le_bytes());
                entities.set_data(row_at, &pair)?;
            }
        }
    }

    for (table, mut row) in owned {
        match entities.tables()[table].name.as_str() {
            MEMBER => row[0..8].copy_from_slice(&member_data_at.to_le_bytes()),
            SIGHT => row.copy_from_slice(&new_uuid().0),
            _ => {}
        }
        entities
            .add_row(table, Some(copy_index), &row)
            .with_context(|| entities.tables()[table].name.clone())?;
    }

    copy_nodes(globals, item, copy)?;
    Ok(copy)
}

/// Research: copies `item`, then makes the copy into the kind of item `like` is, by giving
/// its nodes `like`'s template and game name and giving it `like`'s rows in the tables named
/// in `rows`. Everything else stays as the original's. Returns the copy.
pub fn copy_item_as(
    globals: &mut Lsf,
    entities: &mut NewAge,
    item: Uuid,
    like: Uuid,
    rows: &[&str],
) -> Result<Uuid> {
    let like_index = entities
        .entity_index(like)
        .ok_or_else(|| anyhow!("no entity {like} in the save"))?;
    let copy = copy_item(globals, entities, item)?;
    let copy_index = entities
        .entity_index(copy)
        .ok_or_else(|| anyhow!("the copy is missing"))?;
    for &name in rows {
        let id = table(entities, name)?;
        let (Some(from), Some(to)) = (
            entities.row_of(id, like_index),
            entities.row_of(id, copy_index),
        ) else {
            continue;
        };
        let bytes = entities.row(id, from).to_vec();
        let at = (entities.row_offset(id, to) - 48) as u64;
        entities.set_data(at, &bytes)?;
    }
    let (like_creator, like_node) = item_nodes(globals, like)?;
    let (creator, node) = item_nodes(globals, copy)?;
    for (from, to, names) in [
        (like_creator, creator, &["TemplateType", "TemplateID"][..]),
        (
            like_node,
            node,
            &["CurrentTemplate", "CurrentTemplateType", "Stats"][..],
        ),
    ] {
        for name in names {
            if let (Some(value), Some(attr)) = (globals.get(from, name), globals.attr(to, name)) {
                globals.set_value(attr, &value)?;
            }
        }
    }
    Ok(copy)
}

/// Copies `item` and makes the copy into the item made from `template`, with the game name
/// `stats`: the copy's nodes take the template and name, and its template and visual rows
/// point at the template's identifier. Everything else stays as the original's, so this is
/// for an item of the same kind. Returns the copy.
pub fn copy_item_to(
    globals: &mut Lsf,
    entities: &mut NewAge,
    item: Uuid,
    template: &str,
    stats: &str,
) -> Result<Uuid> {
    let template_id =
        Uuid::parse(template).ok_or_else(|| anyhow!("{template} is not a template identifier"))?;
    let (donor, _) = item_nodes(globals, item)?;
    ensure!(
        globals.get(donor, "TemplateType") == Some(Value::Byte(0)),
        "only an item made from one of the game's own templates can be copied as another"
    );
    let copy = copy_item(globals, entities, item)?;
    let copy_index = entities
        .entity_index(copy)
        .ok_or_else(|| anyhow!("the copy is missing"))?;

    // The template's identifier as the rows hold it: text, at the end of the data.
    let text = template_id.to_string();
    let text_at = entities.append(text.as_bytes());
    for name in [TEMPLATE, VISUAL] {
        let Some(id) = entities.table(name) else {
            continue;
        };
        let Some(row) = entities.row_of(id, copy_index) else {
            continue;
        };
        let at = (entities.row_offset(id, row) - 48) as u64;
        let mut fields = entities.row(id, row)[..12].to_vec();
        fields[0..8].copy_from_slice(&text_at.to_le_bytes());
        fields[8..12].copy_from_slice(&(text.len() as u32).to_le_bytes());
        entities.set_data(at, &fields)?;
    }

    let (creator, node) = item_nodes(globals, copy)?;
    let set = |globals: &mut Lsf, node: NodeId, name: &str, value: Value| -> Result<()> {
        let attr = globals
            .attr(node, name)
            .ok_or_else(|| anyhow!("the item's nodes have no {name}"))?;
        globals.set_value(attr, &value)
    };
    set(globals, creator, "TemplateID", Value::Uuid(template_id))?;
    set(globals, node, "CurrentTemplate", Value::Uuid(template_id))?;
    set(globals, node, "Stats", Value::Text(stats.to_owned()))?;
    Ok(copy)
}

/// Research: test changes that each check one part of the method on its own. None adds
/// anything a player would notice.
pub mod checks {
    use super::*;

    /// The item's `MemberData` copied to the end of the data, outside its table.
    pub fn loose_member_data(entities: &mut NewAge, item: Uuid) -> Result<()> {
        let index = entities
            .entity_index(item)
            .ok_or_else(|| anyhow!("no such entity"))?;
        let place = place(entities, index)?;
        let at = entities.append(&place.member_data);
        let member = table(entities, MEMBER)?;
        let row_at = (entities.row_offset(member, place.member_row) - 48) as u64;
        entities.set_data(row_at, &at.to_le_bytes())
    }

    /// The item's slot record copied to the end of the data, outside its table, and the
    /// inventory's list pointed at the copy.
    pub fn loose_slot_data(entities: &mut NewAge, item: Uuid) -> Result<()> {
        let index = entities
            .entity_index(item)
            .ok_or_else(|| anyhow!("no such entity"))?;
        let place = place(entities, index)?;
        let at = entities.append(&place.slot_data);
        let (keys, mut values) = container_arrays(entities, place.container_row)?;
        let i = place.slot_index * 8;
        values[i..i + 8].copy_from_slice(&at.to_le_bytes());
        set_container_arrays(entities, place.container_row, &keys, &values)
    }

    /// The item's slot record pointed at a copy of its identifier at the end of the data,
    /// outside the entity table.
    pub fn loose_reference(entities: &mut NewAge, item: Uuid) -> Result<()> {
        let index = entities
            .entity_index(item)
            .ok_or_else(|| anyhow!("no such entity"))?;
        let place = place(entities, index)?;
        let id_at = entities.append(&entities.entity_id(index).0);
        let (_, values) = container_arrays(entities, place.container_row)?;
        let record_at = u64_of(&values, place.slot_index * 8);
        entities.set_data(record_at, &id_at.to_le_bytes())
    }

    /// `MemberData` and `ContainerSlotData` each grown by a copy of their last record, which
    /// nothing points at, with every pointer into them moved.
    pub fn grown_tables(entities: &mut NewAge) -> Result<()> {
        for name in [MEMBER_DATA, SLOT_DATA] {
            let id = table(entities, name)?;
            let last = entities.tables()[id].rows - 1;
            let record = entities.row(id, last).to_vec();
            match name {
                MEMBER_DATA => add_member_data(entities, &record)?,
                _ => add_slot_data(entities, &record)?,
            };
        }
        Ok(())
    }

    /// The entity list moved, then one row of where it was, which nothing points at,
    /// overwritten with a new identifier.
    pub fn old_list_row(entities: &mut NewAge) -> Result<()> {
        entities.add_entities(&[])?;
        add_reference(entities, new_uuid()).map(|_| ())
    }
}

/// Copies an item's `Creator` and `Item` nodes, for the entity `copy`, with a new `UUID`.
fn copy_nodes(globals: &mut Lsf, item: Uuid, copy: Uuid) -> Result<()> {
    let (creator, node) = item_nodes(globals, item)?;
    let creators = globals
        .parent(creator)
        .ok_or_else(|| anyhow!("a creator with no list"))?;
    let items = globals
        .parent(node)
        .ok_or_else(|| anyhow!("an item with no list"))?;
    let new_creator = globals.copy_node(creator, creators)?;
    globals.copy_node(node, items)?;
    let entity = globals
        .attr(new_creator, "Entity")
        .ok_or_else(|| anyhow!("the item's creator has no Entity"))?;
    globals.set_value(entity, &Value::Uuid(copy))?;
    if let Some(uuid) = globals.attr(new_creator, "UUID") {
        globals.set_value(uuid, &Value::Uuid(new_uuid()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::build::*;
    use crate::party::{self, Holder};

    fn hero_items(globals: &Lsf, entities: &NewAge) -> Vec<(String, String)> {
        let party = party::read(globals, entities).unwrap();
        let hero = party
            .inventories
            .iter()
            .find(|i| i.holder == Holder::Character(0))
            .unwrap();
        hero.items
            .iter()
            .map(|i| (i.stats.clone(), i.template.clone()))
            .collect()
    }

    #[test]
    fn copies_an_item_into_the_same_pack() {
        let mut globals = sample_globals();
        let mut entities = sample_entities();
        let before = hero_items(&globals, &entities);
        let copy = copy_item(&mut globals, &mut entities, uuid(ROPE)).unwrap();
        entities.store(&mut globals).unwrap();
        // Written out and read again, as an export does.
        let globals = Lsf::parse(&globals.write().unwrap()).unwrap();
        let entities = NewAge::from_lsf(&globals).unwrap();
        let after = hero_items(&globals, &entities);
        assert_eq!(after.len(), before.len() + 1);
        let ropes = after.iter().filter(|(s, _)| s == "OBJ_Rope").count();
        assert_eq!(ropes, 2);
        let index = entities.entity_index(copy).unwrap();
        assert_eq!(why_not_copied(&entities, index), None);
        // It has a slot after the last, and the pack lists it.
        let place = place(&entities, index).unwrap();
        assert_eq!(
            u32::from_le_bytes(place.member_data[8..12].try_into().unwrap()),
            4
        );
    }

    #[test]
    fn a_copy_can_be_made_into_another_item() {
        let mut globals = sample_globals();
        let mut entities = sample_entities();
        let template = "00000000-0000-0000-0000-00000000ffff";
        copy_item_to(
            &mut globals,
            &mut entities,
            uuid(ROPE),
            template,
            "OBJ_Rope_Silk",
        )
        .unwrap();
        entities.store(&mut globals).unwrap();
        let globals = Lsf::parse(&globals.write().unwrap()).unwrap();
        let entities = NewAge::from_lsf(&globals).unwrap();
        let after = hero_items(&globals, &entities);
        assert!(after.contains(&("OBJ_Rope_Silk".to_owned(), template.to_owned())));
        assert!(after.contains(&(
            "OBJ_Rope".to_owned(),
            "00000000-0000-0000-0000-00000000cccc".to_owned()
        )));
    }

    #[test]
    fn says_why_an_item_cant_be_copied() {
        let entities = sample_entities();
        let reason = |e: u32| why_not_copied(&entities, entities.entity_index(uuid(e)).unwrap());
        assert_eq!(reason(ROPE), None);
        assert!(reason(BAG).unwrap().contains("holds other items"));
        assert!(reason(POTIONS).unwrap().contains("stack"));
        assert!(reason(HERO).unwrap().contains("not an item"));
    }
}
