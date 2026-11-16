//! A save as a whole: the package, the summary the game writes beside the data, and whether
//! this editor may change it.

use crate::lsf::{Header, Lsf, Sections};
use crate::lspk::Package;
use crate::value::Value;
use anyhow::{anyhow, bail, ensure, Context, Result};
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

/// Writes `package` as a new save called `name`, in a new folder under `parent`.
///
/// What the game needs, found by trying it:
///
/// * The load menu lists a save under its folder's name, the part after `__`.
/// * The `.lsv` and the screenshot inside must carry that same name. With the files still
///   named after another save, the entry shows no screenshot and loading hangs at 0%.
/// * The names inside the package (the screenshot entry, `Save Name` in `SaveInfo.json`) can
///   stay as they were.
///
/// So the new folder is `<prefix>__<name>`, with the source's prefix, holding `<name>.lsv` and
/// the source's screenshot as `<name>.WebP`. Nothing is overwritten.
/// The save type `meta.lsf` gives a save: 0 manual, 1 quick, 2 auto, 3 Honour Mode.
pub fn save_type(package: &Package) -> Result<Option<i64>> {
    let meta = Lsf::parse(&package.read_file(META)?).context(META)?;
    Ok(meta
        .find(&["MetaData", "MetaData"])
        .and_then(|root| meta.get(root, "SaveGameType"))
        .and_then(|v| v.as_i64()))
}

/// Makes a quicksave or autosave a manual save, as an export is the player's own save. The
/// game shows its own quicksaves and autosaves by a label of its own rather than their
/// names, and replaces them in turn. An Honour Mode save is left as it is. Returns whether
/// anything changed.
pub fn as_manual_save(package: &mut Package) -> Result<bool> {
    let mut meta = Lsf::parse(&package.read_file(META)?).context(META)?;
    let Some(attr) = meta
        .find(&["MetaData", "MetaData"])
        .and_then(|root| meta.attr(root, "SaveGameType"))
    else {
        return Ok(false);
    };
    if !matches!(meta.value(attr)?, Value::Byte(1 | 2)) {
        return Ok(false);
    }
    meta.set_value(attr, &Value::Byte(0))?;
    package.replace(META, &meta.write()?)?;
    Ok(true)
}

pub fn export_copy(source: &Save, package: &Package, parent: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && name == name.trim()
            && !name.ends_with('.')
            && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']),
        "{name:?} can't be used as a save name"
    );
    let source_folder = source.path.parent().unwrap_or(Path::new("."));
    let prefix = source_folder
        .file_name()
        .and_then(|f| f.to_str())
        .and_then(|f| f.split_once("__"))
        .map_or("Nautiloid", |(prefix, _)| prefix);
    let folder = parent.join(format!("{prefix}__{name}"));
    ensure!(!folder.exists(), "{} already exists", folder.display());

    // The screenshot beside the .lsv: the one named after it, or failing that any other.
    let mut screenshot: Option<PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(source_folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("webp"))
            {
                let named_after_save = path.file_stem() == source.path.file_stem();
                if named_after_save || screenshot.is_none() {
                    screenshot = Some(path);
                }
                if named_after_save {
                    break;
                }
            }
        }
    }

    let bytes = package.write()?;
    // Read the result back before anything is created.
    let written = Package::read(bytes.clone())?;
    ensure!(
        written.compute_md5()? == written.stored_md5(),
        "the written package fails its own checksum"
    );
    std::fs::create_dir_all(&folder).with_context(|| folder.display().to_string())?;
    std::fs::write(folder.join(format!("{name}.lsv")), &bytes)
        .with_context(|| folder.display().to_string())?;
    if let Some(path) = screenshot {
        // Keep the extension as the game wrote it (`.WebP`).
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("WebP");
        std::fs::copy(&path, folder.join(format!("{name}.{extension}")))
            .with_context(|| path.display().to_string())?;
    }
    Ok(folder)
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

    #[test]
    fn exports_a_copy_as_a_new_folder() {
        let root = std::env::temp_dir().join(format!("nautiloid-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let source_folder = root.join("Tav-123__QuickSave_1");
        std::fs::create_dir_all(&source_folder).unwrap();
        let bytes = crate::lspk::build(&[("meta.lsf", b"one".as_slice())]).unwrap();
        std::fs::write(source_folder.join("QuickSave_1.lsv"), &bytes).unwrap();
        std::fs::write(source_folder.join("QuickSave_1.WebP"), b"image").unwrap();

        // A screenshot that belongs to something else must not be the one copied.
        std::fs::write(source_folder.join("Another.webp"), b"other").unwrap();

        let save = Save::open(&source_folder).unwrap();
        let folder = export_copy(&save, &save.package, &root, "Edited - 2").unwrap();
        assert_eq!(folder, root.join("Tav-123__Edited - 2"));
        // The files inside are named after the new save, or the game will not load it.
        let mut names: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["Edited - 2.WebP", "Edited - 2.lsv"]);
        assert_eq!(std::fs::read(folder.join("Edited - 2.lsv")).unwrap(), bytes);
        assert_eq!(
            std::fs::read(folder.join("Edited - 2.WebP")).unwrap(),
            b"image"
        );
        // The exported save opens like any other.
        assert!(Save::open(&folder).is_ok());

        // The original is untouched, and a second export of the same name is refused.
        assert_eq!(
            std::fs::read(source_folder.join("QuickSave_1.lsv")).unwrap(),
            bytes
        );
        assert!(export_copy(&save, &save.package, &root, "Edited - 2").is_err());
        assert!(export_copy(&save, &save.package, &root, "a/b").is_err());
        assert!(export_copy(&save, &save.package, &root, " ").is_err());
        assert!(export_copy(&save, &save.package, &root, "Trailing ").is_err());
        assert!(export_copy(&save, &save.package, &root, "Dot.").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
