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
const CONTAINER: &str = "game.inventory.v1.ContainerComponent";
/// Unique to each item: sixteen bytes that look like an identifier.
const SIGHT: &str = "game.sight.v0.DataComponent";
/// A stacked item; copying one is not supported yet.
const STACKED: &str = "game.inventory.v0.NewStackComponent";
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

/// Copies an unstacked item that is in an inventory, into the same inventory. Returns the
/// copy's entity.
pub fn copy_item(globals: &mut Lsf, entities: &mut NewAge, item: Uuid) -> Result<Uuid> {
    let index = entities
        .entity_index(item)
        .ok_or_else(|| anyhow!("no entity {item} in the save"))?;
    let item_table = entities
        .table(crate::party::tables::ITEM)
        .ok_or_else(|| anyhow!("the save has no items"))?;
    ensure!(
        entities.row_of(item_table, index).is_some(),
        "{item} is not an item"
    );
    if let Some(stacked) = entities.table(STACKED) {
        ensure!(
            entities.row_of(stacked, index).is_none(),
            "copying a stacked item is not supported yet"
        );
    }

    // Where the item is: its MemberData, and the inventory's container.
    let member = entities
        .table(MEMBER)
        .ok_or_else(|| anyhow!("the save has no {MEMBER} table"))?;
    let member_row = entities
        .row_of(member, index)
        .ok_or_else(|| anyhow!("{item} is not in an inventory"))?;
    let member_data_at = u64_of(entities.row(member, member_row), 0);
    let member_data = entities.data(member_data_at, 16)?.to_vec();
    let inventory_ref = u64_of(&member_data, 0);
    let inventory = entities
        .entity_at(inventory_ref)
        .ok_or_else(|| anyhow!("the item's inventory is not an entity"))?;
    let container = entities
        .table(CONTAINER)
        .ok_or_else(|| anyhow!("the save has no {CONTAINER} table"))?;
    let container_row = entities
        .row_of(container, inventory)
        .ok_or_else(|| anyhow!("the item's inventory has no container"))?;
    let fields = entities.row(container, container_row).to_vec();
    let (keys, keys_end) = (u64_of(&fields, 0), u64_of(&fields, 8));
    let (values, values_end) = (u64_of(&fields, 16), u64_of(&fields, 24));
    ensure!(
        keys <= keys_end && values <= values_end,
        "the inventory's lists are back to front"
    );
    let count = ((keys_end - keys) / 2) as usize;
    ensure!(
        (values_end - values) / 8 == count as u64,
        "the inventory's lists differ in length"
    );
    let key_bytes = entities.data(keys, count * 2)?.to_vec();
    let value_bytes = entities.data(values, count * 8)?.to_vec();
    let slots: Vec<u16> = key_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut positions = Vec::with_capacity(count);
    let mut own_slot_data = None;
    for i in 0..count {
        let at = u64_of(&value_bytes, i * 8);
        let slot_data = entities.data(at, 16)?.to_vec();
        positions.push(u32_of(&slot_data, 8));
        if entities.entity_at(u64_of(&slot_data, 0)) == Some(index) {
            own_slot_data = Some(slot_data);
        }
    }
    let own_slot_data =
        own_slot_data.ok_or_else(|| anyhow!("the inventory does not list {item}"))?;
    let slot = slots.iter().max().map_or(0, |&s| s + 1);
    let position = positions.iter().max().map_or(0, |&p| p + 1);

    // Every row the item owns, read before anything is added.
    let owned: Vec<(usize, Vec<u8>)> = (0..entities.tables().len())
        .filter_map(|t| {
            let row = entities.row_of(t, index)?;
            Some((t, entities.row(t, row).to_vec()))
        })
        .collect();

    let copy = new_uuid();
    let copy_index = entities.add_entities(&[copy])?[0];
    // What other rows point at to mean the copy.
    let copy_ref = entities.append(&copy.0);

    let mut new_member_data = member_data.clone();
    new_member_data[8..12].copy_from_slice(&(slot as u32).to_le_bytes());
    let new_member_data_at = entities.append(&new_member_data);

    let mut new_slot_data = own_slot_data;
    new_slot_data[0..8].copy_from_slice(&copy_ref.to_le_bytes());
    new_slot_data[8..12].copy_from_slice(&position.to_le_bytes());
    let new_slot_data_at = entities.append(&new_slot_data);

    let mut new_keys = key_bytes;
    new_keys.extend_from_slice(&slot.to_le_bytes());
    let new_keys_at = entities.append(&new_keys);
    let mut new_values = value_bytes;
    new_values.extend_from_slice(&new_slot_data_at.to_le_bytes());
    let new_values_at = entities.append(&new_values);
    let container_at = (entities.row_offset(container, container_row) - 48) as u64;
    let mut new_fields = fields;
    new_fields[0..8].copy_from_slice(&new_keys_at.to_le_bytes());
    new_fields[8..16].copy_from_slice(&(new_keys_at + new_keys.len() as u64).to_le_bytes());
    new_fields[16..24].copy_from_slice(&new_values_at.to_le_bytes());
    new_fields[24..32].copy_from_slice(&(new_values_at + new_values.len() as u64).to_le_bytes());
    entities.set_data(container_at, &new_fields)?;

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
            MEMBER => row[0..8].copy_from_slice(&new_member_data_at.to_le_bytes()),
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

/// Research: copies an item's nodes in the tree for a new entity, and adds nothing to the
/// entity data. Returns the new entity.
pub fn copy_nodes_only(globals: &mut Lsf, item: Uuid) -> Result<Uuid> {
    let copy = new_uuid();
    copy_nodes(globals, item, copy)?;
    Ok(copy)
}
