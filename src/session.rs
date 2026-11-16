//! An editing session: a save that is open, the changes made to it so far, and the way out.
//!
//! Nothing here touches the save on disk until [`Session::export`], and that writes a new
//! save folder; the one that was opened is never modified. Every change is a difference
//! between the party as it was read and the party as it now stands, so the list of changes
//! is always exactly what an export would write, and reverting one puts back exactly what
//! was there.

use crate::catalog::{Catalog, CatalogItem, Kind};
use crate::items;
use crate::lsf::Lsf;
use crate::mods::{self, Module};
use crate::newage::NewAge;
use crate::party::{self, Character, Party};
use crate::save::{self, PartyMember, Save};
use crate::saves::{self, SaveSummary};
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Most steps that can be undone.
const UNDO_LIMIT: usize = 200;
/// Edits to one field less than this many seconds apart count as one step to undo.
const MERGE_WINDOW: f64 = 1.0;
/// The most gold this editor will put in one stack. The game's top bar shows up to seven
/// digits comfortably.
pub const MAX_GOLD: i32 = 9_999_999;
/// The most experience this editor will set.
pub const MAX_EXPERIENCE: i32 = 9_999_999;
/// The most this editor will put in a stack of items. A limit of the editor, not the game.
pub const MAX_AMOUNT: i32 = 9_999;
/// The most of one item this editor will add at once.
pub const MAX_ADDED: i32 = 20;

/// One value that can be edited. Characters and gold stacks are named by position in the
/// party as it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// A character's total experience.
    Experience(usize),
    /// How many level-ups the game should offer a character.
    LevelUps(usize),
    /// Points put into one of a character's abilities (0 Strength to 5 Charisma).
    Allocated(usize, usize),
    /// The amount in one of a character's gold stacks.
    Gold(usize, usize),
    /// The party's Inspiration points.
    Inspiration,
    /// One of a companion's approval ratings: the companion, then which rating.
    Approval(usize, usize),
    /// How many a stack of items holds: the inventory, then the item in it.
    Amount(usize, usize),
    /// How many of an item to add, by its place in [`Session::additions`].
    Add(usize),
}

/// How a new item is made: a copy of `donor`, an item anywhere in the save, made into the
/// item asked for unless it is the same one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub donor: crate::value::Uuid,
    /// Whether the donor is the same item, so a copy needs no making into it.
    pub same: bool,
    /// For a scroll made from another item: the spell it will cast. The copy leaves out the
    /// donor's spell, and the game fills this one in from the template.
    pub spell: Option<String>,
}

/// An item to add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addition {
    pub holder: party::Holder,
    pub item: CatalogItem,
    pub plan: Plan,
    /// Whether the new ones go in as one stack, for an item that stacks.
    pub stacked: bool,
    count: u32,
}

/// An item in the save new items can be copied from, and what it is.
#[derive(Clone, Debug)]
struct Source {
    donor: items::Donor,
    /// Whose inventory it is in, when that is the party's or the camp chest's.
    holder: Option<party::Holder>,
    kind: Kind,
    category: String,
}

/// One difference between the save as opened and as it now stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub field: Field,
    /// Whose it is: a character's name, or "Party".
    pub who: String,
    /// What it is, for example "Experience".
    pub what: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Copy, Debug)]
struct Step {
    field: Field,
    before: i32,
    after: i32,
}

/// A save that is open for editing.
pub struct Session {
    summary: SaveSummary,
    save: Save,
    globals: Lsf,
    /// The entity data as read. Edits are applied to a copy at export.
    entities: NewAge,
    original: Party,
    current: Party,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The field last edited and when, for merging quick repeated edits into one step.
    last_edit: Option<(Field, f64)>,
    /// For each character, their entry in the save's own summary.
    entries: Vec<Option<usize>>,
    /// The characters in the order to show them.
    order: Vec<usize>,
    /// The mods the save lists, as `meta.lsf` has them.
    mods: Vec<Module>,
    /// The mods the game has active, when the save is in the game's own folders.
    active: Option<mods::Active>,
    /// The game's items, for the names players see, once read from the game's files.
    catalog: Option<Arc<Catalog>>,
    /// What new items can be copied from, once the catalog is in.
    sources: Vec<Source>,
    /// Items to add, in the order they were asked for.
    additions: Vec<Addition>,
    /// The story state, read the first time it is asked for.
    story: std::cell::OnceCell<Result<crate::story::State, String>>,
}

/// Finds each character's entry in the save's own summary, which is the only place the
/// classes are written out and which lists the party in the game's order.
///
/// A companion is listed under their own name. Anyone else, a custom character above all, is
/// told apart by level and experience, which the summary and the entity data agree on. A save
/// this editor exported has the summary as it was before the edits (the game writes a new one
/// when it next saves), so after that a character is matched by level alone, which no edit
/// changes. The summary lists only the party, so a companion at camp has no entry.
fn summary_entries(members: &[PartyMember], characters: &[Character]) -> Vec<Option<usize>> {
    // The summary writes Lae'zel as "Laezel".
    let key = |name: &str| -> String {
        name.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let mut entries = vec![None; characters.len()];
    let mut taken = vec![false; members.len()];
    let mut claim = |entries: &mut Vec<Option<usize>>,
                     character: usize,
                     fits: &dyn Fn(&PartyMember) -> bool| {
        if let Some(member) = (0..members.len()).find(|&m| !taken[m] && fits(&members[m])) {
            taken[member] = true;
            entries[character] = Some(member);
        }
    };
    let party = || characters.iter().enumerate().filter(|(_, c)| c.in_party());
    for (index, character) in party() {
        if let Some(name) = character.companion() {
            claim(&mut entries, index, &|m| key(&m.origin) == key(name));
        }
    }
    for (index, character) in party() {
        if entries[index].is_none() {
            let experience = character.experience.map(u64::from);
            claim(&mut entries, index, &|m| {
                (m.level, Some(m.experience_total)) == (character.level, experience)
            });
        }
    }
    for (index, character) in party() {
        if entries[index].is_none() {
            claim(&mut entries, index, &|m| m.level == character.level);
        }
    }
    entries
}

/// A count from the save as the number the session works in.
fn signed(count: u32) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

fn field_of(party: &Party, field: Field) -> Option<i32> {
    let character = |i: usize| party.characters.get(i);
    match field {
        Field::Experience(c) => character(c)?.experience.map(signed),
        Field::LevelUps(c) => {
            character(c).map(|c| signed(c.entitled_level.saturating_sub(c.level)))
        }
        Field::Allocated(c, ability) => character(c)?.allocated?.get(ability).copied().map(signed),
        Field::Gold(c, stack) => character(c)?.gold.get(stack).map(|g| signed(g.amount)),
        Field::Inspiration => party.inspiration.map(|i| signed(i.amount)),
        Field::Approval(c, which) => character(c)?.approval.get(which).map(|a| a.rating),
        Field::Amount(inventory, item) => party
            .inventories
            .get(inventory)?
            .items
            .get(item)
            .filter(|item| item.stacked)
            .map(|item| signed(item.amount)),
        // Kept by the session, not the party.
        Field::Add(_) => None,
    }
}

fn set_field(party: &mut Party, field: Field, value: i32) {
    // Only approval can be below zero; the ranges keep the rest from it.
    let count = value.max(0) as u32;
    if let Field::Approval(c, which) = field {
        party.characters[c].approval[which].rating = value;
        return;
    }
    let value = count;
    match field {
        Field::Experience(c) => party.characters[c].experience = Some(value),
        Field::LevelUps(c) => {
            let character = &mut party.characters[c];
            character.entitled_level = character.level + value;
        }
        Field::Allocated(c, ability) => {
            let character = &mut party.characters[c];
            if let Some(allocated) = character.allocated.as_mut() {
                // The score moves with the points, as the export writes it.
                let score = &mut character.abilities[ability];
                *score = (*score + value).saturating_sub(allocated[ability]);
                allocated[ability] = value;
            }
        }
        Field::Gold(c, stack) => party.characters[c].gold[stack].amount = value,
        Field::Inspiration => {
            if let Some(inspiration) = party.inspiration.as_mut() {
                inspiration.amount = value;
            }
        }
        Field::Approval(..) => {}
        Field::Amount(inventory, item) => party.inventories[inventory].items[item].amount = value,
        Field::Add(_) => {}
    }
}

impl Session {
    /// Opens the save in `folder` for editing.
    pub fn open(folder: &Path) -> Result<Session> {
        let summary = saves::summarize(folder)?;
        if let save::Support::ReadOnly(why) = &summary.support {
            bail!("{why}");
        }
        let save = Save::open(&summary.lsv)?;
        let globals = save.lsf(save::GLOBALS)?;
        let entities = NewAge::from_lsf(&globals).context(save::GLOBALS)?;
        let original = party::read(&globals, &entities)?;
        ensure!(
            !original.characters.is_empty(),
            "the save has no party to edit"
        );
        let entries = summary_entries(&summary.party, &original.characters);
        // The party first, in the summary's order, which is the game's: the main character
        // first. Then the companions at camp, by name.
        let mut order: Vec<usize> = (0..original.characters.len()).collect();
        order.sort_by_key(|&c| {
            let character = &original.characters[c];
            (
                !character.in_party(),
                entries[c].unwrap_or(usize::MAX),
                character.display_name(),
            )
        });
        let mods = mods::read(&save.lsf(save::META)?);
        let active = mods::active_for(&summary.folder);
        Ok(Session {
            summary,
            save,
            globals,
            entities,
            current: original.clone(),
            original,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            entries,
            order,
            mods,
            active,
            catalog: None,
            sources: Vec::new(),
            additions: Vec::new(),
            story: std::cell::OnceCell::new(),
        })
    }

    pub fn summary(&self) -> &SaveSummary {
        &self.summary
    }

    pub fn save(&self) -> &Save {
        &self.save
    }

    /// The characters in the order to show them: the game's own, the main character first.
    /// Each is an index into the party's characters.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// What the save's own summary says of a character, if it lists them.
    pub fn summary_of(&self, character: usize) -> Option<&PartyMember> {
        let entry = (*self.entries.get(character)?)?;
        self.summary.party.get(entry)
    }

    /// The party with the changes made so far.
    pub fn party(&self) -> &Party {
        &self.current
    }

    /// The party as the save has it.
    pub fn original(&self) -> &Party {
        &self.original
    }

    /// Gives the session the game's items, for naming them as the game does.
    pub fn set_catalog(&mut self, catalog: Arc<Catalog>) {
        let holders: std::collections::HashMap<crate::value::Uuid, party::Holder> = self
            .original
            .inventories
            .iter()
            .flat_map(|i| i.items.iter().map(|item| (item.entity, i.holder)))
            .collect();
        self.sources = items::donors(&self.globals, &self.entities)
            .into_iter()
            .map(|donor| {
                let known = catalog.by_stats(&donor.stats);
                Source {
                    holder: holders.get(&donor.entity).copied(),
                    kind: known.map_or(Kind::Other, |k| k.kind),
                    category: known.map(|k| k.category.clone()).unwrap_or_default(),
                    donor,
                }
            })
            .collect();
        self.catalog = Some(catalog);
    }

    /// The game's items, once read from the game's files.
    pub fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_deref()
    }

    /// How a new `item` for `holder` would be made, or why it can't be. It is a copy of the
    /// closest item the save has: the same item, the holder's own before anyone else's,
    /// then for a scroll another scroll, and else an item of the same kind before any other.
    pub fn pattern_for(&self, holder: party::Holder, item: &CatalogItem) -> Result<Plan, String> {
        // Lower is better: what it is, then whose, then whether anything of it is left out.
        let whose = |source: &Source| match source.holder {
            Some(h) if h == holder => 0,
            Some(_) => 1,
            None => 2,
        };
        if !item.pickable {
            return Err(
                "This can't be picked up in the game, so it can't go in an inventory.".to_owned(),
            );
        }
        if item.container {
            return Err(
                "The game gives this item an inventory of its own, as it does a bag or a \
                 chest, and a copy would have none, so it can't be added."
                    .to_owned(),
            );
        }
        let is_scroll = |category: &str| category.contains("MagicScroll");
        let scroll = !item.spell.is_empty() || is_scroll(&item.category);
        let rank = |source: &Source| -> Option<(u8, u8, u8)> {
            let donor = &source.donor;
            let what = if donor.stats == item.stats {
                0
            } else if !donor.root {
                return None;
            } else if scroll {
                // A scroll's spell is in records of its own, which only a scroll has; the game
                // fills them in for a copy that leaves them out.
                match (is_scroll(&source.category), &donor.spell) {
                    (true, Some(_)) => 1,
                    (false, Some(_)) => 2,
                    (_, None) => 3,
                }
            } else if donor.spell.is_some() {
                return None;
            } else if source.kind == item.kind && source.category == item.category {
                1
            } else if source.kind == item.kind {
                2
            } else {
                3
            };
            Some((what, whose(source), !donor.plain as u8))
        };
        let best = self
            .sources
            .iter()
            .filter_map(|source| Some((rank(source)?, source)))
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, source)| source);
        let Some(source) = best else {
            return Err("There is nothing in this save to make one from.".to_owned());
        };
        let same = source.donor.stats == item.stats;
        let spell = (!same && scroll).then(|| item.spell.clone());
        Ok(Plan {
            donor: source.donor.entity,
            same,
            spell,
        })
    }

    /// Items to add, in the order they were asked for. How many of each is [`Field::Add`].
    pub fn additions(&self) -> &[Addition] {
        &self.additions
    }

    /// Adds one more of the item called `stats` for `holder`. Returns its place in
    /// [`Session::additions`], or why it can't be added.
    pub fn add_item(
        &mut self,
        holder: party::Holder,
        stats: &str,
        time: f64,
    ) -> Result<usize, String> {
        let item = self
            .catalog
            .as_ref()
            .and_then(|c| c.by_stats(stats))
            .cloned()
            .ok_or_else(|| "The game's files have no such item.".to_owned())?;
        let index = match self
            .additions
            .iter()
            .position(|a| a.holder == holder && a.item.stats == item.stats)
        {
            Some(index) => index,
            None => {
                let plan = self.pattern_for(holder, &item)?;
                self.additions.push(Addition {
                    holder,
                    stacked: item.max_stack > 1,
                    item,
                    plan,
                    count: 0,
                });
                self.additions.len() - 1
            }
        };
        let field = Field::Add(index);
        let now = self.value(field).unwrap_or(0);
        self.last_edit = None;
        self.set(field, now + 1, time);
        self.last_edit = None;
        Ok(index)
    }

    /// How an addition is made, to say: "A copy of the one Tav holds", "Made from a
    /// Longsword someone outside the party has".
    pub fn addition_hint(&self, index: usize) -> String {
        let Some(addition) = self.additions.get(index) else {
            return String::new();
        };
        let source = self
            .sources
            .iter()
            .find(|s| s.donor.entity == addition.plan.donor);
        let holder = source.and_then(|s| s.holder);
        let place = match holder {
            Some(party::Holder::CampChest) => "in the camp chest".to_owned(),
            Some(h) => format!("{} holds", self.holder_name(h)),
            None => "someone outside the party has".to_owned(),
        };
        let mut hint = match addition.plan.same {
            true if holder == Some(addition.holder) => match addition.holder {
                party::Holder::CampChest => "A copy of one in the camp chest".to_owned(),
                h => format!("A copy of the one {} holds", self.holder_name(h)),
            },
            true => format!("A copy of one {place}"),
            false => {
                let name = source
                    .map(|s| {
                        self.catalog
                            .as_ref()
                            .and_then(|c| c.by_stats(&s.donor.stats))
                            .map(|c| self.catalog_name(c))
                            .unwrap_or_else(|| tidy(&s.donor.stats))
                    })
                    .unwrap_or_default();
                let article = match name.chars().next().map(|c| c.to_ascii_lowercase()) {
                    Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
                    _ => "a",
                };
                format!("Made from {article} {name} {place}")
            }
        };
        if addition.plan.spell.is_some() {
            hint.push_str(", the game giving it this scroll's spell");
        }
        if addition.stacked {
            hint.push_str(", as one stack");
        }
        hint
    }

    /// How many of `stats` are to be added for `holder`.
    pub fn added(&self, holder: party::Holder, stats: &str) -> u32 {
        self.additions
            .iter()
            .find(|a| a.holder == holder && a.item.stats == stats)
            .map_or(0, |a| a.count)
    }

    /// Where new items for `holder` go: the main inventory of a character, owned by them,
    /// or of the camp chest.
    fn destination(&self, holder: party::Holder) -> Result<items::Destination> {
        match holder {
            party::Holder::Character(c) => {
                let character = self
                    .original
                    .characters
                    .get(c)
                    .ok_or_else(|| anyhow!("no character {c}"))?;
                items::Destination::of(&self.entities, character.entity, true)
            }
            party::Holder::CampChest => {
                let chest = party::camp_chest(&self.entities)
                    .ok_or_else(|| anyhow!("the save has no camp chest"))?;
                items::Destination::of(&self.entities, chest, false)
            }
        }
    }

    /// Whether items are named as the game names them.
    pub fn has_item_names(&self) -> bool {
        self.catalog.is_some()
    }

    /// The name to show for an item: the one players see when the game's files have been
    /// read, else its game name made readable.
    pub fn item_name(&self, item: &party::Item) -> String {
        self.catalog
            .as_ref()
            .and_then(|c| c.display_name(&item.template, &item.stats))
            .map(str::to_owned)
            .unwrap_or_else(|| tidy(&item.stats))
    }

    /// The mods the save lists, in its order.
    pub fn mods(&self) -> &[Module] {
        &self.mods
    }

    /// The mods the game has active, if the save is in the game's own folders and its
    /// profile has a mod list.
    pub fn active_mods(&self) -> Option<&mods::Active> {
        self.active.as_ref()
    }

    /// The field's value now, or `None` if this save has no such field (a character with no
    /// character creation record has no allocations; a new game has no Inspiration).
    pub fn value(&self, field: Field) -> Option<i32> {
        match field {
            Field::Add(i) => self.additions.get(i).map(|a| a.count as i32),
            _ => field_of(&self.current, field),
        }
    }

    /// The field's value in the save as opened.
    pub fn original_value(&self, field: Field) -> Option<i32> {
        match field {
            Field::Add(i) => (i < self.additions.len()).then_some(0),
            _ => field_of(&self.original, field),
        }
    }

    /// Changes a field without recording a step.
    fn apply(&mut self, field: Field, value: i32) {
        match field {
            Field::Add(i) => self.additions[i].count = value.max(0) as u32,
            _ => set_field(&mut self.current, field, value),
        }
        // The number a save keeps beside Inspiration goes up with an amount that passes it,
        // as the export writes it, and back down when the amount does.
        if let (Field::Inspiration, Some(now), Some(before)) = (
            field,
            self.current.inspiration.as_mut(),
            self.original.inspiration,
        ) {
            now.max = before.max.max(now.amount);
        }
    }

    pub fn is_changed(&self, field: Field) -> bool {
        self.value(field) != self.original_value(field)
    }

    /// The story's counters, tallies and flags as the save has them, or why they can't be
    /// read. Read once, when first asked for; no edit changes them.
    pub fn story(&self) -> Result<&crate::story::State, &str> {
        self.story
            .get_or_init(|| {
                crate::story::State::read(&self.save, &self.globals).map_err(|e| format!("{e:#}"))
            })
            .as_ref()
            .map_err(String::as_str)
    }

    /// A warning for Inspiration raised past what the game allows by itself.
    pub fn inspiration_warning(&self) -> Option<String> {
        let now = self.party().inspiration?.amount;
        let before = self.original.inspiration?.amount;
        (now > party::INSPIRATION_CAP && now > before).then(|| {
            format!(
                "The game allows {} Inspiration points at most. To keep {now}, make sure a mod \
                 that lifts that limit is active when you load the save.",
                party::INSPIRATION_CAP
            )
        })
    }

    /// The values the field can take.
    pub fn range(&self, field: Field) -> RangeInclusive<i32> {
        match field {
            Field::Experience(_) => 0..=MAX_EXPERIENCE,
            Field::LevelUps(c) => {
                let level = self.original.characters.get(c).map_or(0, |c| c.level);
                0..=signed(party::MAX_LEVEL.saturating_sub(level))
            }
            Field::Allocated(..) => 0..=signed(party::MAX_ALLOCATED),
            Field::Gold(..) => 1..=MAX_GOLD,
            // The number a save keeps beside the amount is no limit: it equals the amount.
            Field::Inspiration => {
                let now = self.original.inspiration.map_or(0, |i| i.amount);
                0..=signed(party::MAX_INSPIRATION.max(now))
            }
            Field::Approval(..) => {
                // The game's own range, widened to take in a rating a mod has put outside.
                let now = self.original_value(field).unwrap_or(0);
                party::APPROVAL_MIN.min(now)..=party::APPROVAL_MAX.max(now)
            }
            // A stack the save already has above the editor's limit keeps its room.
            Field::Amount(..) => 1..=MAX_AMOUNT.max(self.original_value(field).unwrap_or(1)),
            // As one stack, up to what one holds; else one item each.
            Field::Add(i) => match self.additions.get(i) {
                Some(a) if a.stacked => 0..=(a.item.max_stack.min(MAX_AMOUNT as u32) as i32),
                _ => 0..=MAX_ADDED,
            },
        }
    }

    /// Sets a field, clamped to its range. `time` is the current time in seconds, used to
    /// fold quick repeated edits to one field into a single step to undo.
    pub fn set(&mut self, field: Field, value: i32, time: f64) {
        let Some(before) = self.value(field) else {
            return;
        };
        let range = self.range(field);
        let value = value.clamp(*range.start(), *range.end());
        if value == before {
            return;
        }
        self.apply(field, value);
        self.redo.clear();
        let merges = self
            .last_edit
            .is_some_and(|(last, at)| last == field && time - at < MERGE_WINDOW);
        match self.undo.last_mut() {
            Some(step) if merges && step.field == field => step.after = value,
            _ => self.undo.push(Step {
                field,
                before,
                after: value,
            }),
        }
        // A merged step that ends where it began is no step at all.
        if self.undo.last().is_some_and(|s| s.before == s.after) {
            self.undo.pop();
        }
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.last_edit = Some((field, time));
    }

    /// Puts one field back to what the save has.
    pub fn revert(&mut self, field: Field, time: f64) {
        if let Some(original) = self.original_value(field) {
            // Not merged with whatever came before: a revert is its own step.
            self.last_edit = None;
            self.set(field, original, time);
            self.last_edit = None;
        }
    }

    /// Puts every field back to what the save has, as one step per field.
    pub fn revert_all(&mut self, time: f64) {
        for change in self.changes() {
            self.revert(change.field, time);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last step and returns the field it was to.
    pub fn undo(&mut self) -> Option<Field> {
        let step = self.undo.pop()?;
        self.apply(step.field, step.before);
        self.redo.push(step);
        self.last_edit = None;
        Some(step.field)
    }

    /// Redoes the last step undone and returns the field it was to.
    pub fn redo(&mut self) -> Option<Field> {
        let step = self.redo.pop()?;
        self.apply(step.field, step.after);
        self.undo.push(step);
        self.last_edit = None;
        Some(step.field)
    }

    /// Every field that can exist in this save, in the order the Changes panel lists them.
    fn fields(&self) -> Vec<Field> {
        let mut fields = Vec::new();
        for (c, character) in self.original.characters.iter().enumerate() {
            fields.push(Field::Experience(c));
            fields.push(Field::LevelUps(c));
            if character.allocated.is_some() {
                fields.extend((0..party::ABILITIES.len()).map(|a| Field::Allocated(c, a)));
            }
            fields.extend((0..character.gold.len()).map(|s| Field::Gold(c, s)));
            fields.extend((0..character.approval.len()).map(|a| Field::Approval(c, a)));
        }
        if self.original.inspiration.is_some() {
            fields.push(Field::Inspiration);
        }
        for (i, inventory) in self.original.inventories.iter().enumerate() {
            let stacked = inventory
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.stacked);
            fields.extend(stacked.map(|(item, _)| Field::Amount(i, item)));
        }
        fields.extend((0..self.additions.len()).map(Field::Add));
        fields
    }

    /// What `field` is called and whose it is.
    pub fn describe(&self, field: Field) -> (String, String) {
        let name = |c: usize| {
            self.original
                .characters
                .get(c)
                .map(Character::display_name)
                .unwrap_or_default()
        };
        match field {
            Field::Experience(c) => (name(c), "Experience".to_owned()),
            Field::LevelUps(c) => (name(c), "Level-ups to offer".to_owned()),
            Field::Allocated(c, ability) => (
                name(c),
                format!(
                    "{} points",
                    party::ABILITIES.get(ability).unwrap_or(&"Ability")
                ),
            ),
            Field::Gold(c, stack) => {
                let character = self.original.characters.get(c);
                let several = character
                    .is_some_and(|c| c.gold.iter().filter(|g| !g.in_container).count() > 1);
                let bagged = character
                    .and_then(|c| c.gold.get(stack))
                    .is_some_and(|g| g.in_container);
                let what = match (several, bagged) {
                    (_, true) => "Gold in a container".to_owned(),
                    (true, false) => format!("Gold (stack {})", stack + 1),
                    (false, false) => "Gold".to_owned(),
                };
                (name(c), what)
            }
            Field::Inspiration => ("Party".to_owned(), "Inspiration".to_owned()),
            Field::Approval(c, which) => {
                let ratings = self.original.characters.get(c).map(|c| &c.approval);
                let what = match ratings {
                    // With more than one player, say whose character it is of.
                    Some(ratings) if ratings.len() > 1 => {
                        format!("Approval of {}", self.name_of(ratings[which].of))
                    }
                    _ => "Approval".to_owned(),
                };
                (name(c), what)
            }
            Field::Amount(inventory, item) => {
                let inventory = self.original.inventories.get(inventory);
                let who = inventory.map_or_else(String::new, |i| self.holder_name(i.holder));
                let what = inventory
                    .and_then(|i| i.items.get(item))
                    .map_or_else(String::new, |item| self.item_name(item));
                (who, what)
            }
            Field::Add(i) => match self.additions.get(i) {
                Some(addition) => (
                    self.holder_name(addition.holder),
                    format!("Add {}", self.catalog_name(&addition.item)),
                ),
                None => (String::new(), String::new()),
            },
        }
    }

    /// A catalog item's name: the one players see, else its game name made readable.
    pub fn catalog_name(&self, item: &CatalogItem) -> String {
        match item.display.is_empty() {
            true => tidy(&item.stats),
            false => item.display.clone(),
        }
    }

    /// Who holds an inventory, by name.
    pub fn holder_name(&self, holder: party::Holder) -> String {
        match holder {
            party::Holder::Character(c) => self
                .original
                .characters
                .get(c)
                .map(Character::display_name)
                .unwrap_or_default(),
            party::Holder::CampChest => "Camp chest".to_owned(),
        }
    }

    /// Who holds an inventory, as put in the middle of a sentence: "Tav", "the camp chest".
    pub fn holder_in_sentence(&self, holder: party::Holder) -> String {
        match holder {
            party::Holder::CampChest => "the camp chest".to_owned(),
            _ => self.holder_name(holder),
        }
    }

    /// The name of the character with the entity `id`, for saying whom a rating is of.
    pub fn name_of(&self, id: crate::value::Uuid) -> String {
        self.original
            .characters
            .iter()
            .find(|c| c.entity == id)
            .map_or_else(|| "another character".to_owned(), Character::display_name)
    }

    /// The differences between the save as opened and as it now stands.
    pub fn changes(&self) -> Vec<Change> {
        self.fields()
            .into_iter()
            .filter(|&field| self.is_changed(field))
            .filter_map(|field| {
                let (who, what) = self.describe(field);
                Some(Change {
                    field,
                    who,
                    what,
                    before: self.original_value(field)?.to_string(),
                    after: self.value(field)?.to_string(),
                })
            })
            .collect()
    }

    pub fn is_dirty(&self) -> bool {
        self.current != self.original || self.additions.iter().any(|a| a.count > 0)
    }

    /// The folder exports go to unless told otherwise: the one the opened save is in.
    pub fn default_export_folder(&self) -> PathBuf {
        self.summary
            .folder
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    /// A name for the exported save that no save in `parent` has yet.
    pub fn suggested_name(&self, parent: &Path) -> String {
        let taken = |name: &str| {
            std::fs::read_dir(parent)
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| saves::display_name(&e.path()).eq_ignore_ascii_case(name))
        };
        let base = format!("{} - Edited", self.summary.name);
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|name| !taken(name))
            .unwrap_or(base)
    }

    /// The entity data with every change applied.
    fn edited_entities(&self) -> Result<NewAge> {
        let mut entities = self.entities.clone();
        for change in self.changes() {
            let value = self
                .value(change.field)
                .ok_or_else(|| anyhow!("a changed field has no value"))?;
            let character = |c: usize| &self.original.characters[c];
            let count = u32::try_from(value);
            match change.field {
                Field::Experience(c) => {
                    party::set_experience(&mut entities, character(c).entity, count?)?
                }
                Field::LevelUps(c) => party::set_entitled_level(
                    &mut entities,
                    character(c).entity,
                    character(c).level + count?,
                )?,
                Field::Allocated(c, ability) => {
                    party::set_allocated(&mut entities, character(c).entity, ability, count?)?
                }
                Field::Gold(c, stack) => {
                    party::set_gold(&mut entities, character(c).gold[stack].item, count?)?
                }
                Field::Inspiration => party::set_inspiration(&mut entities, count?)?,
                Field::Amount(inventory, item) => party::set_amount(
                    &mut entities,
                    self.original.inventories[inventory].items[item].entity,
                    count?,
                )?,
                // Added at export, with the node tree.
                Field::Add(_) => {}
                Field::Approval(c, which) => party::set_approval(
                    &mut entities,
                    character(c).entity,
                    character(c).approval[which].of,
                    value,
                )?,
            }
        }
        Ok(entities)
    }

    /// Writes the save with its changes as a new save called `name` in `parent`, and returns
    /// the new folder. The save that was opened is not touched. The result is read back and
    /// checked before this returns; if it does not hold what was asked for, it is removed.
    pub fn export(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let mut entities = self.edited_entities()?;
        let mut globals = self.globals.clone();
        // New items: each a copy of its donor in the holder's own inventory, made into the
        // item asked for unless it is the same one; one stack of them for an item that
        // stacks.
        let mut added: Vec<(party::Holder, crate::value::Uuid, String, u32)> = Vec::new();
        for addition in self.additions.iter().filter(|a| a.count > 0) {
            let to = self.destination(addition.holder)?;
            let (copies, amount) = match addition.stacked {
                true => (1, Some(addition.count)),
                false => (addition.count, None),
            };
            let item = &addition.item;
            for _ in 0..copies {
                let plan = &addition.plan;
                // A scroll given another spell leaves the donor's out, for the game to fill in.
                let without: &[&str] = match plan.spell {
                    Some(_) => &items::SPELL_ROWS,
                    None => &[],
                };
                let copy = items::copy_into_without(
                    &mut globals,
                    &mut entities,
                    plan.donor,
                    &to,
                    amount,
                    without,
                )?;
                if !plan.same {
                    items::make_into(
                        &mut globals,
                        &mut entities,
                        copy,
                        &item.template,
                        &item.stats,
                    )?;
                }
                added.push((
                    addition.holder,
                    copy,
                    item.stats.clone(),
                    amount.unwrap_or(1),
                ));
            }
        }
        entities.store(&mut globals)?;
        let mut package = self.save.package.clone();
        package.replace(save::GLOBALS, &globals.write()?)?;
        // An export is the player's own save, shown by its name.
        let made_manual = save::as_manual_save(&mut package)?;
        let folder = save::export_copy(&self.save, &package, parent, name)?;

        let check = || -> Result<()> {
            let written = Save::open(&folder)?;
            let globals = written.lsf(save::GLOBALS)?;
            let mut party = party::read(&globals, &NewAge::from_lsf(&globals)?)?;
            // Every new item is where it was asked for, as the item asked for.
            for (holder, entity, stats, amount) in &added {
                let found = party
                    .inventories
                    .iter()
                    .filter(|i| i.holder == *holder)
                    .flat_map(|i| &i.items)
                    .find(|i| i.entity == *entity);
                ensure!(
                    found.is_some_and(|i| &i.stats == stats && i.amount == *amount),
                    "an added item is not where it should be"
                );
            }
            // And apart from them, the party is as it was edited.
            for inventory in &mut party.inventories {
                inventory
                    .items
                    .retain(|i| !added.iter().any(|(_, entity, ..)| *entity == i.entity));
            }
            party.inventories.retain(|i| !i.items.is_empty());
            ensure!(
                same_party(&party, &self.current),
                "the exported save does not hold the changes"
            );
            if made_manual {
                ensure!(
                    save::save_type(&written.package)? == Some(0),
                    "the exported save is not a manual save"
                );
            }
            for entry in self.save.package.entries() {
                let changed =
                    entry.name() == save::GLOBALS || (made_manual && entry.name() == save::META);
                if !changed {
                    ensure!(
                        self.save.package.unpack(entry)?
                            == written.package.read_file(entry.name())?,
                        "{} changed in the exported save",
                        entry.name()
                    );
                }
            }
            Ok(())
        };
        if let Err(error) = check() {
            // Only ever the folder this call just created.
            let _ = std::fs::remove_dir_all(&folder);
            return Err(error.context("the export was checked and removed"));
        }
        Ok(folder)
    }
}

/// The game's own name for an item, made readable: `CONS_FOOD_Fruit_Apple` as "Fruit
/// Apple", `OBJ_Kit_ThievesTools` as "Kit Thieves Tools". The names the game shows are in its
/// language files, not in the save.
pub fn tidy(stats: &str) -> String {
    let parts: Vec<&str> = stats.split('_').filter(|p| !p.is_empty()).collect();
    // Leading words in capitals are categories: OBJ, CONS, FOOD, a mod's initials.
    let is_category = |p: &str| {
        p.len() <= 5
            && p.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };
    let skip = parts
        .iter()
        .take(parts.len().saturating_sub(1))
        .take_while(|p| is_category(p))
        .count();
    let mut words: Vec<String> = Vec::new();
    for part in &parts[skip..] {
        let shouting = part.len() > 1 && !part.chars().any(|c| c.is_lowercase());
        if shouting {
            // APPLE as Apple.
            let mut chars = part.chars();
            let first = chars.next().unwrap_or_default();
            words.push(format!("{first}{}", chars.as_str().to_lowercase()));
            continue;
        }
        // ThievesTools as Thieves Tools.
        let mut word = String::new();
        let mut previous_lower = false;
        for c in part.chars() {
            if c.is_uppercase() && previous_lower {
                words.push(std::mem::take(&mut word));
            }
            previous_lower = c.is_lowercase();
            word.push(c);
        }
        words.push(word);
    }
    match words.is_empty() {
        true => "Unnamed item".to_owned(),
        false => words.join(" "),
    }
}

/// Whether two readings of a party agree. Gold stacks are compared by item, since reading
/// lists them by amount and an edit can reorder them.
fn same_party(a: &Party, b: &Party) -> bool {
    let normalized = |party: &Party| {
        let mut party = party.clone();
        for character in &mut party.characters {
            character.gold.sort_by_key(|g| g.item.0);
        }
        party
    };
    normalized(a) == normalized(b)
}

/// Builds a whole save on disk that can be opened for editing. Used by tests.
pub mod build {
    use super::*;
    use crate::saves::build::{save_folder, SaveSpec};

    /// Writes a save holding the sample party from [`party::build`] under `dir`.
    pub fn sample_save(dir: &Path, name: &str) -> Result<PathBuf> {
        let mut spec = SaveSpec::new("Tav", name);
        // The game's own quicksaves and autosaves are named for what they are.
        spec.save_type = match name {
            n if n.starts_with("QuickSave_") => 1,
            n if n.starts_with("AutoSave_") => 2,
            _ => 0,
        };
        spec.mods = vec!["Faerun Colors".to_owned(), "Better UI".to_owned()];
        spec.files = vec![
            (save::GLOBALS.to_owned(), party::build::globals_lsf()),
            (save::STORY.to_owned(), crate::story::build::sample()),
        ];
        spec.screenshot = Some(b"not really a picture".to_vec());
        save_folder(dir, &spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nautiloid-session-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(dir: &Path) -> Session {
        Session::open(&build::sample_save(dir, "QuickSave_3").unwrap()).unwrap()
    }

    fn character(template: &str, name: Option<&str>, level: u32, experience: u32) -> Character {
        Character {
            entity: crate::value::Uuid([0; 16]),
            template: template.to_owned(),
            name: name.map(str::to_owned),
            level,
            entitled_level: level,
            experience: Some(experience),
            approval: Vec::new(),
            abilities: [10; 6],
            allocated: None,
            gold: Vec::new(),
        }
    }

    fn member(origin: &str, class: &str, level: u32, experience: u64) -> PartyMember {
        PartyMember {
            origin: origin.to_owned(),
            race: String::new(),
            level,
            classes: vec![save::ClassEntry {
                main: class.to_owned(),
                sub: String::new(),
            }],
            experience_total: experience,
            experience_in_level: 0,
        }
    }

    const KARLACH: &str = "2c76687d-93a2-477b-8b18-8a14b549304c";
    const SHADOWHEART: &str = "3ed74f06-3c60-42dc-83f6-f034cb47c679";
    const ASTARION: &str = "c7c13742-bacd-460a-8f65-f864fe41f255";
    const LAEZEL: &str = "58a69333-40bf-8358-1d17-fff240d7fb12";
    const CUSTOM: &str = "e285d7b5-6b09-4857-89aa-a91e9e6898e5";

    #[test]
    fn tidies_the_games_item_names() {
        assert_eq!(tidy("CONS_FOOD_Fruit_Apple"), "Fruit Apple");
        assert_eq!(tidy("OBJ_Kit_ThievesTools"), "Kit Thieves Tools");
        assert_eq!(tidy("FOOD_FRUIT_APPLE"), "Apple");
        assert_eq!(tidy("WPN_QMD_Mace"), "Mace");
        assert_eq!(tidy("Food_Haggis"), "Food Haggis");
        assert_eq!(tidy("OBJ_Potion_Healing"), "Potion Healing");
        assert_eq!(tidy("ARM_Camp_Sandals_A1_Black"), "Camp Sandals A1 Black");
        assert_eq!(tidy("OBJ"), "Obj");
        assert_eq!(tidy(""), "Unnamed item");
    }

    #[test]
    fn companions_are_matched_to_the_summary_by_name() {
        // As in a real save: the main character and Shadowheart have the same level and
        // experience, and the entity data lists the party in another order.
        let characters = [
            character(KARLACH, Some("Karlach"), 5, 5655),
            character(SHADOWHEART, Some("Shadowheart"), 5, 5595),
            character(ASTARION, Some("Astarion"), 5, 5645),
            character(CUSTOM, Some("Nelthryiis"), 5, 5595),
        ];
        let members = [
            member("Generic", "Sorcerer", 5, 5595),
            member("Shadowheart", "Cleric", 5, 5595),
            member("Karlach", "Barbarian", 5, 5655),
            member("Astarion", "Rogue", 5, 5645),
        ];
        assert_eq!(
            summary_entries(&members, &characters),
            [Some(2), Some(1), Some(3), Some(0)]
        );
    }

    #[test]
    fn others_are_matched_by_level_and_experience() {
        let characters = [
            // Played as an origin, so with no stored name; the summary spells it "Laezel".
            character(LAEZEL, None, 3, 900),
            character(CUSTOM, Some("Second player"), 3, 950),
            character(CUSTOM, Some("First player"), 3, 900),
            // Someone the summary doesn't list.
            character("0f0e0d0c-0000-0000-0000-000000000000", None, 1, 0),
        ];
        let members = [
            member("Generic", "Bard", 3, 900),
            member("Laezel", "Fighter", 3, 900),
            member("Generic", "Monk", 3, 950),
            // A summon, which is in the summary but isn't a character here.
            member("Djinni_PlanarAlly", "", 10, 5),
        ];
        assert_eq!(
            summary_entries(&members, &characters),
            [Some(1), Some(2), Some(0), None]
        );
    }

    #[test]
    fn opens_and_reports_fields() {
        let dir = temp_dir("open");
        let session = open(&dir);
        assert_eq!(session.summary().name, "QuickSave_3");
        assert_eq!(session.party().characters.len(), 3);
        assert_eq!(session.value(Field::Experience(0)), Some(5595));
        assert_eq!(session.value(Field::LevelUps(0)), Some(0));
        assert_eq!(session.value(Field::Allocated(0, 5)), Some(7));
        assert_eq!(session.value(Field::Gold(0, 0)), Some(3385));
        assert_eq!(session.value(Field::Gold(0, 1)), Some(371));
        assert_eq!(session.value(Field::Inspiration), Some(3));
        // The companion has no character creation record and carries no gold.
        assert_eq!(session.value(Field::Allocated(1, 0)), None);
        assert_eq!(session.value(Field::Gold(1, 0)), None);
        assert_eq!(session.value(Field::Experience(9)), None);
        assert_eq!(session.range(Field::LevelUps(0)), 0..=15);
        assert_eq!(session.range(Field::Inspiration), 0..=20);
        assert!(!session.is_dirty() && session.changes().is_empty());
        assert!(!session.can_undo() && !session.can_redo());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn changes_are_the_difference_from_the_save() {
        let dir = temp_dir("changes");
        let mut session = open(&dir);
        session.set(Field::Experience(0), 7100, 0.0);
        session.set(Field::LevelUps(0), 1, 10.0);
        session.set(Field::Allocated(0, 0), 7, 20.0);
        session.set(Field::Gold(0, 1), 5000, 30.0);
        session.set(Field::Inspiration, 99, 40.0);
        assert_eq!(
            session.value(Field::Inspiration),
            Some(20),
            "clamped to the editor's limit"
        );
        // Past the game's own limit of 4, which only a mod lifts.
        let warning = session.inspiration_warning().unwrap();
        assert!(
            warning.contains("4 Inspiration points at most"),
            "{warning}"
        );
        assert!(warning.contains("To keep 20"), "{warning}");
        session.set(Field::Inspiration, 4, 40.1);
        assert_eq!(session.inspiration_warning(), None);
        assert_eq!(session.party().characters[0].entitled_level, 6);

        let changes = session.changes();
        let lines: Vec<String> = changes
            .iter()
            .map(|c| format!("{}: {} {} -> {}", c.who, c.what, c.before, c.after))
            .collect();
        assert_eq!(
            lines,
            [
                "Tav: Experience 5595 -> 7100",
                "Tav: Level-ups to offer 0 -> 1",
                "Tav: Strength points 0 -> 7",
                "Tav: Gold in a container 371 -> 5000",
                "Party: Inspiration 3 -> 4",
            ]
        );
        assert!(session.is_dirty());

        // Setting a field back by hand removes its change; so does reverting.
        session.set(Field::Experience(0), 5595, 50.0);
        session.revert(Field::Gold(0, 1), 60.0);
        assert_eq!(session.changes().len(), 3);
        assert!(!session.is_changed(Field::Gold(0, 1)));
        session.revert_all(70.0);
        assert!(!session.is_dirty());
        // A field this save does not have is left alone.
        session.set(Field::Allocated(1, 0), 5, 80.0);
        assert!(!session.is_dirty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_and_redo_step_through_edits() {
        let dir = temp_dir("undo");
        let mut session = open(&dir);
        // Three quick edits to one field are one step; a later one is another.
        session.set(Field::Experience(0), 5600, 0.0);
        session.set(Field::Experience(0), 5700, 0.3);
        session.set(Field::Experience(0), 5800, 0.6);
        session.set(Field::Experience(0), 6000, 5.0);
        session.set(Field::Inspiration, 1, 5.1);

        assert_eq!(session.undo(), Some(Field::Inspiration));
        assert_eq!(session.value(Field::Inspiration), Some(3));
        assert_eq!(session.undo(), Some(Field::Experience(0)));
        assert_eq!(session.value(Field::Experience(0)), Some(5800));
        assert_eq!(session.undo(), Some(Field::Experience(0)));
        assert_eq!(session.value(Field::Experience(0)), Some(5595));
        assert_eq!(session.undo(), None);
        assert!(!session.is_dirty());

        assert_eq!(session.redo(), Some(Field::Experience(0)));
        assert_eq!(session.value(Field::Experience(0)), Some(5800));
        // A new edit after an undo drops what could have been redone.
        session.set(Field::Gold(0, 0), 10, 20.0);
        assert!(!session.can_redo());
        assert_eq!(session.undo(), Some(Field::Gold(0, 0)));
        assert_eq!(session.value(Field::Gold(0, 0)), Some(3385));

        // Quick edits that end where they began leave nothing to undo.
        let mut session = open(&temp_dir("undo-2"));
        session.set(Field::Inspiration, 2, 0.0);
        session.set(Field::Inspiration, 3, 0.2);
        assert!(!session.can_undo());
        // A revert is its own step even straight after an edit.
        session.set(Field::Inspiration, 1, 1.0);
        session.set(Field::Inspiration, 0, 1.1);
        session.revert(Field::Inspiration, 1.2);
        assert_eq!(session.undo(), Some(Field::Inspiration));
        assert_eq!(session.value(Field::Inspiration), Some(0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_companion_at_camp_is_listed_last_and_exports_its_changes() {
        let dir = temp_dir("camp");
        let mut session = open(&dir);
        // The hero and the companion in the party, then Gale at camp.
        assert_eq!(session.order(), [0, 1, 2]);
        assert!(!session.party().characters[2].in_party());
        assert_eq!(
            session.summary_of(2),
            None,
            "the summary lists only the party"
        );
        assert_eq!(session.value(Field::Experience(2)), None);
        assert_eq!(session.value(Field::Gold(2, 0)), Some(120));

        session.set(Field::Experience(2), 9000, 0.0);
        assert!(!session.is_dirty(), "there is no experience to change");
        session.set(Field::LevelUps(2), 1, 10.0);
        session.set(Field::Allocated(2, 3), 9, 20.0);
        session.set(Field::Gold(2, 0), 900, 30.0);
        assert_eq!(session.changes()[0].who, "Gale");

        // Approval is the one value that goes below zero.
        assert_eq!(session.value(Field::Approval(2, 0)), Some(10));
        assert_eq!(session.range(Field::Approval(2, 0)), -50..=100);
        session.set(Field::Approval(2, 0), -500, 40.0);
        assert_eq!(session.value(Field::Approval(2, 0)), Some(-50));
        session.set(Field::Approval(2, 0), -20, 50.0);
        let change = session.changes().pop().unwrap();
        assert_eq!(
            (change.who.as_str(), change.what.as_str()),
            ("Gale", "Approval")
        );
        assert_eq!(
            (change.before.as_str(), change.after.as_str()),
            ("10", "-20")
        );
        assert_eq!(session.value(Field::Approval(1, 0)), None, "no rating yet");

        let exported = session.export(&dir, "Camp edit").unwrap();
        let reopened = Session::open(&exported).unwrap();
        let gale = &reopened.party().characters[2];
        assert_eq!(gale.approval[0].rating, -20);
        assert_eq!((gale.level, gale.entitled_level), (4, 5));
        assert_eq!(gale.allocated, Some([0, 5, 6, 9, 2, 4]));
        assert_eq!(gale.gold_total(), 900);
        assert_eq!(reopened.party().gold_total(), 3756, "the party's own gold");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_the_saves_mods() {
        let dir = temp_dir("mods");
        let session = open(&dir);
        let names: Vec<&str> = session.mods().iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["GustavX", "Faerun Colors", "Better UI"]);
        assert!(session.mods()[0].base && !session.mods()[1].base);
        assert!(session.active_mods().is_none(), "not in the game's folders");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A catalog of the sample's rope, a rope it has nowhere, a weapon and a scroll.
    fn sample_catalog(dir: &Path) -> Arc<Catalog> {
        use crate::catalog::build::{game_folder_with, GameItem};
        let item = |template, stats, display, kind, category| GameItem {
            template,
            stats,
            display,
            kind,
            category,
            max_stack: 1,
        };
        let data = game_folder_with(
            &dir.join("game"),
            &[
                item(
                    "00000000-0000-0000-0000-00000000cccc",
                    "OBJ_Rope",
                    "Rope",
                    Kind::Object,
                    "Tool",
                ),
                item(
                    "00000000-0000-0000-0000-00000000ffff",
                    "OBJ_Rope_Silk",
                    "Silk Rope",
                    Kind::Object,
                    "Tool",
                ),
                item(
                    "00000000-0000-0000-0000-000000001111",
                    "WPN_Longsword_1",
                    "Longsword +1",
                    Kind::Weapon,
                    "",
                ),
                item(
                    "00000000-0000-0000-0000-000000005555",
                    "WPN_Longsword",
                    "Longsword",
                    Kind::Weapon,
                    "",
                ),
                item(
                    "00000000-0000-0000-0000-00000000bbbb",
                    "OBJ_Bag_AlchemyPouch",
                    "Alchemy Pouch",
                    Kind::Object,
                    "Container",
                ),
                item(
                    "00000000-0000-0000-0000-000000002222",
                    "OBJ_Scroll_Fireball",
                    "Scroll of Fireball",
                    Kind::Object,
                    "MagicScroll_3",
                ),
            ],
        )
        .unwrap();
        Arc::new(Catalog::from_game(&data).unwrap())
    }

    #[test]
    fn adds_items_copied_from_what_the_holder_carries() {
        use party::build::{uuid, ROPE, TRADER_SWORD};
        use party::Holder;
        let dir = temp_dir("add");
        let mut session = open(&dir);
        session.set_catalog(sample_catalog(&dir));
        let hero = Holder::Character(0);

        // The same item: a plain copy of the hero's rope.
        let rope = session.add_item(hero, "OBJ_Rope", 0.0).unwrap();
        assert!(session.additions()[rope].plan.same);
        assert_eq!(session.additions()[rope].plan.donor, uuid(ROPE));
        // Another of the same kind, made from the rope.
        let silk = session.add_item(hero, "OBJ_Rope_Silk", 10.0).unwrap();
        assert!(!session.additions()[silk].plan.same);
        assert_eq!(session.additions()[silk].plan.donor, uuid(ROPE));
        // Nobody in the party has a weapon, but the trader has one equipped.
        let sword = session.add_item(hero, "WPN_Longsword_1", 20.0).unwrap();
        assert_eq!(session.additions()[sword].plan.donor, uuid(TRADER_SWORD));
        assert!(session
            .addition_hint(sword)
            .starts_with("Made from a Longsword someone outside the party has"));
        // There is no scroll anywhere, so a scroll is made from something else; its spell is
        // left for the game to fill in.
        let catalog = session.catalog().unwrap();
        let fireball = catalog.by_stats("OBJ_Scroll_Fireball").unwrap().clone();
        let plan = session.pattern_for(hero, &fireball).unwrap();
        assert!(plan.spell.is_some());
        assert_eq!(plan.donor, uuid(ROPE));
        // Something nobody can pick up, such as a Bag of Rotten Food meant for throwing in a
        // fight, would be a blank slot in an inventory.
        let mut thrown = catalog.by_stats("OBJ_Rope").unwrap().clone();
        thrown.pickable = false;
        let why = session.pattern_for(hero, &thrown).unwrap_err();
        assert!(why.contains("can't be picked up"), "{why}");
        // A bag can't be added: a copy would have no inventory of its own.
        let bag = session
            .add_item(hero, "OBJ_Bag_AlchemyPouch", 35.0)
            .unwrap_err();
        assert!(bag.contains("inventory of its own"), "{bag}");
        // The camp chest gets a copy of the hero's rope.
        let chest_rope = session
            .add_item(Holder::CampChest, "OBJ_Rope", 40.0)
            .unwrap();
        assert_eq!(session.addition_hint(chest_rope), "A copy of one Tav holds");

        // Asking again adds one more; each is a change that can be undone.
        assert_eq!(session.add_item(hero, "OBJ_Rope", 50.0).unwrap(), rope);
        assert_eq!(session.value(Field::Add(rope)), Some(2));
        let whats: Vec<String> = session.changes().iter().map(|c| c.what.clone()).collect();
        assert_eq!(
            whats,
            ["Add Rope", "Add Silk Rope", "Add Longsword +1", "Add Rope"]
        );
        session.undo();
        assert_eq!(session.value(Field::Add(rope)), Some(1));
        assert!(session.is_dirty());

        let exported = session.export(&dir, "With more rope").unwrap();
        let reopened = Session::open(&exported).unwrap();
        let items_of = |holder: Holder| -> Vec<String> {
            reopened
                .original()
                .inventories
                .iter()
                .find(|i| i.holder == holder)
                .unwrap()
                .items
                .iter()
                .map(|i| i.stats.clone())
                .collect()
        };
        let hero_items = items_of(hero);
        let count = |items: &[String], stats: &str| items.iter().filter(|s| *s == stats).count();
        assert_eq!(count(&hero_items, "OBJ_Rope"), 2);
        assert_eq!(count(&hero_items, "OBJ_Rope_Silk"), 1);
        assert_eq!(count(&hero_items, "WPN_Longsword_1"), 1);
        assert_eq!(count(&items_of(Holder::CampChest), "OBJ_Rope"), 1);
        assert_eq!(reopened.party().characters, session.party().characters);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exports_are_manual_saves_except_in_honour_mode() {
        use crate::saves::build::{save_folder, SaveSpec};
        let dir = temp_dir("types");
        for (name, kind, after) in [("AutoSave_9", 2, 0), ("HonourMode", 3, 3), ("Mine", 0, 0)] {
            let mut spec = SaveSpec::new("Tav", name);
            spec.save_type = kind;
            let folder = save_folder(&dir, &spec).unwrap();
            let mut package = Save::open(&folder).unwrap().package.clone();
            let changed = save::as_manual_save(&mut package).unwrap();
            assert_eq!(changed, kind != after, "{name}");
            assert_eq!(
                save::save_type(&package).unwrap(),
                Some(after as i64),
                "{name}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exports_a_new_save_and_leaves_the_original() {
        let dir = temp_dir("export");
        let folder = build::sample_save(&dir, "QuickSave_3").unwrap();
        let original_bytes = std::fs::read(folder.join("QuickSave_3.lsv")).unwrap();
        let mut session = Session::open(&folder).unwrap();
        session.set(Field::Experience(0), 7100, 0.0);
        session.set(Field::LevelUps(0), 1, 10.0);
        session.set(Field::Allocated(0, 0), 7, 20.0);
        // Makes the bagged stack the larger one, which reorders the stacks when read back.
        session.set(Field::Gold(0, 1), 5000, 30.0);
        // Past the number the save keeps beside it, which the export raises too.
        session.set(Field::Inspiration, 6, 40.0);

        assert_eq!(session.default_export_folder(), dir);
        let name = session.suggested_name(&dir);
        assert_eq!(name, "QuickSave_3 - Edited");
        let exported = session.export(&dir, &name).unwrap();
        assert_eq!(exported, dir.join("Tav-1234__QuickSave_3 - Edited"));
        // A quicksave exported is the player's own save, shown by its name.
        assert_eq!(save::save_type(&session.save().package).unwrap(), Some(1));
        let written = Save::open(&exported.join("QuickSave_3 - Edited.lsv")).unwrap();
        assert_eq!(save::save_type(&written.package).unwrap(), Some(0));
        assert!(exported.join("QuickSave_3 - Edited.lsv").is_file());
        assert!(exported.join("QuickSave_3 - Edited.WebP").is_file());
        assert_eq!(
            std::fs::read(folder.join("QuickSave_3.lsv")).unwrap(),
            original_bytes,
            "the opened save is untouched"
        );

        // The exported save opens with the changes as its starting point.
        let reopened = Session::open(&exported).unwrap();
        assert!(!reopened.is_dirty());
        // The hero's experience and level-ups no longer match the save's summary, which the
        // game writes anew only when it next saves, but they are still first and found there.
        assert_eq!(reopened.order()[0], 0);
        assert!(reopened.summary_of(0).is_some());
        let hero = &reopened.party().characters[0];
        assert_eq!((hero.experience, hero.entitled_level), (Some(7100), 6));
        assert_eq!(hero.allocated, Some([7, 6, 7, 0, 2, 7]));
        assert_eq!(hero.gold_total(), 8385);
        assert_eq!(reopened.value(Field::Inspiration), Some(6));

        // The name is now taken, here and for the next suggestion.
        assert!(session.export(&dir, &name).is_err());
        assert_eq!(session.suggested_name(&dir), "QuickSave_3 - Edited 2");
        // An export with no changes is a plain copy that opens the same.
        let plain = Session::open(&folder).unwrap();
        let copy = plain.export(&dir, "Copy").unwrap();
        assert_eq!(Session::open(&copy).unwrap().party(), plain.party());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_saves_it_cannot_edit() {
        let dir = temp_dir("refuse");
        // A save with no Globals.lsf.
        let folder =
            saves::build::save_folder(&dir, &saves::build::SaveSpec::new("Tav", "Bare")).unwrap();
        let error = Session::open(&folder).err().unwrap().to_string();
        assert!(error.contains("Globals.lsf"), "{error}");
        assert!(Session::open(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
