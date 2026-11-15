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
/// On an item someone has wielded: that character's identifier itself, sixteen bytes.
const WIELDED: &str = "game.inventory.v0.WieldedComponent";
/// On an item: an offset to its owner's identifier, or none (all ones) for an item in the
/// camp chest.
const OWNEE_CURRENT: &str = "game.v0.OwneeCurrentComponent";
/// On an item: three offsets to owners' identifiers. The game writes the item's owner in
/// the first two and none in the third.
const OWNEE_HISTORY: &str = "game.v2.OwneeHistoryComponent";
/// On whatever holds inventories (a character, a bag, the camp chest): a pair of offsets to
/// a list of its inventories, then an offset to the main one.
const INVENTORIES: &str = "game.inventory.v0.OwnerComponent";
/// A stack's record: a pair of offsets to a list of its items (each an offset to an
/// identifier), and a pair of offsets to its [`STACK_ENTRY`] records.
const STACK: &str = "game.inventory.v0.Stack";
/// One item's amount in a stack: a u16 index into the stack's items, two unused bytes and
/// the amount, a u32.
const STACK_ENTRY: &str = "game.inventory.v0.StackEntry";
/// What an offset holds when it points at nothing.
const NONE: u64 = u64::MAX;

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
/// old copy is moved by the same distance. `pointers` lists where those pointers are; one
/// may point just past the last record, as the end of a list does, and one that points at
/// nothing (all ones) is left alone. Returns the new record's offset.
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
        if old == NONE {
            continue;
        }
        ensure!(
            (old_start..=old_start + len).contains(&old),
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

/// Where every pointer into the stack entries is: each stack's pair of offsets to its own.
fn stack_entry_pointers(entities: &NewAge) -> Result<Vec<u64>> {
    let stacks = table(entities, STACK)?;
    Ok((0..entities.tables()[stacks].rows)
        .flat_map(|row| {
            let at = (entities.row_offset(stacks, row) - 48) as u64;
            [at + 16, at + 24]
        })
        .collect())
}

/// Where every pointer to a stack's record is: the start of every `NewStackComponent` row.
fn stack_pointers(entities: &NewAge) -> Result<Vec<u64>> {
    let stacked = table(entities, STACKED)?;
    Ok((0..entities.tables()[stacked].rows)
        .map(|row| (entities.row_offset(stacked, row) - 48) as u64)
        .collect())
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
    Ok(add_references(entities, &[id])?[0])
}

/// [`add_reference`] for several entities at once, each in a row of its own. A row handed
/// out is free until something points at it, so references wanted together must be made
/// together.
pub fn add_references(entities: &mut NewAge, ids: &[Uuid]) -> Result<Vec<u64>> {
    let table_id = table(entities, ENTITY_TABLE)?;
    let rows = entities.tables()[table_id].rows;
    let start = (entities.row_offset(table_id, 0) - 48) as u64;
    let list = entities.entity_list_offset();
    // Rows of the entity table that are not the entity list as it now stands.
    let candidates: Vec<u64> = (0..rows as u64)
        .map(|r| start + r * 16)
        .filter(|&at| !(list.0..list.1).contains(&at))
        .take(4096)
        .collect();
    let free = entities.unreferenced(&candidates);
    ensure!(
        free.len() >= ids.len(),
        "not enough free rows in the entity table"
    );
    for (at, id) in free.iter().zip(ids) {
        entities.set_data(*at, &id.0)?;
    }
    Ok(free[..ids.len()].to_vec())
}

/// Where a new item goes: an inventory, and who owns what is in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Destination {
    /// The inventory's entity.
    pub inventory: Uuid,
    /// The character the item belongs to, or none for the camp chest, whose items belong
    /// to nobody.
    pub owner: Option<Uuid>,
}

impl Destination {
    /// The main inventory of `holder`, a character or the camp chest, with the character
    /// as the owner of what goes in it.
    pub fn of(entities: &NewAge, holder: Uuid, is_character: bool) -> Result<Destination> {
        let index = entities
            .entity_index(holder)
            .ok_or_else(|| anyhow!("no entity {holder} in the save"))?;
        let lists = table(entities, INVENTORIES)?;
        let row = entities
            .row_of(lists, index)
            .ok_or_else(|| anyhow!("{holder} holds no inventory"))?;
        let main = entities
            .entity_at(u64_of(entities.row(lists, row), 16))
            .ok_or_else(|| anyhow!("{holder}'s main inventory is not an entity"))?;
        ensure!(
            entities.row_of(table(entities, CONTAINER)?, main).is_some(),
            "{holder}'s main inventory has no container"
        );
        Ok(Destination {
            inventory: entities.entity_id(main),
            owner: is_character.then_some(holder),
        })
    }
}

/// Why an item can't be what a new item is copied from, wherever it is, if it can't.
/// Unlike [`why_not_copied`], an equipped, stacked or story item can be: the copy is none of
/// those.
pub fn why_not_donor(entities: &NewAge, index: u32) -> Option<&'static str> {
    let has = |name: &str| {
        entities
            .table(name)
            .is_some_and(|t| entities.row_of(t, index).is_some())
    };
    if !has(crate::party::tables::ITEM) {
        return Some("it is not an item");
    }
    if has(CONTAINER) || has(INVENTORIES) {
        return Some("it holds other items");
    }
    if !has(MEMBER) {
        return Some("it is not in an inventory");
    }
    None
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

/// Copies `donor`, an item in any inventory, into the inventory `to`, as a single item or,
/// with `amount`, as a stack of that many. The copy belongs to `to`'s owner and is not
/// equipped, a story item or part of the donor's stack, whatever the donor is. Returns the
/// copy's entity.
pub fn copy_into(
    globals: &mut Lsf,
    entities: &mut NewAge,
    donor: Uuid,
    to: &Destination,
    amount: Option<u32>,
) -> Result<Uuid> {
    let index = entities
        .entity_index(donor)
        .ok_or_else(|| anyhow!("no entity {donor} in the save"))?;
    if let Some(reason) = why_not_donor(entities, index) {
        anyhow::bail!("{donor} can't be copied: {reason}");
    }
    item_nodes(globals, donor)?;
    let inventory = entities
        .entity_index(to.inventory)
        .ok_or_else(|| anyhow!("no inventory {} in the save", to.inventory))?;
    let container_row = entities
        .row_of(table(entities, CONTAINER)?, inventory)
        .ok_or_else(|| anyhow!("{} is not an inventory", to.inventory))?;
    let owner = match to.owner {
        Some(owner) => Some(
            entities
                .entity_index(owner)
                .ok_or_else(|| anyhow!("no owner {owner} in the save"))?,
        ),
        None => None,
    };
    if amount.is_some() {
        table(entities, STACKED)?;
        table(entities, STACK_MEMBER)?;
        table(entities, STACK)?;
        table(entities, STACK_ENTRY)?;
    }
    // Whether the donor already belongs to the copy's owner.
    let donor_owner = entities
        .table(OWNEE_CURRENT)
        .and_then(|t| Some((t, entities.row_of(t, index)?)))
        .and_then(|(t, row)| entities.entity_at(u64_of(entities.row(t, row), 0)));
    let same_owner = donor_owner == owner;

    // Every row the donor owns, read before anything is added.
    let owned: Vec<(usize, Vec<u8>)> = (0..entities.tables().len())
        .filter_map(|t| {
            let row = entities.row_of(t, index)?;
            Some((t, entities.row(t, row).to_vec()))
        })
        .collect();
    // The last four bytes of a slot record, which the game writes the same in all of them.
    let slot_tail = {
        let slots = table(entities, SLOT_DATA)?;
        ensure!(
            entities.tables()[slots].rows > 0,
            "the save has no slot records"
        );
        entities.row(slots, 0)[12..16].to_vec()
    };

    let copy = new_uuid();
    let copy_index = entities.add_entities(&[copy])?[0];
    let mut wanted = vec![copy, to.inventory];
    wanted.extend(to.owner);
    let refs = add_references(entities, &wanted)?;
    let (copy_ref, inventory_ref) = (refs[0], refs[1]);
    let owner_ref = refs.get(2).copied().unwrap_or(NONE);

    // A slot in the inventory, after the last, and a position after the last.
    let (keys, values) = container_arrays(entities, container_row)?;
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

    let mut member_data = inventory_ref.to_le_bytes().to_vec();
    member_data.extend_from_slice(&(slot as u32).to_le_bytes());
    member_data.extend_from_slice(&[0; 4]);
    let member_data_at = add_member_data(entities, &member_data)?;

    let mut slot_data = copy_ref.to_le_bytes().to_vec();
    slot_data.extend_from_slice(&position.to_le_bytes());
    slot_data.extend_from_slice(&slot_tail);
    let slot_data_at = add_slot_data(entities, &slot_data)?;

    // The inventory's lists, read again now that the pointers in them have moved.
    let (mut keys, mut values) = container_arrays(entities, container_row)?;
    keys.extend_from_slice(&slot.to_le_bytes());
    values.extend_from_slice(&slot_data_at.to_le_bytes());
    set_container_arrays(entities, container_row, &keys, &values)?;

    // The owner lists what it owns.
    if let Some(owner) = owner {
        for name in OWNER_LISTS {
            let Some(table) = entities.table(name) else {
                continue;
            };
            let Some(row) = entities.row_of(table, owner) else {
                continue;
            };
            let fields = entities.row(table, row).to_vec();
            let (begin, end) = (u64_of(&fields, 0), u64_of(&fields, 8));
            let mut list = match begin != NONE && begin <= end && (end - begin).is_multiple_of(8) {
                true => entities.data(begin, (end - begin) as usize)?.to_vec(),
                false => Vec::new(),
            };
            list.extend_from_slice(&copy_ref.to_le_bytes());
            let at = entities.append(&list);
            let row_at = (entities.row_offset(table, row) - 48) as u64;
            let mut pair = at.to_le_bytes().to_vec();
            pair.extend_from_slice(&(at + list.len() as u64).to_le_bytes());
            entities.set_data(row_at, &pair)?;
        }
    }

    for (table, mut row) in owned {
        match entities.tables()[table].name.as_str() {
            // Added below, or not the copy's.
            MEMBER | WIELDING | STORY | STACKED | STACK_MEMBER => continue,
            WIELDED if !same_owner => continue,
            SIGHT => row.copy_from_slice(&new_uuid().0),
            OWNEE_CURRENT => row[0..8].copy_from_slice(&owner_ref.to_le_bytes()),
            OWNEE_HISTORY if row.len() >= 24 => {
                row[0..8].copy_from_slice(&owner_ref.to_le_bytes());
                row[8..16].copy_from_slice(&owner_ref.to_le_bytes());
                row[16..24].copy_from_slice(&NONE.to_le_bytes());
            }
            _ => {}
        }
        entities
            .add_row(table, Some(copy_index), &row)
            .with_context(|| entities.tables()[table].name.clone())?;
    }
    let member = table(entities, MEMBER)?;
    entities.add_row(member, Some(copy_index), &member_data_at.to_le_bytes())?;
    if let Some(amount) = amount {
        add_stack(entities, copy_index, copy_ref, amount)?;
    }

    copy_nodes(globals, donor, copy)?;
    Ok(copy)
}

/// Makes the item `item` a stack of `amount` on its own: a stack record listing it, with
/// one entry.
fn add_stack(entities: &mut NewAge, item: u32, item_ref: u64, amount: u32) -> Result<()> {
    let mut entry = vec![0u8; 8];
    entry[4..8].copy_from_slice(&amount.to_le_bytes());
    let pointers = stack_entry_pointers(entities)?;
    let entry_at = add_record(entities, STACK_ENTRY, &entry, &pointers)?;
    let items_at = entities.append(&item_ref.to_le_bytes());
    let mut stack = Vec::with_capacity(32);
    for value in [items_at, items_at + 8, entry_at, entry_at + 8] {
        stack.extend_from_slice(&value.to_le_bytes());
    }
    let pointers = stack_pointers(entities)?;
    let stack_at = add_record(entities, STACK, &stack, &pointers)?;
    let stacked = table(entities, STACKED)?;
    entities.add_row(stacked, Some(item), &stack_at.to_le_bytes())?;
    let member = table(entities, STACK_MEMBER)?;
    entities.add_row(member, Some(item), &item_ref.to_le_bytes())?;
    Ok(())
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

/// Copies `item` into the same inventory, then makes the copy into the item made from
/// `template`, whose game name is `stats`. Returns the copy.
pub fn copy_item_to(
    globals: &mut Lsf,
    entities: &mut NewAge,
    item: Uuid,
    template: &str,
    stats: &str,
) -> Result<Uuid> {
    ensure!(
        can_become_another(globals, item),
        "only an item made from one of the game's own templates can be copied as another"
    );
    let copy = copy_item(globals, entities, item)?;
    make_into(globals, entities, copy, template, stats)?;
    Ok(copy)
}

/// Makes `item`, a copy made from one of the game's own templates, into the item made from
/// `template`, whose game name is `stats`: its nodes and its template and visual rows name
/// the new template. The game fills in the rest from the template when the save loads.
pub fn make_into(
    globals: &mut Lsf,
    entities: &mut NewAge,
    item: Uuid,
    template: &str,
    stats: &str,
) -> Result<()> {
    let template_id =
        Uuid::parse(template).ok_or_else(|| anyhow!("{template} is not a template identifier"))?;
    ensure!(
        can_become_another(globals, item),
        "only an item made from one of the game's own templates can be made into another"
    );
    let index = entities
        .entity_index(item)
        .ok_or_else(|| anyhow!("no entity {item} in the save"))?;

    // The template's identifier as the rows hold it: text, at the end of the data.
    let text = template_id.to_string();
    let text_at = entities.append(text.as_bytes());
    for name in [TEMPLATE, VISUAL] {
        let Some(id) = entities.table(name) else {
            continue;
        };
        let Some(row) = entities.row_of(id, index) else {
            continue;
        };
        let at = (entities.row_offset(id, row) - 48) as u64;
        let mut fields = entities.row(id, row)[..12].to_vec();
        fields[0..8].copy_from_slice(&text_at.to_le_bytes());
        fields[8..12].copy_from_slice(&(text.len() as u32).to_le_bytes());
        entities.set_data(at, &fields)?;
    }

    let (creator, node) = item_nodes(globals, item)?;
    let set = |globals: &mut Lsf, node: NodeId, name: &str, value: Value| -> Result<()> {
        let attr = globals
            .attr(node, name)
            .ok_or_else(|| anyhow!("the item's nodes have no {name}"))?;
        globals.set_value(attr, &value)
    };
    set(globals, creator, "TemplateID", Value::Uuid(template_id))?;
    set(globals, node, "CurrentTemplate", Value::Uuid(template_id))?;
    set(globals, node, "Stats", Value::Text(stats.to_owned()))?;
    Ok(())
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

    /// Written out and read again, as an export does.
    fn reread(globals: &mut Lsf, entities: &NewAge) -> (Lsf, NewAge) {
        entities.store(globals).unwrap();
        let globals = Lsf::parse(&globals.write().unwrap()).unwrap();
        let entities = NewAge::from_lsf(&globals).unwrap();
        (globals, entities)
    }

    fn row_u64(entities: &NewAge, name: &str, item: Uuid, at: usize) -> Option<u64> {
        let index = entities.entity_index(item)?;
        let table = entities.table(name)?;
        let row = entities.row_of(table, index)?;
        Some(u64_of(entities.row(table, row), at))
    }

    #[test]
    fn finds_where_new_items_go() {
        let entities = sample_entities();
        let hero = Destination::of(&entities, uuid(HERO), true).unwrap();
        assert_eq!(
            hero,
            Destination {
                inventory: uuid(HERO_PACK),
                owner: Some(uuid(HERO))
            }
        );
        let chest = Destination::of(&entities, uuid(CHEST), false).unwrap();
        assert_eq!(chest.inventory, uuid(CHEST_INSIDE));
        assert_eq!(chest.owner, None);
    }

    #[test]
    fn copies_an_item_someone_else_has_equipped() {
        let mut globals = sample_globals();
        let mut entities = sample_entities();
        let to = Destination::of(&entities, uuid(HERO), true).unwrap();
        let copy = copy_into(&mut globals, &mut entities, uuid(TRADER_SWORD), &to, None).unwrap();
        make_into(
            &mut globals,
            &mut entities,
            copy,
            "00000000-0000-0000-0000-000000001111",
            "WPN_Longsword_1",
        )
        .unwrap();
        let (globals, entities) = reread(&mut globals, &entities);
        let after = hero_items(&globals, &entities);
        assert!(after.contains(&(
            "WPN_Longsword_1".to_owned(),
            "00000000-0000-0000-0000-000000001111".to_owned()
        )));
        // The trader's sword is as it was, and still equipped.
        let index = entities.entity_index(uuid(TRADER_SWORD)).unwrap();
        assert!(why_not_copied(&entities, index)
            .unwrap()
            .contains("equipped"));
        // The copy is the hero's, not equipped, and listed as the hero's.
        let copy_index = entities.entity_index(copy).unwrap();
        assert_eq!(why_not_copied(&entities, copy_index), None);
        let owner = row_u64(&entities, OWNEE_CURRENT, copy, 0).unwrap();
        assert_eq!(entities.entity_at(owner), entities.entity_index(uuid(HERO)));
        let history = row_u64(&entities, OWNEE_HISTORY, copy, 8).unwrap();
        assert_eq!(
            entities.entity_at(history),
            entities.entity_index(uuid(HERO))
        );
        for name in OWNER_LISTS {
            let begin = row_u64(&entities, name, uuid(HERO), 0).unwrap();
            let end = row_u64(&entities, name, uuid(HERO), 8).unwrap();
            let listed: Vec<Option<u32>> = entities
                .data(begin, (end - begin) as usize)
                .unwrap()
                .chunks_exact(8)
                .map(|p| entities.entity_at(u64_of(p, 0)))
                .collect();
            assert_eq!(listed.len(), 6, "{name}");
            assert_eq!(listed.last(), Some(&Some(copy_index)), "{name}");
            let trader_end = row_u64(&entities, name, uuid(TRADER), 8).unwrap();
            let trader_begin = row_u64(&entities, name, uuid(TRADER), 0).unwrap();
            assert_eq!(trader_end - trader_begin, 16, "the trader's {name}");
        }
    }

    #[test]
    fn copies_a_stack_into_the_camp_chest_as_a_new_stack() {
        let mut globals = sample_globals();
        let mut entities = sample_entities();
        let to = Destination::of(&entities, uuid(CHEST), false).unwrap();
        let copy = copy_into(&mut globals, &mut entities, uuid(POTIONS), &to, Some(7)).unwrap();
        let (globals, entities) = reread(&mut globals, &entities);
        let party = party::read(&globals, &entities).unwrap();
        let chest = party
            .inventories
            .iter()
            .find(|i| i.holder == Holder::CampChest)
            .unwrap();
        let potion = chest.items.iter().find(|i| i.entity == copy).unwrap();
        assert_eq!(potion.stats, "OBJ_Potion_Healing");
        assert!(potion.stacked);
        assert_eq!(potion.amount, 7);
        // The hero's potions are as they were.
        let hero = party
            .inventories
            .iter()
            .find(|i| i.holder == Holder::Character(0))
            .unwrap();
        let potions = hero
            .items
            .iter()
            .find(|i| i.entity == uuid(POTIONS))
            .unwrap();
        assert_eq!(potions.amount, 3);
        // Nobody owns what is in the camp chest.
        assert_eq!(row_u64(&entities, OWNEE_CURRENT, copy, 0), Some(NONE));
        // The copy is its own stack.
        let member = row_u64(&entities, STACK_MEMBER, copy, 0).unwrap();
        assert_eq!(member, row_u64(&entities, STACK_MEMBER, copy, 0).unwrap());
        assert_eq!(entities.entity_at(member), entities.entity_index(copy));
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
