//! An editing session: a save that is open, the changes made to it so far, and the way out.
//!
//! Nothing here touches the save on disk until [`Session::export`], and that writes a new
//! save folder; the one that was opened is never modified. Every change is a difference
//! between the party as it was read and the party as it now stands, so the list of changes
//! is always exactly what an export would write, and reverting one puts back exactly what
//! was there.

use crate::lsf::Lsf;
use crate::newage::NewAge;
use crate::party::{self, Character, Party};
use crate::save::{self, Save};
use crate::saves::{self, SaveSummary};
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

/// Most steps that can be undone.
const UNDO_LIMIT: usize = 200;
/// Edits to one field less than this many seconds apart count as one step to undo.
const MERGE_WINDOW: f64 = 1.0;
/// The most gold this editor will put in one stack. The game's top bar shows up to seven
/// digits comfortably.
pub const MAX_GOLD: u32 = 9_999_999;
/// The most experience this editor will set.
pub const MAX_EXPERIENCE: u32 = 9_999_999;

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
    before: u32,
    after: u32,
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
}

fn field_of(party: &Party, field: Field) -> Option<u32> {
    let character = |i: usize| party.characters.get(i);
    match field {
        Field::Experience(c) => character(c).map(|c| c.experience),
        Field::LevelUps(c) => character(c).map(|c| c.entitled_level.saturating_sub(c.level)),
        Field::Allocated(c, ability) => character(c)?.allocated?.get(ability).copied(),
        Field::Gold(c, stack) => character(c)?.gold.get(stack).map(|g| g.amount),
        Field::Inspiration => party.inspiration.map(|i| i.amount),
    }
}

fn set_field(party: &mut Party, field: Field, value: u32) {
    match field {
        Field::Experience(c) => party.characters[c].experience = value,
        Field::LevelUps(c) => {
            let character = &mut party.characters[c];
            character.entitled_level = character.level + value;
        }
        Field::Allocated(c, ability) => {
            if let Some(allocated) = party.characters[c].allocated.as_mut() {
                allocated[ability] = value;
            }
        }
        Field::Gold(c, stack) => party.characters[c].gold[stack].amount = value,
        Field::Inspiration => {
            if let Some(inspiration) = party.inspiration.as_mut() {
                inspiration.amount = value;
            }
        }
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
        })
    }

    pub fn summary(&self) -> &SaveSummary {
        &self.summary
    }

    pub fn save(&self) -> &Save {
        &self.save
    }

    /// The party with the changes made so far.
    pub fn party(&self) -> &Party {
        &self.current
    }

    /// The party as the save has it.
    pub fn original(&self) -> &Party {
        &self.original
    }

    /// The field's value now, or `None` if this save has no such field (a character with no
    /// character creation record has no allocations; a new game has no Inspiration).
    pub fn value(&self, field: Field) -> Option<u32> {
        field_of(&self.current, field)
    }

    /// The field's value in the save as opened.
    pub fn original_value(&self, field: Field) -> Option<u32> {
        field_of(&self.original, field)
    }

    pub fn is_changed(&self, field: Field) -> bool {
        self.value(field) != self.original_value(field)
    }

    /// The values the field can take.
    pub fn range(&self, field: Field) -> RangeInclusive<u32> {
        match field {
            Field::Experience(_) => 0..=MAX_EXPERIENCE,
            Field::LevelUps(c) => {
                let level = self.original.characters.get(c).map_or(0, |c| c.level);
                0..=party::MAX_LEVEL.saturating_sub(level)
            }
            Field::Allocated(..) => 0..=party::MAX_ALLOCATED,
            Field::Gold(..) => 1..=MAX_GOLD,
            Field::Inspiration => 0..=self.original.inspiration.map_or(0, |i| i.max),
        }
    }

    /// Sets a field, clamped to its range. `time` is the current time in seconds, used to
    /// fold quick repeated edits to one field into a single step to undo.
    pub fn set(&mut self, field: Field, value: u32, time: f64) {
        let Some(before) = self.value(field) else {
            return;
        };
        let range = self.range(field);
        let value = value.clamp(*range.start(), *range.end());
        if value == before {
            return;
        }
        set_field(&mut self.current, field, value);
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
        set_field(&mut self.current, step.field, step.before);
        self.redo.push(step);
        self.last_edit = None;
        Some(step.field)
    }

    /// Redoes the last step undone and returns the field it was to.
    pub fn redo(&mut self) -> Option<Field> {
        let step = self.redo.pop()?;
        set_field(&mut self.current, step.field, step.after);
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
        }
        if self.original.inspiration.is_some() {
            fields.push(Field::Inspiration);
        }
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
                let several = character.is_some_and(|c| c.gold.len() > 1);
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
        }
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
        self.current != self.original
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
            match change.field {
                Field::Experience(c) => {
                    party::set_experience(&mut entities, character(c).entity, value)?
                }
                Field::LevelUps(c) => party::set_entitled_level(
                    &mut entities,
                    character(c).entity,
                    character(c).level + value,
                )?,
                Field::Allocated(c, ability) => {
                    party::set_allocated(&mut entities, character(c).entity, ability, value)?
                }
                Field::Gold(c, stack) => {
                    party::set_gold(&mut entities, character(c).gold[stack].item, value)?
                }
                Field::Inspiration => party::set_inspiration(&mut entities, value)?,
            }
        }
        Ok(entities)
    }

    /// Writes the save with its changes as a new save called `name` in `parent`, and returns
    /// the new folder. The save that was opened is not touched. The result is read back and
    /// checked before this returns; if it does not hold what was asked for, it is removed.
    pub fn export(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let entities = self.edited_entities()?;
        let mut globals = self.globals.clone();
        entities.store(&mut globals)?;
        let mut package = self.save.package.clone();
        package.replace(save::GLOBALS, &globals.write()?)?;
        let folder = save::export_copy(&self.save, &package, parent, name)?;

        let check = || -> Result<()> {
            let written = Save::open(&folder)?;
            let globals = written.lsf(save::GLOBALS)?;
            let party = party::read(&globals, &NewAge::from_lsf(&globals)?)?;
            ensure!(
                same_party(&party, &self.current),
                "the exported save does not hold the changes"
            );
            for entry in self.save.package.entries() {
                if entry.name() != save::GLOBALS {
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
        spec.files = vec![
            (save::GLOBALS.to_owned(), party::build::globals_lsf()),
            (save::STORY.to_owned(), vec![7u8; 4096]),
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

    #[test]
    fn opens_and_reports_fields() {
        let dir = temp_dir("open");
        let session = open(&dir);
        assert_eq!(session.summary().name, "QuickSave_3");
        assert_eq!(session.party().characters.len(), 2);
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
        assert_eq!(session.range(Field::Inspiration), 0..=4);
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
            Some(4),
            "clamped to the save's maximum"
        );
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
        session.set(Field::Inspiration, 4, 40.0);

        assert_eq!(session.default_export_folder(), dir);
        let name = session.suggested_name(&dir);
        assert_eq!(name, "QuickSave_3 - Edited");
        let exported = session.export(&dir, &name).unwrap();
        assert_eq!(exported, dir.join("Tav-1234__QuickSave_3 - Edited"));
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
        let hero = &reopened.party().characters[0];
        assert_eq!((hero.experience, hero.entitled_level), (7100, 6));
        assert_eq!(hero.allocated, Some([7, 6, 7, 0, 2, 7]));
        assert_eq!(hero.gold_total(), 8385);
        assert_eq!(reopened.value(Field::Inspiration), Some(4));

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
