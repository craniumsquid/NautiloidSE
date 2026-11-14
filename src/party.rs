//! The party as a save's entity data describes it, and the edits the game has been seen to
//! accept.
//!
//! Every field here was located by matching a number the game shows, and every setter makes
//! a change that was then loaded in the game on build `4.1.1.7631656` and seen to take
//! effect. `docs/FORMAT.md` has the record.
//!
//! Some of what the game shows is worked out again on every load and so cannot be edited
//! where it is stored: the ability score totals (from the points allocated at character
//! creation, which can) and the camp supplies total (from the food items).

use crate::lsf::Lsf;
use crate::newage::{NewAge, TableId};
use crate::value::Uuid;
use anyhow::{anyhow, ensure, Result};
use std::collections::HashMap;

/// Tables and field offsets in the entity data.
pub mod tables {
    /// Rows of 12 bytes. The first two u32 both hold the character's total experience.
    /// Only party members have a row.
    pub const EXPERIENCE: &str = "game.experience.v0.ExperienceComponent";
    pub const EXPERIENCE_TOTAL: [usize; 2] = [0, 4];
    /// Rows of 4 bytes: the level the character is entitled to. Above the character level,
    /// the game offers a level-up.
    pub const ENTITLED_LEVEL: &str = "game.experience.v0.AvailableLevelComponent";
    /// Rows of 4 bytes: the character level.
    pub const LEVEL: &str = "game.stats.v0.LevelComponent";
    /// Rows of 36 bytes. Strength to Charisma are six u32 from offset 8. These are totals
    /// that the game recalculates on load.
    pub const STATS: &str = "game.stats.v3.StatsComponent";
    pub const STATS_ABILITIES: usize = 8;
    /// Rows of 88 bytes: the name as an offset at 56 and a length at 64, and at 72 a pair of
    /// offsets to seven u32, the points put into each ability above the base of 8, with
    /// Strength at index 1.
    pub const CREATION: &str = "game.character_creation.v1.CharacterCreationStatsComponent";
    pub const CREATION_NAME: usize = 56;
    pub const CREATION_NAME_LEN: usize = 64;
    pub const CREATION_ABILITIES: usize = 72;
    /// One row of 4 bytes: the camp supplies total, which the game recalculates on load.
    pub const CAMP_SUPPLIES: &str = "game.camp.v0.TotalSuppliesComponent";
    /// One row, owned by the party's own entity.
    pub const PARTY: &str = "game.party.v0.CompositionComponent";
    /// Rows of 16 bytes: a pair of offsets to 64-byte entries. An entry is a resource
    /// identifier, then the amount at 24 and the maximum at 32, both f64.
    pub const RESOURCES: &str = "game.action_resources.v1.Component";
    pub const RESOURCE_LEN: usize = 64;
    pub const RESOURCE_AMOUNT: usize = 24;
    pub const RESOURCE_MAX: usize = 32;
    /// Rows of 8 bytes, one per stack of items: an offset to a row of [`STACK`].
    pub const STACK_OF: &str = "game.inventory.v0.NewStackComponent";
    /// Rows of 32 bytes: a pair of offsets to the stack's items (each an offset to an entity
    /// identifier) and a pair of offsets to its rows of [`STACK_ENTRY`].
    pub const STACK: &str = "game.inventory.v0.Stack";
    /// Rows of 8 bytes: a u16 index into the stack's items, two unused bytes, a u32 amount.
    pub const STACK_ENTRY: &str = "game.inventory.v0.StackEntry";
    pub const STACK_ENTRY_AMOUNT: usize = 4;
    /// Rows of 8 bytes, one per item in an inventory: an offset to a row of [`MEMBER_DATA`].
    pub const MEMBER: &str = "game.inventory.v0.MemberComponent";
    /// Rows of 16 bytes: an offset to the inventory's entity identifier, then the slot.
    pub const MEMBER_DATA: &str = "game.inventory.v0.MemberData";
    /// Rows of 8 bytes, one per inventory: an offset to its owner's entity identifier.
    pub const INVENTORY_OWNER: &str = "game.inventory.v1.IsOwnedComponent";
    /// Rows of 48 bytes, on a companion: three pairs of offsets. The first is to a list of
    /// offsets to the entities approved of (the players' characters), the second to an i32
    /// rating for each, the third to 16-byte identifiers, one for each thing that has moved
    /// the rating. Every field is all ones on a companion who has no rating yet.
    pub const APPROVAL: &str = "game.approval.v0.Ratings";
    /// Rows of 4 bytes, on a food item: the camp supplies one of it is worth.
    pub const SUPPLY: &str = "game.camp.v0.SupplyComponent";
    /// One row, on the camp chest.
    pub const CAMP_CHEST: &str = "game.camp.v1.ChestComponent";
    /// One byte, on every item. A character can be in an inventory too (a carried corpse,
    /// in one sample), and has none.
    pub const ITEM: &str = "game.item.v0.ItemComponent";
}

pub const ABILITIES: [&str; 6] = [
    "Strength",
    "Dexterity",
    "Constitution",
    "Intelligence",
    "Wisdom",
    "Charisma",
];
/// What every ability starts at before points are allocated.
pub const ABILITY_BASE: u32 = 8;
/// The most points this editor will put into one ability. The game's own limit at character
/// creation is 7; it loaded 9 without complaint.
pub const MAX_ALLOCATED: u32 = 22;
/// The highest level this editor will entitle a character to. The game stops at 12; mods
/// raise that to 20.
pub const MAX_LEVEL: u32 = 20;
/// The range the game keeps approval in.
pub const APPROVAL_MIN: i32 = -50;
pub const APPROVAL_MAX: i32 = 100;
/// The template of the gold item.
pub const GOLD_TEMPLATE: &str = "1c3c9c74-34a1-4685-989e-410dc080be6f";
/// The resource on the party's entity that counts Inspiration.
pub const INSPIRATION: &str = "c38dea32-f5cb-4e38-a341-eb3fa653fb3e";

/// Gold a character carries. Gold is an item like any other: a character can hold several
/// stacks, some of them inside bags, and the game's total is their sum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldStack {
    /// The gold item's entity, which identifies the stack for [`set_gold`].
    pub item: Uuid,
    pub amount: u32,
    /// Whether it is inside a bag or other container the character carries.
    pub in_container: bool,
}

/// What a companion thinks of one of the players' characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    /// The character approved of.
    pub of: Uuid,
    pub rating: i32,
}

/// Something a character carries or the camp chest holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The item's entity, which identifies it for [`set_amount`].
    pub entity: Uuid,
    /// The game's own name for what it is, for example `FOOD_FRUIT_APPLE`. The names shown in
    /// the game are in its language files, not in the save.
    pub stats: String,
    /// How many there are: 1 for an item that is not a stack.
    pub amount: u32,
    /// Whether the amount can be changed. Only a stack has one stored.
    pub stacked: bool,
    /// Whether it is inside a bag or other container.
    pub in_container: bool,
    /// For food, the camp supplies one of it is worth.
    pub supply: Option<u32>,
}

/// Who holds an inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Holder {
    /// A character, by position in [`Party::characters`].
    Character(usize),
    CampChest,
}

/// What one holder has. Gold a character carries is not listed here: it is that
/// character's [`Character::gold`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub holder: Holder,
    /// By name, then by entity, so the order is the same on every read.
    pub items: Vec<Item>,
}

/// A character the player controls: a party member, or a companion waiting at camp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Character {
    /// The entity's identifier in the save.
    pub entity: Uuid,
    /// The template the character was created from: a companion's fixed identifier, or the
    /// base template of a custom character.
    pub template: String,
    /// The name, for characters that have a character creation record.
    pub name: Option<String>,
    pub level: u32,
    /// The level the character is entitled to. Above `level`, the game offers a level-up.
    pub entitled_level: u32,
    /// Total experience, for a character in the party. The game shows experience into the
    /// current level. It keeps none for a companion at camp, who has `None`.
    pub experience: Option<u32>,
    /// Ability score totals, Strength to Charisma, with every bonus applied. Read only: the
    /// game recalculates them on load.
    pub abilities: [u32; 6],
    /// Points put into each ability above [`ABILITY_BASE`], Strength to Charisma, for
    /// characters that have a character creation record.
    pub allocated: Option<[u32; 6]>,
    pub gold: Vec<GoldStack>,
    /// A companion's approval of each player's character: one entry in a game for one
    /// player. Empty for the player's own character and a companion not yet met.
    pub approval: Vec<Approval>,
}

impl Character {
    pub fn gold_total(&self) -> u64 {
        self.gold.iter().map(|g| g.amount as u64).sum()
    }

    /// Whether the character is in the party, not waiting at camp.
    pub fn in_party(&self) -> bool {
        self.experience.is_some()
    }

    /// A name to show: the stored one, or the companion's for a character played as an
    /// origin, whose record has none.
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        self.companion().unwrap_or("Unnamed character").to_owned()
    }

    /// The companion this character is, whether recruited or played as an origin.
    pub fn companion(&self) -> Option<&'static str> {
        COMPANIONS
            .iter()
            .find(|(template, _)| *template == self.template)
            .map(|(_, name)| *name)
    }
}

/// Companions by template. Each pairing was read from a sample save in which the character
/// has a stored name.
const COMPANIONS: [(&str, &str); 6] = [
    ("3ed74f06-3c60-42dc-83f6-f034cb47c679", "Shadowheart"),
    ("c7c13742-bacd-460a-8f65-f864fe41f255", "Astarion"),
    ("2c76687d-93a2-477b-8b18-8a14b549304c", "Karlach"),
    ("ad9af97d-75da-406a-ae13-7071c563f604", "Gale"),
    ("c774d764-4a17-48dc-b470-32ace9ce447d", "Wyll"),
    ("58a69333-40bf-8358-1d17-fff240d7fb12", "Lae'zel"),
];

/// The party's Inspiration points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inspiration {
    pub amount: u32,
    /// The most the save allows. The game's own cap is 4; mods raise it.
    pub max: u32,
}

/// The party, the companions at camp, and what the party shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Party {
    /// The party in the order the save lists it, then the companions at camp.
    pub characters: Vec<Character>,
    /// What each character and the camp chest hold. A holder with nothing has no entry.
    pub inventories: Vec<Inventory>,
    pub inspiration: Option<Inspiration>,
    /// Read only: the game recalculates it from the food items on load.
    pub camp_supplies: Option<u32>,
}

impl Party {
    /// The gold the party carries, which is the number the game shows. What companions at
    /// camp carry is not part of it.
    pub fn gold_total(&self) -> u64 {
        self.characters
            .iter()
            .filter(|c| c.in_party())
            .map(Character::gold_total)
            .sum()
    }
}

fn u32_of(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
fn u64_of(bytes: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(b)
}

/// The template of every character and item, by entity identifier, from the node tree.
fn templates(globals: &Lsf) -> HashMap<Uuid, String> {
    let mut out = HashMap::new();
    for path in [
        ["Characters", "CharacterFactory", "Creators"],
        ["Items", "ItemFactory", "Creators"],
    ] {
        let Some(creators) = globals.find(&path) else {
            continue;
        };
        for creator in globals.children_named(creators, "Creator") {
            if let (Some(entity), Some(template)) = (
                globals.get(creator, "Entity").and_then(|v| v.as_uuid()),
                globals.get(creator, "TemplateID"),
            ) {
                let template = match template.as_uuid() {
                    Some(id) => id.to_string(),
                    None => template.as_str().unwrap_or_default().to_owned(),
                };
                out.insert(entity, template);
            }
        }
    }
    out
}

/// The game's name for every item, by entity identifier, from the node tree. The item
/// factory lists its creators and its items in the same order.
fn stats_names(globals: &Lsf) -> HashMap<Uuid, String> {
    let mut out = HashMap::new();
    let (Some(creators), Some(items)) = (
        globals.find(&["Items", "ItemFactory", "Creators"]),
        globals.find(&["Items", "ItemFactory", "Items"]),
    ) else {
        return out;
    };
    let creators = globals.children_named(creators, "Creator");
    let items = globals.children_named(items, "Item");
    for (creator, item) in creators.zip(items) {
        if let (Some(entity), Some(stats)) = (
            globals.get(creator, "Entity").and_then(|v| v.as_uuid()),
            globals.get(item, "Stats"),
        ) {
            out.insert(entity, stats.as_str().unwrap_or_default().to_owned());
        }
    }
    out
}

/// Where a stacked item's amount is and what it holds.
struct Stacked {
    /// Where its stack entry is in the data section.
    entry: u64,
    amount: u32,
}

/// How items are tied to stacks, inventories and owners.
#[derive(Default)]
struct Inventories {
    /// Item entity to its stack entry.
    stacked: HashMap<u32, Stacked>,
    /// Item entity to the inventory entity it is in.
    inventory_of: HashMap<u32, u32>,
    /// Inventory entity to the entity that owns it.
    owner_of: HashMap<u32, u32>,
}

impl Inventories {
    fn read(entities: &NewAge) -> Result<Inventories> {
        let mut out = Inventories::default();
        let table = |name: &str| entities.table(name);

        // A save with no stacks or no inventories simply lacks the tables. Offsets are
        // followed wherever they lead: the game keeps what they point at in a table, and this
        // editor appends what it adds to the end of the data.
        if let Some(stack_of) = table(tables::STACK_OF) {
            for (row, _) in entities.owned_rows(stack_of) {
                let target = u64_of(entities.row(stack_of, row), 0);
                let fields = entities.data(target, 32)?;
                let (items, items_end) = (u64_of(fields, 0), u64_of(fields, 8));
                let (entries, entries_end) = (u64_of(fields, 16), u64_of(fields, 24));
                ensure!(
                    items <= items_end && entries <= entries_end,
                    "a stack's lists are back to front"
                );
                let item_count = ((items_end - items) / 8) as usize;
                let item_offsets = entities.data(items, item_count * 8)?;
                ensure!(
                    (entries_end - entries).is_multiple_of(8),
                    "a stack's entries are not whole"
                );
                for at in (entries..entries_end).step_by(8) {
                    let fields = entities.data(at, 8)?;
                    let index = u16::from_le_bytes([fields[0], fields[1]]) as usize;
                    ensure!(
                        index < item_count,
                        "a stack entry names an item the stack lacks"
                    );
                    // An item that is not in the entity list is skipped, not an error.
                    if let Some(item) = entities.entity_at(u64_of(item_offsets, index * 8)) {
                        out.stacked.insert(
                            item,
                            Stacked {
                                entry: at,
                                amount: u32_of(fields, tables::STACK_ENTRY_AMOUNT),
                            },
                        );
                    }
                }
            }
        }

        if let Some(member) = table(tables::MEMBER) {
            for (row, item) in entities.owned_rows(member) {
                let target = u64_of(entities.row(member, row), 0);
                let inventory = u64_of(entities.data(target, 16)?, 0);
                if let Some(inventory) = entities.entity_at(inventory) {
                    out.inventory_of.insert(item, inventory);
                }
            }
        }
        if let Some(owned) = table(tables::INVENTORY_OWNER) {
            for (row, inventory) in entities.owned_rows(owned) {
                if let Some(owner) = entities.entity_at(u64_of(entities.row(owned, row), 0)) {
                    out.owner_of.insert(inventory, owner);
                }
            }
        }
        Ok(out)
    }

    /// The entity that holds `item`, and whether it is inside a container that entity
    /// carries. Containers are followed outwards until `is_holder` accepts an owner.
    fn holder(&self, item: u32, is_holder: impl Fn(u32) -> bool) -> Option<(u32, bool)> {
        let mut current = item;
        // Deeper nesting than this would be a loop in the data.
        for depth in 0..16 {
            let owner = *self.owner_of.get(self.inventory_of.get(&current)?)?;
            if is_holder(owner) {
                return Some((owner, depth > 0));
            }
            current = owner;
        }
        None
    }
}

fn required(entities: &NewAge, name: &str) -> Result<TableId> {
    entities
        .table(name)
        .ok_or_else(|| anyhow!("the save has no {name} table"))
}

/// The entry for resource `id` in the resources of `entity`: its offset in the data section.
fn resource(entities: &NewAge, entity: u32, id: Uuid) -> Result<Option<u64>> {
    let Some(table) = entities.table(tables::RESOURCES) else {
        return Ok(None);
    };
    let Some(row) = entities.row_of(table, entity) else {
        return Ok(None);
    };
    let fields = entities.row(table, row);
    let (begin, end) = (u64_of(fields, 0), u64_of(fields, 8));
    ensure!(
        begin <= end && (end - begin).is_multiple_of(tables::RESOURCE_LEN as u64),
        "the party's resources are not a whole number of entries"
    );
    for at in (begin..end).step_by(tables::RESOURCE_LEN) {
        if entities.data(at, 16)? == id.0 {
            return Ok(Some(at));
        }
    }
    Ok(None)
}

fn exact4(bytes: &[u8]) -> [u8; 4] {
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// A companion's ratings: for each, the character approved of and where the rating is in
/// the data section.
fn approvals(entities: &NewAge, companion: u32) -> Result<Vec<(Uuid, u64)>> {
    let Some(table) = entities.table(tables::APPROVAL) else {
        return Ok(Vec::new());
    };
    let Some(row) = entities.row_of(table, companion) else {
        return Ok(Vec::new());
    };
    let fields = entities.row(table, row);
    let (of, of_end) = (u64_of(fields, 0), u64_of(fields, 8));
    let (ratings, ratings_end) = (u64_of(fields, 16), u64_of(fields, 24));
    if of == u64::MAX {
        return Ok(Vec::new());
    }
    ensure!(
        of <= of_end && ratings <= ratings_end && (of_end - of) / 8 == (ratings_end - ratings) / 4,
        "a companion's approval lists do not match"
    );
    let mut out = Vec::new();
    for i in 0..(of_end - of) / 8 {
        let target = u64_of(entities.data(of + i * 8, 8)?, 0);
        let entity = entities
            .entity_at(target)
            .ok_or_else(|| anyhow!("an approval rating is of no entity in the save"))?;
        out.push((entities.entity_id(entity), ratings + i * 4));
    }
    Ok(out)
}

fn inspiration_id() -> Uuid {
    Uuid::parse(INSPIRATION).unwrap_or(Uuid([0; 16]))
}

/// The entity that stands for the party as a whole.
fn party_entity(entities: &NewAge) -> Option<u32> {
    let table = entities.table(tables::PARTY)?;
    entities.owned_rows(table).next().map(|(_, owner)| owner)
}

/// A whole number held as an f64, as the game stores resource amounts.
fn whole(bytes: &[u8]) -> Result<u32> {
    let mut b = [0u8; 8];
    b.copy_from_slice(bytes);
    let value = f64::from_le_bytes(b);
    ensure!(
        value >= 0.0 && value <= u32::MAX as f64 && value.fract() == 0.0,
        "a resource amount of {value}"
    );
    Ok(value as u32)
}

/// Reads the party from a save's `Globals.lsf` and the entity data inside it.
pub fn read(globals: &Lsf, entities: &NewAge) -> Result<Party> {
    let templates = templates(globals);
    let experience = required(entities, tables::EXPERIENCE)?;
    // The party is whoever has experience.
    let mut members: Vec<u32> = entities
        .owned_rows(experience)
        .map(|(_, owner)| owner)
        .collect();
    let party_size = members.len();
    let creation = entities.table(tables::CREATION);
    // Everyone else with a character creation record is a companion waiting at camp. They
    // keep their level, their ability points and what they carry, but no experience.
    if let Some(creation) = creation {
        let has = |name: &str, entity: u32| {
            entities
                .table(name)
                .is_some_and(|table| entities.row_of(table, entity).is_some())
        };
        for (_, owner) in entities.owned_rows(creation) {
            let complete = [tables::LEVEL, tables::ENTITLED_LEVEL, tables::STATS]
                .iter()
                .all(|name| has(name, owner));
            if complete && !members.contains(&owner) {
                members.push(owner);
            }
        }
    }

    let mut characters = Vec::with_capacity(members.len());
    for (position, &index) in members.iter().enumerate() {
        let entity = entities.entity_id(index);
        let mut abilities = [0u32; 6];
        for (i, ability) in abilities.iter_mut().enumerate() {
            *ability = entities.get_u32(tables::STATS, index, tables::STATS_ABILITIES + i * 4)?;
        }
        let (mut name, mut allocated) = (None, None);
        if let Some(row) = creation.and_then(|table| Some((table, entities.row_of(table, index)?)))
        {
            let fields = entities.row(row.0, row.1);
            let name_len = u32_of(fields, tables::CREATION_NAME_LEN) as usize;
            let bytes = entities.data(u64_of(fields, tables::CREATION_NAME), name_len)?;
            name = std::str::from_utf8(bytes)
                .ok()
                .filter(|n| !n.is_empty())
                .map(str::to_owned);
            let points =
                entities.get_array_u32(tables::CREATION, index, tables::CREATION_ABILITIES)?;
            ensure!(
                points.len() == 7,
                "a character has {} ability allocations, not 7",
                points.len()
            );
            let mut six = [0u32; 6];
            six.copy_from_slice(&points[1..]);
            allocated = Some(six);
        }
        characters.push(Character {
            entity,
            template: templates.get(&entity).cloned().unwrap_or_default(),
            name,
            level: entities.get_u32(tables::LEVEL, index, 0)?,
            entitled_level: entities.get_u32(tables::ENTITLED_LEVEL, index, 0)?,
            experience: match position < party_size {
                true => Some(entities.get_u32(
                    tables::EXPERIENCE,
                    index,
                    tables::EXPERIENCE_TOTAL[0],
                )?),
                false => None,
            },
            abilities,
            allocated,
            gold: Vec::new(),
            approval: approvals(entities, index)?
                .into_iter()
                .map(|(of, at)| {
                    Ok(Approval {
                        of,
                        rating: i32::from_le_bytes(exact4(entities.data(at, 4)?)),
                    })
                })
                .collect::<Result<_>>()?,
        });
    }

    // Gold: every stacked item made from the gold template that one of them holds.
    let inventories = Inventories::read(entities)?;
    for (&item, stacked) in &inventories.stacked {
        let id = entities.entity_id(item);
        if templates.get(&id).map(String::as_str) != Some(GOLD_TEMPLATE) {
            continue;
        }
        if let Some((holder, in_container)) = inventories.holder(item, |e| members.contains(&e)) {
            if let Some(position) = members.iter().position(|&m| m == holder) {
                characters[position].gold.push(GoldStack {
                    item: id,
                    amount: stacked.amount,
                    in_container,
                });
            }
        }
    }
    for character in &mut characters {
        // Largest first, loose before bagged; the identifier settles ties so the order is
        // the same on every read.
        character.gold.sort_by(|a, b| {
            (a.in_container, std::cmp::Reverse(a.amount), a.item.0).cmp(&(
                b.in_container,
                std::cmp::Reverse(b.amount),
                b.item.0,
            ))
        });
    }

    // Everything else they hold, and what is in the camp chest.
    let stats = stats_names(globals);
    let chest = entities
        .table(tables::CAMP_CHEST)
        .and_then(|table| entities.owned_rows(table).next())
        .map(|(_, owner)| owner);
    let supply = entities.table(tables::SUPPLY);
    let is_item: std::collections::HashSet<u32> = entities
        .table(tables::ITEM)
        .map(|t| entities.owned_rows(t).map(|(_, owner)| owner).collect())
        .unwrap_or_default();
    let mut held: HashMap<Holder, Vec<Item>> = HashMap::new();
    for &item in inventories.inventory_of.keys() {
        if !is_item.contains(&item) {
            continue;
        }
        let Some((owner, in_container)) =
            inventories.holder(item, |e| members.contains(&e) || Some(e) == chest)
        else {
            continue;
        };
        let holder = match members.iter().position(|&m| m == owner) {
            Some(position) => Holder::Character(position),
            None => Holder::CampChest,
        };
        let id = entities.entity_id(item);
        let is_gold = templates.get(&id).map(String::as_str) == Some(GOLD_TEMPLATE);
        let stacked = inventories.stacked.get(&item);
        if is_gold && stacked.is_some() && holder != Holder::CampChest {
            // A character's gold is listed with the character.
            continue;
        }
        held.entry(holder).or_default().push(Item {
            entity: id,
            stats: stats.get(&id).cloned().unwrap_or_default(),
            amount: stacked.map_or(1, |s| s.amount),
            stacked: stacked.is_some(),
            in_container,
            supply: supply
                .and_then(|table| entities.row_of(table, item).map(|row| (table, row)))
                .map(|(table, row)| u32_of(entities.row(table, row), 0)),
        });
    }
    let mut inventories: Vec<Inventory> = held
        .into_iter()
        .map(|(holder, mut items)| {
            items.sort_by(|a, b| (&a.stats, a.entity.0).cmp(&(&b.stats, b.entity.0)));
            Inventory { holder, items }
        })
        .collect();
    inventories.sort_by_key(|inventory| match inventory.holder {
        Holder::Character(position) => position,
        Holder::CampChest => usize::MAX,
    });

    let mut inspiration = None;
    if let Some(party) = party_entity(entities) {
        if let Some(at) = resource(entities, party, inspiration_id())? {
            let entry = entities.data(at, tables::RESOURCE_LEN)?;
            inspiration = Some(Inspiration {
                amount: whole(&entry[tables::RESOURCE_AMOUNT..tables::RESOURCE_AMOUNT + 8])?,
                max: whole(&entry[tables::RESOURCE_MAX..tables::RESOURCE_MAX + 8])?,
            });
        }
    }

    let camp_supplies = entities
        .table(tables::CAMP_SUPPLIES)
        .filter(|&t| entities.tables()[t].rows == 1)
        .map(|t| u32_of(entities.row(t, 0), 0));

    Ok(Party {
        characters,
        inventories,
        inspiration,
        camp_supplies,
    })
}

/// The entity of a character, in the party or at camp.
fn character_index(entities: &NewAge, character: Uuid) -> Result<u32> {
    entities
        .entity_index(character)
        .ok_or_else(|| anyhow!("no entity {character} in the save"))
}

/// The entity of a party member.
fn member(entities: &NewAge, character: Uuid) -> Result<u32> {
    let index = character_index(entities, character)?;
    let experience = required(entities, tables::EXPERIENCE)?;
    ensure!(
        entities.row_of(experience, index).is_some(),
        "{character} is not in the party, and the game keeps experience only for the party"
    );
    Ok(index)
}

/// Sets a party member's total experience. The game does not offer a level-up for this on
/// its own; see [`set_entitled_level`].
pub fn set_experience(entities: &mut NewAge, character: Uuid, total: u32) -> Result<()> {
    ensure!(
        total <= i32::MAX as u32,
        "experience can't be more than {}",
        i32::MAX
    );
    let index = member(entities, character)?;
    for offset in tables::EXPERIENCE_TOTAL {
        entities.set_u32(tables::EXPERIENCE, index, offset, total)?;
    }
    Ok(())
}

/// Sets the level a character is entitled to. Above their level, the game offers a level-up
/// and takes the player through the choices itself.
pub fn set_entitled_level(entities: &mut NewAge, character: Uuid, level: u32) -> Result<()> {
    let index = character_index(entities, character)?;
    let current = entities.get_u32(tables::LEVEL, index, 0)?;
    ensure!(
        (current..=MAX_LEVEL).contains(&level),
        "the entitled level must be from the character's level, {current}, to {MAX_LEVEL}"
    );
    entities.set_u32(tables::ENTITLED_LEVEL, index, 0, level)
}

/// Sets the points put into one ability (0 for Strength to 5 for Charisma) above
/// [`ABILITY_BASE`]. The game works the ability score out from this on load and adds the
/// character's other bonuses on top.
pub fn set_allocated(
    entities: &mut NewAge,
    character: Uuid,
    ability: usize,
    points: u32,
) -> Result<()> {
    ensure!(ability < ABILITIES.len(), "there is no ability {ability}");
    ensure!(
        points <= MAX_ALLOCATED,
        "at most {MAX_ALLOCATED} points can go into an ability"
    );
    let index = character_index(entities, character)?;
    let current = entities.get_array_u32(tables::CREATION, index, tables::CREATION_ABILITIES)?;
    ensure!(
        current.len() == 7,
        "the character has {} ability allocations, not 7",
        current.len()
    );
    entities.set_array_u32(
        tables::CREATION,
        index,
        tables::CREATION_ABILITIES,
        ability + 1,
        points,
    )
}

/// Sets what a companion thinks of the character `of`, as [`Approval::of`] gives it.
pub fn set_approval(entities: &mut NewAge, companion: Uuid, of: Uuid, rating: i32) -> Result<()> {
    let index = character_index(entities, companion)?;
    let (_, at) = approvals(entities, index)?
        .into_iter()
        .find(|(id, _)| *id == of)
        .ok_or_else(|| anyhow!("{companion} has no approval rating of {of}"))?;
    entities.set_data(at, &rating.to_le_bytes())
}

/// Sets the amount of a gold stack, named by its item as [`GoldStack::item`] gives it.
pub fn set_gold(entities: &mut NewAge, item: Uuid, amount: u32) -> Result<()> {
    set_amount(entities, item, amount)
}

/// Sets how many a stack of items holds, named by its item as [`Item::entity`] gives it.
pub fn set_amount(entities: &mut NewAge, item: Uuid, amount: u32) -> Result<()> {
    ensure!(
        (1..=i32::MAX as u32).contains(&amount),
        "a stack holds from 1 to {}",
        i32::MAX
    );
    let index = entities
        .entity_index(item)
        .ok_or_else(|| anyhow!("no item {item} in the save"))?;
    let inventories = Inventories::read(entities)?;
    let stacked = inventories
        .stacked
        .get(&index)
        .ok_or_else(|| anyhow!("item {item} is not in a stack"))?;
    let at = stacked.entry + tables::STACK_ENTRY_AMOUNT as u64;
    entities.set_data(at, &amount.to_le_bytes())
}

/// Sets the party's Inspiration points, up to the maximum the save already allows.
pub fn set_inspiration(entities: &mut NewAge, amount: u32) -> Result<()> {
    let party = party_entity(entities).ok_or_else(|| anyhow!("the save has no party entity"))?;
    let at = resource(entities, party, inspiration_id())?
        .ok_or_else(|| anyhow!("the party has no Inspiration resource"))?;
    let max = whole(entities.data(at + tables::RESOURCE_MAX as u64, 8)?)?;
    ensure!(
        amount <= max,
        "this save allows at most {max} Inspiration points"
    );
    entities.set_data(
        at + tables::RESOURCE_AMOUNT as u64,
        &(amount as f64).to_le_bytes(),
    )
}

/// Builds a small party from scratch. Used by tests, which have no real save to start from.
pub mod build {
    use super::*;
    use crate::lsf::build::{self as tree, Node};
    use crate::newage::build::{blob, TableSpec};
    use crate::value::{DataType, Value};

    // Entities, by position in the entity list.
    pub const HERO: u32 = 0;
    pub const COMPANION: u32 = 1;
    pub const PARTY: u32 = 2;
    pub const HERO_GOLD: u32 = 3;
    pub const BAG: u32 = 4;
    pub const BAGGED_GOLD: u32 = 5;
    pub const HERO_PACK: u32 = 6;
    pub const BAG_INSIDE: u32 = 7;
    pub const TRADER: u32 = 8;
    pub const TRADER_PACK: u32 = 9;
    pub const TRADER_GOLD: u32 = 10;
    pub const ROPE: u32 = 11;
    pub const CAMPER: u32 = 12;
    pub const CAMPER_GOLD: u32 = 13;
    pub const CAMPER_PACK: u32 = 14;
    pub const POTIONS: u32 = 15;
    pub const CHEST: u32 = 16;
    pub const CHEST_INSIDE: u32 = 17;
    pub const APPLES: u32 = 18;
    pub const ENTITY_COUNT: u32 = 19;

    pub fn id(entity: u32) -> [u8; 16] {
        let mut id = [0u8; 16];
        id[0] = entity as u8 + 1;
        id[15] = 0xA0;
        id
    }

    pub fn uuid(entity: u32) -> Uuid {
        Uuid(id(entity))
    }

    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn resource_entry(resource: Uuid, amount: f64, max: f64) -> Vec<u8> {
        let mut entry = vec![0u8; 64];
        entry[..16].copy_from_slice(&resource.0);
        entry[24..32].copy_from_slice(&amount.to_le_bytes());
        entry[32..40].copy_from_slice(&max.to_le_bytes());
        entry
    }

    /// A `Globals.lsf` for the sample party: the creators the party reader looks at, and
    /// the entity data from [`sample_entities`].
    pub fn globals_lsf() -> Vec<u8> {
        let creator = |entity: u32, template: &str| {
            Node::new("Creator")
                .attr("Entity", DataType::Uuid, Value::Uuid(uuid(entity)))
                .attr(
                    "TemplateID",
                    DataType::FixedString,
                    Value::Text(template.into()),
                )
        };
        let characters = Node::new("Creators")
            .child(creator(HERO, "e285d7b5-6b09-4857-89aa-a91e9e6898e5"))
            .child(creator(COMPANION, "2c76687d-93a2-477b-8b18-8a14b549304c"))
            .child(creator(TRADER, "00000000-0000-0000-0000-00000000aaaa"))
            .child(creator(CAMPER, "ad9af97d-75da-406a-ae13-7071c563f604"));
        let items = Node::new("Creators")
            .child(creator(HERO_GOLD, GOLD_TEMPLATE))
            .child(creator(BAGGED_GOLD, GOLD_TEMPLATE))
            .child(creator(TRADER_GOLD, GOLD_TEMPLATE))
            .child(creator(CAMPER_GOLD, GOLD_TEMPLATE))
            .child(creator(BAG, "00000000-0000-0000-0000-00000000bbbb"))
            .child(creator(ROPE, "00000000-0000-0000-0000-00000000cccc"))
            .child(creator(POTIONS, "00000000-0000-0000-0000-00000000dddd"))
            .child(creator(APPLES, "00000000-0000-0000-0000-00000000eeee"));
        // The items themselves, in the creators' order, each with the game's name for it.
        let mut item_list = Node::new("Items");
        for stats in [
            "LOOT_Gold_A",
            "LOOT_Gold_A",
            "LOOT_Gold_A",
            "LOOT_Gold_A",
            "OBJ_Bag_AlchemyPouch",
            "OBJ_Rope",
            "OBJ_Potion_Healing",
            "CONS_FOOD_Fruit_Apple",
        ] {
            item_list = item_list.child(Node::new("Item").attr(
                "Stats",
                DataType::FixedString,
                Value::Text(stats.into()),
            ));
        }
        let entities = Node::new(crate::newage::NODE).attr(
            crate::newage::NODE,
            DataType::ScratchBuffer,
            Value::Bytes(sample_entities().to_bytes()),
        );
        tree::lsf(&[
            Node::new("Characters").child(Node::new("CharacterFactory").child(characters)),
            Node::new("Items").child(Node::new("ItemFactory").child(items).child(item_list)),
            entities,
        ])
        .expect("the sample tree is well formed")
    }

    /// The sample `Globals.lsf`, parsed.
    pub fn sample_globals() -> Lsf {
        Lsf::parse(&globals_lsf()).expect("the sample document parses")
    }

    /// Entity data for a hero with 3385 gold loose and 371 in a bag, a companion with none,
    /// a companion at camp with 120, a trader with 500, and a party with 3 of 4 Inspiration.
    /// The hero also carries a bag, a rope and three healing potions, and the camp chest
    /// holds four apples.
    pub fn sample_entities() -> NewAge {
        let entities: Vec<[u8; 16]> = (0..ENTITY_COUNT).map(id).collect();
        let stats = |abilities: [u32; 6]| {
            let mut row = words(&[4, 0]);
            row.extend(words(&abilities));
            row.extend(words(&[6]));
            row
        };
        let specs = vec![
            TableSpec {
                name: tables::EXPERIENCE,
                row_len: 12,
                rows: vec![words(&[5595, 5595, 0]), words(&[5655, 5655, 0])],
                owners: Some(vec![HERO, COMPANION]),
            },
            TableSpec {
                name: tables::LEVEL,
                row_len: 4,
                rows: vec![words(&[5]), words(&[5]), words(&[3]), words(&[4])],
                owners: Some(vec![HERO, COMPANION, TRADER, CAMPER]),
            },
            TableSpec {
                name: tables::ENTITLED_LEVEL,
                row_len: 4,
                rows: vec![words(&[5]), words(&[5]), words(&[3]), words(&[4])],
                owners: Some(vec![HERO, COMPANION, TRADER, CAMPER]),
            },
            TableSpec {
                name: tables::STATS,
                row_len: 36,
                rows: vec![
                    stats([8, 14, 16, 17, 10, 20]),
                    stats([18, 13, 15, 8, 12, 10]),
                    stats([10, 10, 10, 10, 10, 10]),
                    stats([8, 13, 15, 17, 10, 12]),
                ],
                owners: Some(vec![HERO, COMPANION, TRADER, CAMPER]),
            },
            // The hero and the companion at camp have a character creation record; the
            // companion in the party does not.
            TableSpec {
                name: tables::CREATION,
                row_len: 88,
                rows: vec![vec![0; 88]; 2],
                owners: Some(vec![HERO, CAMPER]),
            },
            TableSpec {
                name: "test.v0.NameBytes",
                row_len: 4,
                rows: vec![b"Tav\0".to_vec(), b"Gale".to_vec()],
                owners: None,
            },
            TableSpec {
                name: "test.v0.Allocations",
                row_len: 4,
                rows: [0, 0, 6, 7, 0, 2, 7, 0, 0, 5, 6, 7, 2, 4]
                    .iter()
                    .map(|&p| words(&[p]))
                    .collect(),
                owners: None,
            },
            // The companion at camp rates the hero at 10; the one in the party has no
            // rating yet.
            TableSpec {
                name: tables::APPROVAL,
                row_len: 48,
                rows: vec![vec![0; 48], vec![0xff; 48]],
                owners: Some(vec![CAMPER, COMPANION]),
            },
            TableSpec {
                name: "test.v0.ApprovalOf",
                row_len: 8,
                rows: vec![vec![0; 8]],
                owners: None,
            },
            TableSpec {
                name: "test.v0.ApprovalRatings",
                row_len: 4,
                rows: vec![words(&[10])],
                owners: None,
            },
            TableSpec {
                name: tables::CAMP_SUPPLIES,
                row_len: 4,
                rows: vec![words(&[660])],
                owners: Some(vec![TRADER]),
            },
            TableSpec {
                name: tables::PARTY,
                row_len: 32,
                rows: vec![vec![0; 32]],
                owners: Some(vec![PARTY]),
            },
            TableSpec {
                name: tables::RESOURCES,
                row_len: 16,
                rows: vec![vec![0; 16], vec![0; 16]],
                owners: Some(vec![PARTY, HERO]),
            },
            TableSpec {
                name: "test.v0.ResourceEntries",
                row_len: 64,
                rows: vec![
                    resource_entry(Uuid([9; 16]), 2.0, 2.0),
                    resource_entry(inspiration_id(), 3.0, 4.0),
                    resource_entry(Uuid([7; 16]), 1.0, 1.0),
                ],
                owners: None,
            },
            // Four stacks: the hero's loose gold, the gold in the hero's bag, the trader's
            // and the camp companion's.
            TableSpec {
                name: tables::STACK_OF,
                row_len: 8,
                rows: vec![vec![0; 8]; 6],
                owners: Some(vec![
                    HERO_GOLD,
                    BAGGED_GOLD,
                    TRADER_GOLD,
                    CAMPER_GOLD,
                    POTIONS,
                    APPLES,
                ]),
            },
            TableSpec {
                name: tables::STACK,
                row_len: 32,
                rows: vec![vec![0; 32]; 6],
                owners: None,
            },
            TableSpec {
                name: tables::STACK_ENTRY,
                row_len: 8,
                // The upper half of the first word is unused and holds leftovers.
                rows: vec![
                    words(&[0x4d72_0000, 3385]),
                    words(&[0x4d72_0000, 371]),
                    words(&[0x4d72_0000, 500]),
                    words(&[0x4d72_0000, 120]),
                    words(&[0x4d72_0000, 3]),
                    words(&[0x4d72_0000, 4]),
                ],
                owners: None,
            },
            TableSpec {
                name: "test.v0.StackItems",
                row_len: 8,
                rows: vec![vec![0; 8]; 6],
                owners: None,
            },
            TableSpec {
                name: tables::MEMBER,
                row_len: 8,
                rows: vec![vec![0; 8]; 8],
                owners: Some(vec![
                    HERO_GOLD,
                    BAG,
                    BAGGED_GOLD,
                    TRADER_GOLD,
                    ROPE,
                    CAMPER_GOLD,
                    POTIONS,
                    APPLES,
                ]),
            },
            TableSpec {
                name: tables::MEMBER_DATA,
                row_len: 16,
                rows: vec![vec![0; 16]; 8],
                owners: None,
            },
            TableSpec {
                name: tables::INVENTORY_OWNER,
                row_len: 8,
                rows: vec![vec![0; 8]; 5],
                owners: Some(vec![
                    HERO_PACK,
                    BAG_INSIDE,
                    TRADER_PACK,
                    CAMPER_PACK,
                    CHEST_INSIDE,
                ]),
            },
            TableSpec {
                name: tables::CAMP_CHEST,
                row_len: 48,
                rows: vec![vec![0; 48]],
                owners: Some(vec![CHEST]),
            },
            // Every item has an item component; owners in entity order, as the game keeps
            // them.
            TableSpec {
                name: tables::ITEM,
                row_len: 1,
                rows: vec![vec![0]; 8],
                owners: Some(vec![
                    HERO_GOLD,
                    BAG,
                    BAGGED_GOLD,
                    TRADER_GOLD,
                    ROPE,
                    CAMPER_GOLD,
                    POTIONS,
                    APPLES,
                ]),
            },
            // An apple is worth 3 camp supplies.
            TableSpec {
                name: tables::SUPPLY,
                row_len: 4,
                rows: vec![words(&[3])],
                owners: Some(vec![APPLES]),
            },
        ];
        let mut data = NewAge::parse(blob(&entities, &specs)).unwrap();

        // Fill in the offsets, which are only known once the tables are laid out.
        let at = |data: &NewAge, name: &str, row: usize| {
            (data.row_offset(data.table(name).unwrap(), row) - 48) as u32
        };
        let entity = |data: &NewAge, e: u32| at(data, "core.v0.EntityId", e as usize);
        let mut point = |name: &str, row: usize, offset: usize, target: u32| {
            data.set_row_u32(name, row, offset, target).unwrap();
        };
        let fixed = NewAge::parse(blob(&entities, &specs)).unwrap();

        // The hero is "Tav", the companion at camp "Gale"; each has seven allocations.
        for (row, name_len) in [(0, 3), (1, 4)] {
            point(
                tables::CREATION,
                row,
                56,
                at(&fixed, "test.v0.NameBytes", row),
            );
            point(tables::CREATION, row, 64, name_len);
            let allocations = at(&fixed, "test.v0.Allocations", row * 7);
            point(tables::CREATION, row, 72, allocations);
            point(tables::CREATION, row, 80, allocations + 28);
        }
        let of = at(&fixed, "test.v0.ApprovalOf", 0);
        point("test.v0.ApprovalOf", 0, 0, entity(&fixed, HERO));
        point(tables::APPROVAL, 0, 0, of);
        point(tables::APPROVAL, 0, 8, of + 8);
        let ratings = at(&fixed, "test.v0.ApprovalRatings", 0);
        point(tables::APPROVAL, 0, 16, ratings);
        point(tables::APPROVAL, 0, 24, ratings + 4);
        // The party's resources are the first two entries, the hero's the third.
        let entries = at(&fixed, "test.v0.ResourceEntries", 0);
        point(tables::RESOURCES, 0, 0, entries);
        point(tables::RESOURCES, 0, 8, entries + 128);
        point(tables::RESOURCES, 1, 0, entries + 128);
        point(tables::RESOURCES, 1, 8, entries + 192);
        let stacks = [
            HERO_GOLD,
            BAGGED_GOLD,
            TRADER_GOLD,
            CAMPER_GOLD,
            POTIONS,
            APPLES,
        ];
        for (stack, item) in stacks.into_iter().enumerate() {
            point(tables::STACK_OF, stack, 0, at(&fixed, tables::STACK, stack));
            let items = at(&fixed, "test.v0.StackItems", stack);
            point("test.v0.StackItems", stack, 0, entity(&fixed, item));
            point(tables::STACK, stack, 0, items);
            point(tables::STACK, stack, 8, items + 8);
            let entry = at(&fixed, tables::STACK_ENTRY, stack);
            point(tables::STACK, stack, 16, entry);
            point(tables::STACK, stack, 24, entry + 8);
        }
        let packs = [
            HERO_PACK,
            HERO_PACK,
            BAG_INSIDE,
            TRADER_PACK,
            HERO_PACK,
            CAMPER_PACK,
            HERO_PACK,
            CHEST_INSIDE,
        ];
        for (row, pack) in packs.into_iter().enumerate() {
            point(tables::MEMBER, row, 0, at(&fixed, tables::MEMBER_DATA, row));
            point(tables::MEMBER_DATA, row, 0, entity(&fixed, pack));
        }
        for (row, owner) in [HERO, BAG, TRADER, CAMPER, CHEST].into_iter().enumerate() {
            point(tables::INVENTORY_OWNER, row, 0, entity(&fixed, owner));
        }
        NewAge::parse(data.to_bytes()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn reads_the_party() {
        let party = read(&sample_globals(), &sample_entities()).unwrap();
        assert_eq!(party.characters.len(), 3);
        assert_eq!(party.inspiration, Some(Inspiration { amount: 3, max: 4 }));
        assert_eq!(party.camp_supplies, Some(660));
        assert_eq!(party.gold_total(), 3756);

        let hero = &party.characters[0];
        assert_eq!(hero.entity, uuid(HERO));
        assert_eq!(hero.template, "e285d7b5-6b09-4857-89aa-a91e9e6898e5");
        assert_eq!(hero.name.as_deref(), Some("Tav"));
        assert_eq!(
            (hero.level, hero.entitled_level, hero.experience),
            (5, 5, Some(5595))
        );
        assert_eq!(hero.abilities, [8, 14, 16, 17, 10, 20]);
        assert_eq!(hero.allocated, Some([0, 6, 7, 0, 2, 7]));
        // Loose gold first, then the gold in the bag; the trader's is not the party's.
        assert_eq!(
            hero.gold,
            [
                GoldStack {
                    item: uuid(HERO_GOLD),
                    amount: 3385,
                    in_container: false
                },
                GoldStack {
                    item: uuid(BAGGED_GOLD),
                    amount: 371,
                    in_container: true
                },
            ]
        );

        let companion = &party.characters[1];
        assert_eq!(companion.name, None);
        assert_eq!(
            companion.display_name(),
            "Karlach",
            "named after the template"
        );
        assert_eq!(hero.display_name(), "Tav");
        assert_eq!(companion.allocated, None);
        assert_eq!(companion.abilities, [18, 13, 15, 8, 12, 10]);
        assert!(companion.gold.is_empty());
        assert!(hero.in_party() && companion.in_party());

        // A companion at camp comes after the party. The game keeps no experience for
        // them, and what they carry is not the party's gold.
        let camper = &party.characters[2];
        assert_eq!(camper.entity, uuid(CAMPER));
        assert_eq!(camper.name.as_deref(), Some("Gale"));
        assert!(!camper.in_party());
        assert_eq!(
            (camper.level, camper.entitled_level, camper.experience),
            (4, 4, None)
        );
        assert_eq!(camper.allocated, Some([0, 5, 6, 7, 2, 4]));
        assert_eq!(camper.gold_total(), 120);

        // Approval is a companion's, of the player's character.
        assert_eq!(
            camper.approval,
            [Approval {
                of: uuid(HERO),
                rating: 10
            }]
        );
        assert!(hero.approval.is_empty());
        assert!(companion.approval.is_empty(), "no rating yet");
    }

    #[test]
    fn reads_what_each_holder_has_and_changes_a_stack() {
        let globals = sample_globals();
        let mut entities = sample_entities();
        let party = read(&globals, &entities).unwrap();
        // The hero and the camp chest. The companion at camp carries only gold, which is
        // listed with the character, and the trader is nobody's.
        let holders: Vec<Holder> = party.inventories.iter().map(|i| i.holder).collect();
        assert_eq!(holders, [Holder::Character(0), Holder::CampChest]);
        let names: Vec<(&str, u32, bool)> = party.inventories[0]
            .items
            .iter()
            .map(|i| (i.stats.as_str(), i.amount, i.stacked))
            .collect();
        assert_eq!(
            names,
            [
                ("OBJ_Bag_AlchemyPouch", 1, false),
                ("OBJ_Potion_Healing", 3, true),
                ("OBJ_Rope", 1, false),
            ]
        );
        let apples = &party.inventories[1].items[0];
        assert_eq!(
            (apples.stats.as_str(), apples.amount, apples.supply),
            ("CONS_FOOD_Fruit_Apple", 4, Some(3))
        );

        set_amount(&mut entities, uuid(POTIONS), 12).unwrap();
        set_amount(&mut entities, uuid(APPLES), 40).unwrap();
        assert!(
            set_amount(&mut entities, uuid(ROPE), 2).is_err(),
            "not a stack"
        );
        assert!(set_amount(&mut entities, uuid(POTIONS), 0).is_err());
        let after = read(&globals, &NewAge::parse(entities.to_bytes()).unwrap()).unwrap();
        assert_eq!(after.inventories[0].items[1].amount, 12);
        assert_eq!(after.inventories[1].items[0].amount, 40);
        assert_eq!(after.gold_total(), 3756, "gold is as it was");
    }

    #[test]
    fn approval_can_be_set_below_zero() {
        let globals = sample_globals();
        let mut entities = sample_entities();
        set_approval(&mut entities, uuid(CAMPER), uuid(HERO), -20).unwrap();
        assert!(set_approval(&mut entities, uuid(COMPANION), uuid(HERO), 5).is_err());
        let after = read(&globals, &NewAge::parse(entities.to_bytes()).unwrap()).unwrap();
        assert_eq!(after.characters[2].approval[0].rating, -20);
    }

    #[test]
    fn a_companion_at_camp_can_be_edited_except_for_experience() {
        let globals = sample_globals();
        let mut entities = sample_entities();
        set_entitled_level(&mut entities, uuid(CAMPER), 5).unwrap();
        set_allocated(&mut entities, uuid(CAMPER), 3, 9).unwrap();
        set_gold(&mut entities, uuid(CAMPER_GOLD), 900).unwrap();
        let error = set_experience(&mut entities, uuid(CAMPER), 5000).unwrap_err();
        assert!(error.to_string().contains("not in the party"), "{error}");

        let after = read(&globals, &NewAge::parse(entities.to_bytes()).unwrap()).unwrap();
        let camper = &after.characters[2];
        assert_eq!((camper.level, camper.entitled_level), (4, 5));
        assert_eq!(camper.allocated, Some([0, 5, 6, 9, 2, 4]));
        assert_eq!(camper.gold_total(), 900);
        // The hero's record is beside it and is untouched.
        assert_eq!(after.characters[0].allocated, Some([0, 6, 7, 0, 2, 7]));
        assert_eq!(after.gold_total(), 3756);
    }

    #[test]
    fn edits_read_back_and_touch_nothing_else() {
        let globals = sample_globals();
        let original = sample_entities();
        let mut entities = original.clone();
        set_experience(&mut entities, uuid(HERO), 7100).unwrap();
        set_entitled_level(&mut entities, uuid(HERO), 6).unwrap();
        set_allocated(&mut entities, uuid(HERO), 0, 7).unwrap();
        set_gold(&mut entities, uuid(BAGGED_GOLD), 5000).unwrap();
        set_inspiration(&mut entities, 4).unwrap();

        // Written out and read again, as an export does.
        let entities = NewAge::parse(entities.to_bytes()).unwrap();
        let before = read(&globals, &original).unwrap();
        let after = read(&globals, &entities).unwrap();
        let hero = &after.characters[0];
        assert_eq!(
            (hero.experience, hero.entitled_level, hero.level),
            (Some(7100), 6, 5)
        );
        assert_eq!(hero.allocated, Some([7, 6, 7, 0, 2, 7]));
        assert_eq!(
            hero.abilities, before.characters[0].abilities,
            "totals are the game's"
        );
        // Loose gold is still listed first, however much the bag now holds.
        assert_eq!(
            (hero.gold[0].item, hero.gold[0].amount),
            (uuid(HERO_GOLD), 3385)
        );
        assert_eq!(
            (hero.gold[1].item, hero.gold[1].amount),
            (uuid(BAGGED_GOLD), 5000)
        );
        assert_eq!(after.inspiration, Some(Inspiration { amount: 4, max: 4 }));
        assert_eq!(after.characters[1], before.characters[1]);
        assert_eq!(after.camp_supplies, before.camp_supplies);

        // Five edits: experience twice over, a level, an allocation, a stack amount and
        // eight bytes of a float, plus the checksum. Nothing else.
        let (old, new) = (original.to_bytes(), entities.to_bytes());
        let changed = (0..old.len()).filter(|&i| old[i] != new[i]).count();
        assert!(
            changed <= 8 + 4 + 4 + 4 + 4 + 4 + 8,
            "{changed} bytes changed"
        );
    }

    #[test]
    fn refuses_edits_that_make_no_sense() {
        let mut entities = sample_entities();
        let hero = uuid(HERO);
        assert!(set_experience(&mut entities, hero, u32::MAX).is_err());
        assert!(
            set_experience(&mut entities, uuid(TRADER), 100).is_err(),
            "not in the party"
        );
        assert!(
            set_experience(&mut entities, Uuid([0xEE; 16]), 100).is_err(),
            "no such entity"
        );
        assert!(
            set_entitled_level(&mut entities, hero, 4).is_err(),
            "below the level"
        );
        assert!(set_entitled_level(&mut entities, hero, 21).is_err());
        assert!(
            set_entitled_level(&mut entities, hero, 5).is_ok(),
            "no level-up pending"
        );
        assert!(
            set_allocated(&mut entities, hero, 6, 1).is_err(),
            "six abilities"
        );
        assert!(set_allocated(&mut entities, hero, 0, 23).is_err());
        assert!(
            set_allocated(&mut entities, uuid(COMPANION), 0, 1).is_err(),
            "no character creation record"
        );
        assert!(set_gold(&mut entities, uuid(HERO_GOLD), 0).is_err());
        assert!(
            set_gold(&mut entities, uuid(ROPE), 5).is_err(),
            "not in a stack"
        );
        assert!(
            set_gold(&mut entities, uuid(TRADER_GOLD), 5).is_ok(),
            "any stack can be set"
        );
        assert!(
            set_inspiration(&mut entities, 5).is_err(),
            "over the save's maximum"
        );
        assert!(set_inspiration(&mut entities, 0).is_ok());
    }
}
