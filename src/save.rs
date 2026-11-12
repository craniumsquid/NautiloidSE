//! A save as a whole: the package, the summary the game writes beside the data, and whether
//! this editor may change it.

use crate::lsf::{Header, Lsf, Sections};
use crate::lspk::Package;
use crate::newage::NewAge;
use crate::value::Uuid;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};

/// The game patch this editor writes saves for. The engine version in each `.lsf` header is
/// `4.<patch>`: 4.4 and 4.5 in the Patch 4 and 5 samples, 4.8 in every Patch 8 one.
pub const SUPPORTED_ENGINE: (u32, u32) = (4, 8);

pub const META: &str = "meta.lsf";
pub const GLOBALS: &str = "Globals.lsf";
pub const SAVE_INFO: &str = "SaveInfo.json";
pub const STORY: &str = "StorySave.bin";

/// One class a party member has levels in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassEntry {
    pub main: String,
    pub sub: String,
}

/// A party member as `SaveInfo.json` describes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyMember {
    /// `Generic` for a custom character, `DarkUrge`, or a companion's name.
    pub origin: String,
    pub race: String,
    pub level: u32,
    pub classes: Vec<ClassEntry>,
    pub experience_total: u64,
    pub experience_in_level: u64,
}

/// The summary in `SaveInfo.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveInfo {
    /// For example `4.1.1.7631656`.
    pub game_version: String,
    pub save_name: String,
    /// The level the party is in, for example `WLD_Main_A`.
    pub current_level: String,
    pub platform: String,
    /// For example `["DifficultyHard", "RulesetHonour"]`. Empty when the save has custom rules.
    pub difficulty: Vec<String>,
    pub party: Vec<PartyMember>,
}

fn text(json: &Json, key: &str) -> String {
    json.get(key)
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn number(json: &Json, key: &str) -> u64 {
    json.get(key).and_then(Json::as_u64).unwrap_or_default()
}

impl SaveInfo {
    pub fn parse(bytes: &[u8]) -> Result<SaveInfo> {
        let json: Json = serde_json::from_slice(bytes).context("SaveInfo.json")?;
        let characters = json
            .get("Active Party")
            .and_then(|p| p.get("Characters"))
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default();
        let party = characters
            .iter()
            .map(|c| PartyMember {
                origin: text(c, "Origin"),
                race: text(c, "Race"),
                level: number(c, "Level") as u32,
                classes: c
                    .get("Classes")
                    .and_then(Json::as_array)
                    .map(|classes| {
                        classes
                            .iter()
                            .map(|class| ClassEntry {
                                main: text(class, "Main"),
                                sub: text(class, "Sub"),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                experience_total: number(c, "Experience Points (Total)"),
                experience_in_level: number(c, "Experience Points (Current level)"),
            })
            .collect();
        Ok(SaveInfo {
            game_version: text(&json, "Game Version"),
            save_name: text(&json, "Save Name"),
            current_level: text(&json, "Current Level"),
            platform: text(&json, "Platform"),
            difficulty: json
                .get("Difficulty")
                .and_then(Json::as_array)
                .map(|d| {
                    d.iter()
                        .filter_map(Json::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            party,
        })
    }

    pub fn is_honour_mode(&self) -> bool {
        self.difficulty.iter().any(|d| d == "RulesetHonour")
    }
}

/// Whether the editor may change a save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Editable,
    /// Listed and readable, but not written. The text says why.
    ReadOnly(String),
}

/// A save opened from disk.
pub struct Save {
    /// The `.lsv` file.
    pub path: PathBuf,
    pub package: Package,
}

impl Save {
    /// Opens a `.lsv` file, or the save folder that holds one.
    pub fn open(path: &Path) -> Result<Save> {
        let path = if path.is_dir() {
            let mut found = Vec::new();
            for entry in std::fs::read_dir(path).with_context(|| path.display().to_string())? {
                let file = entry?.path();
                if file
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("lsv"))
                {
                    found.push(file);
                }
            }
            match found.len() {
                0 => bail!("{} has no .lsv file", path.display()),
                1 => found.remove(0),
                n => bail!("{} has {n} .lsv files; name one", path.display()),
            }
        } else {
            path.to_owned()
        };
        let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
        let package = Package::read(bytes).with_context(|| path.display().to_string())?;
        Ok(Save { path, package })
    }

    /// The summary from `SaveInfo.json`.
    pub fn info(&self) -> Result<SaveInfo> {
        SaveInfo::parse(&self.package.read_file(SAVE_INFO)?)
    }

    /// Parses one of the save's `.lsf` documents.
    pub fn lsf(&self, name: &str) -> Result<Lsf> {
        Lsf::parse(&self.package.read_file(name)?).with_context(|| name.to_owned())
    }

    /// Names of the save's `.lsf` documents, in file list order.
    pub fn lsf_names(&self) -> Vec<&str> {
        self.package
            .entries()
            .iter()
            .map(|e| e.name())
            .filter(|n| n.ends_with(".lsf"))
            .collect()
    }

    /// The header of `meta.lsf`, which carries the engine version that wrote the save.
    pub fn meta_header(&self) -> Result<Header> {
        Ok(Sections::read(&self.package.read_file(META)?)
            .context(META)?
            .header)
    }

    /// Whether this save was written by the supported patch.
    pub fn support(&self) -> Support {
        let header = match self.meta_header() {
            Ok(header) => header,
            Err(error) => return Support::ReadOnly(format!("{error:#}")),
        };
        let (major, minor) = header.engine_major_minor();
        if (major, minor) == SUPPORTED_ENGINE {
            Support::Editable
        } else {
            let version = self
                .info()
                .map(|i| i.game_version)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| header.engine_version_text());
            Support::ReadOnly(format!(
                "Written by game version {version} (engine {major}.{minor}). Only Patch {} saves \
                 can be edited; load it in the current game and save again.",
                SUPPORTED_ENGINE.1
            ))
        }
    }
}

/// The mods a save was made with, from `meta.lsf`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModEntry {
    pub name: String,
    pub folder: String,
    pub uuid: String,
}

/// Reads the mod list out of a parsed `meta.lsf`.
pub fn mods(meta: &Lsf) -> Result<Vec<ModEntry>> {
    let list = meta
        .find(&["MetaData", "MetaData", "ModuleSettings", "Mods"])
        .ok_or_else(|| anyhow!("meta.lsf has no mod list"))?;
    let field = |node, name: &str| {
        meta.get(node, name)
            .and_then(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.as_uuid().map(|u| u.to_string()))
            })
            .unwrap_or_default()
    };
    Ok(meta
        .children_named(list, "ModuleShortDesc")
        .map(|m| ModEntry {
            name: field(m, "Name"),
            folder: field(m, "Folder"),
            uuid: field(m, "UUID"),
        })
        .collect())
}

/// Tables and field offsets in the entity data. Each was found by matching known values
/// (the party's level and experience from `SaveInfo.json`) across the sample saves.
pub mod fields {
    /// Rows of 12 bytes. The first two u32 both hold the character's total experience.
    pub const EXPERIENCE: &str = "game.experience.v0.ExperienceComponent";
    pub const EXPERIENCE_TOTAL: [usize; 2] = [0, 4];
    /// Rows of 4 bytes: the character level.
    pub const LEVEL: &str = "game.stats.v0.LevelComponent";
    /// Rows of 36 bytes. Strength, Dexterity, Constitution, Intelligence, Wisdom and Charisma
    /// are six u32 starting at offset 8.
    pub const STATS: &str = "game.stats.v3.StatsComponent";
    pub const STATS_ABILITIES: usize = 8;
    /// One row of 4 bytes: the camp supplies total.
    pub const CAMP_SUPPLIES: &str = "game.camp.v0.TotalSuppliesComponent";
}

/// A party member as the entity data in `Globals.lsf` describes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Character {
    /// The entity's identifier in the save.
    pub entity: Uuid,
    /// The template the character was created from: a companion's fixed identifier, or the
    /// base template of a custom character.
    pub template: String,
    pub level: u32,
    pub experience: u32,
    /// Strength, Dexterity, Constitution, Intelligence, Wisdom, Charisma.
    pub abilities: [u32; 6],
}

/// The characters that have experience, which is the party, in the order the save lists them.
pub fn party(globals: &Lsf, entities: &NewAge) -> Result<Vec<Character>> {
    // The node tree lists every character's entity identifier beside its template.
    let mut templates = std::collections::HashMap::new();
    if let Some(creators) = globals.find(&["Characters", "CharacterFactory", "Creators"]) {
        for creator in globals.children_named(creators, "Creator") {
            if let (Some(entity), Some(template)) = (
                globals.get(creator, "Entity").and_then(|v| v.as_uuid()),
                globals.get(creator, "TemplateID"),
            ) {
                templates.insert(entity, template.as_str().unwrap_or_default().to_owned());
            }
        }
    }
    let experience = entities
        .table(fields::EXPERIENCE)
        .ok_or_else(|| anyhow!("the save has no {} table", fields::EXPERIENCE))?;
    let mut party = Vec::new();
    for row in 0..entities.tables()[experience].rows {
        let index = entities
            .owner(experience, row)
            .ok_or_else(|| anyhow!("{} is not a component table", fields::EXPERIENCE))?;
        let entity = entities.entity_id(index);
        let mut abilities = [0u32; 6];
        for (i, ability) in abilities.iter_mut().enumerate() {
            *ability = entities.get_u32(fields::STATS, index, fields::STATS_ABILITIES + i * 4)?;
        }
        party.push(Character {
            entity,
            template: templates.get(&entity).cloned().unwrap_or_default(),
            level: entities.get_u32(fields::LEVEL, index, 0)?,
            experience: entities.get_u32(fields::EXPERIENCE, index, fields::EXPERIENCE_TOTAL[0])?,
            abilities,
        });
    }
    Ok(party)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_summary() {
        let json = br#"{"Active Party":{"Characters":[
            {"Classes":[{"Main":"Sorcerer","Sub":"DraconicBloodline"},{"Main":"Warlock","Sub":"WarlockUndead"}],
             "Experience Points (Current level)":1122,"Experience Points (Total)":4372,"Level":5,
             "Origin":"Generic","Position":[1.0,2.0,3.0],"Race":"Drow_LolthSworn","Subregion":""},
            {"Level":10,"Origin":"Djinni_PlanarAlly "}]},
            "Current Level":"WLD_Main_A","Difficulty":["DifficultyHard","RulesetHonour"],
            "Game Version":"4.1.1.7631656","Platform":"Steam","Save Name":"AutoSave_19"}"#;
        let info = SaveInfo::parse(json).unwrap();
        assert_eq!(info.game_version, "4.1.1.7631656");
        assert_eq!(info.current_level, "WLD_Main_A");
        assert!(info.is_honour_mode());
        assert_eq!(info.party.len(), 2);
        assert_eq!(info.party[0].level, 5);
        assert_eq!(info.party[0].classes[1].sub, "WarlockUndead");
        assert_eq!(info.party[0].experience_total, 4372);
        // A summon in the party has no classes.
        assert!(info.party[1].classes.is_empty());

        let custom = SaveInfo::parse(br#"{"Game Version":"4.1.1.7631656"}"#).unwrap();
        assert!(custom.difficulty.is_empty() && !custom.is_honour_mode());
        assert!(SaveInfo::parse(b"not json").is_err());
    }
}
