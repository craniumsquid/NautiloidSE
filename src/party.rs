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
use anyhow::{anyhow, bail, ensure, Result};
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

/// A party member.
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
    /// Total experience. The game shows experience into the current level.
    pub experience: u32,
    /// Ability score totals, Strength to Charisma, with every bonus applied. Read only: the
    /// game recalculates them on load.
    pub abilities: [u32; 6],
    /// Points put into each ability above [`ABILITY_BASE`], Strength to Charisma, for
    /// characters that have a character creation record.
    pub allocated: Option<[u32; 6]>,
    pub gold: Vec<GoldStack>,
}

impl Character {
    pub fn gold_total(&self) -> u64 {
        self.gold.iter().map(|g| g.amount as u64).sum()
    }

    /// A name to show: the stored one, or the companion's for a character played as an
    /// origin, whose record has none.
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        COMPANIONS
            .iter()
            .find(|(template, _)| *template == self.template)
            .map_or("Unnamed character", |(_, name)| name)
            .to_owned()
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

/// The party and what it shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Party {
    /// In the order the save lists them.
    pub characters: Vec<Character>,
    pub inspiration: Option<Inspiration>,
    /// Read only: the game recalculates it from the food items on load.
    pub camp_supplies: Option<u32>,
}

impl Party {
    pub fn gold_total(&self) -> u64 {
        self.characters.iter().map(Character::gold_total).sum()
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

/// Where a stacked item's amount is and what it holds.
struct Stacked {
    /// Row of [`tables::STACK_ENTRY`].
    entry: usize,
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

        // A save with no stacks or no inventories simply lacks the tables.
        if let (Some(stack_of), Some(stack), Some(entry)) = (
            table(tables::STACK_OF),
            table(tables::STACK),
            table(tables::STACK_ENTRY),
        ) {
            for (row, _) in entities.owned_rows(stack_of) {
                let target = u64_of(entities.row(stack_of, row), 0);
                let Some((found, stack_row, 0)) = entities.locate(target) else {
                    bail!("a stack points outside the stack table");
                };
                ensure!(found == stack, "a stack points outside the stack table");
                let fields = entities.row(stack, stack_row);
                let (items, items_end) = (u64_of(fields, 0), u64_of(fields, 8));
                let (entries, entries_end) = (u64_of(fields, 16), u64_of(fields, 24));
                ensure!(
                    items <= items_end && entries <= entries_end,
                    "a stack's lists are back to front"
                );
                let item_count = ((items_end - items) / 8) as usize;
                let item_offsets = entities.data(items, item_count * 8)?;
                for at in (entries..entries_end).step_by(8) {
                    let Some((found, entry_row, 0)) = entities.locate(at) else {
                        bail!("a stack's entries are outside the stack entry table");
                    };
                    ensure!(
                        found == entry,
                        "a stack's entries are outside the stack entry table"
                    );
                    let fields = entities.row(entry, entry_row);
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
                                entry: entry_row,
                                amount: u32_of(fields, tables::STACK_ENTRY_AMOUNT),
                            },
                        );
                    }
                }
            }
        }

        if let (Some(member), Some(member_data)) =
            (table(tables::MEMBER), table(tables::MEMBER_DATA))
        {
            for (row, item) in entities.owned_rows(member) {
                let target = u64_of(entities.row(member, row), 0);
                if let Some((found, data_row, 0)) = entities.locate(target) {
                    if found == member_data {
                        let inventory = u64_of(entities.row(member_data, data_row), 0);
                        if let Some(inventory) = entities.entity_at(inventory) {
                            out.inventory_of.insert(item, inventory);
                        }
                    }
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
    let members: Vec<u32> = entities
        .owned_rows(experience)
        .map(|(_, owner)| owner)
        .collect();
    let creation = entities.table(tables::CREATION);

    let mut characters = Vec::with_capacity(members.len());
    for &index in &members {
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
            experience: entities.get_u32(tables::EXPERIENCE, index, tables::EXPERIENCE_TOTAL[0])?,
            abilities,
            allocated,
            gold: Vec::new(),
        });
    }

    // Gold: every stacked item made from the gold template that a party member holds.
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
        inspiration,
        camp_supplies,
    })
}

fn member(entities: &NewAge, character: Uuid) -> Result<u32> {
    let index = entities
        .entity_index(character)
        .ok_or_else(|| anyhow!("no entity {character} in the save"))?;
    let experience = required(entities, tables::EXPERIENCE)?;
    ensure!(
        entities.row_of(experience, index).is_some(),
        "{character} is not a party member"
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

/// Sets the level a party member is entitled to. Above their level, the game offers a
/// level-up and takes the player through the choices itself.
pub fn set_entitled_level(entities: &mut NewAge, character: Uuid, level: u32) -> Result<()> {
    let index = member(entities, character)?;
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
    let index = member(entities, character)?;
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

/// Sets the amount of a gold stack, named by its item as [`GoldStack::item`] gives it.
pub fn set_gold(entities: &mut NewAge, item: Uuid, amount: u32) -> Result<()> {
    ensure!(
        (1..=i32::MAX as u32).contains(&amount),
        "a gold stack holds from 1 to {}",
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
    entities.set_row_u32(
        tables::STACK_ENTRY,
        stacked.entry,
        tables::STACK_ENTRY_AMOUNT,
        amount,
    )
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
    pub const ENTITY_COUNT: u32 = 12;

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
            .child(creator(TRADER, "00000000-0000-0000-0000-00000000aaaa"));
        let items = Node::new("Creators")
            .child(creator(HERO_GOLD, GOLD_TEMPLATE))
            .child(creator(BAGGED_GOLD, GOLD_TEMPLATE))
            .child(creator(TRADER_GOLD, GOLD_TEMPLATE))
            .child(creator(BAG, "00000000-0000-0000-0000-00000000bbbb"))
            .child(creator(ROPE, "00000000-0000-0000-0000-00000000cccc"));
        let entities = Node::new(crate::newage::NODE).attr(
            crate::newage::NODE,
            DataType::ScratchBuffer,
            Value::Bytes(sample_entities().to_bytes()),
        );
        tree::lsf(&[
            Node::new("Characters").child(Node::new("CharacterFactory").child(characters)),
            Node::new("Items").child(Node::new("ItemFactory").child(items)),
            entities,
        ])
        .expect("the sample tree is well formed")
    }

    /// The sample `Globals.lsf`, parsed.
    pub fn sample_globals() -> Lsf {
        Lsf::parse(&globals_lsf()).expect("the sample document parses")
    }

    /// Entity data for a hero with 3385 gold loose and 371 in a bag, a companion with none,
    /// a trader with 500, and a party with 3 of 4 Inspiration.
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
                rows: vec![words(&[5]), words(&[5]), words(&[3])],
                owners: Some(vec![HERO, COMPANION, TRADER]),
            },
            TableSpec {
                name: tables::ENTITLED_LEVEL,
                row_len: 4,
                rows: vec![words(&[5]), words(&[5]), words(&[3])],
                owners: Some(vec![HERO, COMPANION, TRADER]),
            },
            TableSpec {
                name: tables::STATS,
                row_len: 36,
                rows: vec![
                    stats([8, 14, 16, 17, 10, 20]),
                    stats([18, 13, 15, 8, 12, 10]),
                    stats([10, 10, 10, 10, 10, 10]),
                ],
                owners: Some(vec![HERO, COMPANION, TRADER]),
            },
            // The hero has a character creation record; the companion does not.
            TableSpec {
                name: tables::CREATION,
                row_len: 88,
                rows: vec![vec![0; 88]],
                owners: Some(vec![HERO]),
            },
            TableSpec {
                name: "test.v0.NameBytes",
                row_len: 3,
                rows: vec![b"Tav".to_vec()],
                owners: None,
            },
            TableSpec {
                name: "test.v0.Allocations",
                row_len: 4,
                rows: [0, 0, 6, 7, 0, 2, 7].iter().map(|&p| words(&[p])).collect(),
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
            // Three stacks: the hero's loose gold, the gold in the hero's bag, the trader's.
            TableSpec {
                name: tables::STACK_OF,
                row_len: 8,
                rows: vec![vec![0; 8]; 3],
                owners: Some(vec![HERO_GOLD, BAGGED_GOLD, TRADER_GOLD]),
            },
            TableSpec {
                name: tables::STACK,
                row_len: 32,
                rows: vec![vec![0; 32]; 3],
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
                ],
                owners: None,
            },
            TableSpec {
                name: "test.v0.StackItems",
                row_len: 8,
                rows: vec![vec![0; 8]; 3],
                owners: None,
            },
            TableSpec {
                name: tables::MEMBER,
                row_len: 8,
                rows: vec![vec![0; 8]; 5],
                owners: Some(vec![HERO_GOLD, BAG, BAGGED_GOLD, TRADER_GOLD, ROPE]),
            },
            TableSpec {
                name: tables::MEMBER_DATA,
                row_len: 16,
                rows: vec![vec![0; 16]; 5],
                owners: None,
            },
            TableSpec {
                name: tables::INVENTORY_OWNER,
                row_len: 8,
                rows: vec![vec![0; 8]; 3],
                owners: Some(vec![HERO_PACK, BAG_INSIDE, TRADER_PACK]),
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

        point(tables::CREATION, 0, 56, at(&fixed, "test.v0.NameBytes", 0));
        point(tables::CREATION, 0, 64, 3);
        point(
            tables::CREATION,
            0,
            72,
            at(&fixed, "test.v0.Allocations", 0),
        );
        point(
            tables::CREATION,
            0,
            80,
            at(&fixed, "test.v0.Allocations", 0) + 28,
        );
        // The party's resources are the first two entries, the hero's the third.
        let entries = at(&fixed, "test.v0.ResourceEntries", 0);
        point(tables::RESOURCES, 0, 0, entries);
        point(tables::RESOURCES, 0, 8, entries + 128);
        point(tables::RESOURCES, 1, 0, entries + 128);
        point(tables::RESOURCES, 1, 8, entries + 192);
        for (stack, item) in [HERO_GOLD, BAGGED_GOLD, TRADER_GOLD]
            .into_iter()
            .enumerate()
        {
            point(tables::STACK_OF, stack, 0, at(&fixed, tables::STACK, stack));
            let items = at(&fixed, "test.v0.StackItems", stack);
            point("test.v0.StackItems", stack, 0, entity(&fixed, item));
            point(tables::STACK, stack, 0, items);
            point(tables::STACK, stack, 8, items + 8);
            let entry = at(&fixed, tables::STACK_ENTRY, stack);
            point(tables::STACK, stack, 16, entry);
            point(tables::STACK, stack, 24, entry + 8);
        }
        let packs = [HERO_PACK, HERO_PACK, BAG_INSIDE, TRADER_PACK, HERO_PACK];
        for (row, pack) in packs.into_iter().enumerate() {
            point(tables::MEMBER, row, 0, at(&fixed, tables::MEMBER_DATA, row));
            point(tables::MEMBER_DATA, row, 0, entity(&fixed, pack));
        }
        for (row, owner) in [HERO, BAG, TRADER].into_iter().enumerate() {
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
        assert_eq!(party.characters.len(), 2);
        assert_eq!(party.inspiration, Some(Inspiration { amount: 3, max: 4 }));
        assert_eq!(party.camp_supplies, Some(660));
        assert_eq!(party.gold_total(), 3756);

        let hero = &party.characters[0];
        assert_eq!(hero.entity, uuid(HERO));
        assert_eq!(hero.template, "e285d7b5-6b09-4857-89aa-a91e9e6898e5");
        assert_eq!(hero.name.as_deref(), Some("Tav"));
        assert_eq!(
            (hero.level, hero.entitled_level, hero.experience),
            (5, 5, 5595)
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
            (7100, 6, 5)
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
