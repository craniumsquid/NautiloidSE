//! Finding saves on disk and describing them for the save browser.
//!
//! A save is a folder named `<prefix>__<name>` holding `<name>.lsv` and `<name>.WebP`. The
//! game lists it under `<name>`. Everything the browser shows comes from two small files in
//! the package, `meta.lsf` and `SaveInfo.json`, which are read without loading the rest.

use crate::lsf::Lsf;
use crate::lspk;
use crate::save::{self, PartyMember, SaveInfo, Support, SUPPORTED_ENGINE};
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

/// How a save was made, from `SaveGameType` in `meta.lsf`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    Manual,
    Quick,
    Auto,
    Honour,
}

impl SaveKind {
    pub fn label(self) -> &'static str {
        match self {
            SaveKind::Manual => "Manual",
            SaveKind::Quick => "Quick",
            SaveKind::Auto => "Auto",
            SaveKind::Honour => "Honour",
        }
    }
}

/// What the browser shows for one save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveSummary {
    /// The save's folder.
    pub folder: PathBuf,
    /// The `.lsv` inside it.
    pub lsv: PathBuf,
    /// The screenshot beside the `.lsv`, if there is one.
    pub screenshot: Option<PathBuf>,
    /// The name the game lists the save under: the folder's name after `__`.
    pub name: String,
    /// Identifies the playthrough; every save of one campaign shares it.
    pub campaign: String,
    /// The name of the character leading the party.
    pub leader: String,
    pub kind: SaveKind,
    /// When the save was made, in seconds since 1970.
    pub saved_at: u64,
    /// For example `4.1.1.7631656`.
    pub game_version: String,
    pub support: Support,
    /// The level the party is in, for example `WLD_Main_A`.
    pub level: String,
    pub party: Vec<PartyMember>,
    pub honour_mode: bool,
    /// Names of the mods the save was made with, the game's own modules left out.
    pub mods: Vec<String>,
}

impl SaveSummary {
    /// The highest level in the party.
    pub fn party_level(&self) -> u32 {
        self.party.iter().map(|m| m.level).max().unwrap_or(0)
    }
}

/// A folder that looks like a save but could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreadable {
    pub folder: PathBuf,
    pub name: String,
    pub error: String,
}

/// One entry in a saves folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listed {
    Save(Box<SaveSummary>),
    Unreadable(Unreadable),
}

/// The name the game lists a save folder under.
pub fn display_name(folder: &Path) -> String {
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.split_once("__") {
        Some((_, shown)) if !shown.is_empty() => shown.to_owned(),
        _ => name,
    }
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

/// The `.lsv` in a folder, if it has exactly one.
fn lsv_in(folder: &Path) -> Option<PathBuf> {
    let mut found = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| has_extension(p, "lsv") && p.is_file());
    match (found.next(), found.next()) {
        (Some(lsv), None) => Some(lsv),
        _ => None,
    }
}

/// Whether `folder` holds a save.
pub fn is_save_folder(folder: &Path) -> bool {
    folder.is_dir() && lsv_in(folder).is_some()
}

fn text(lsf: &Lsf, node: u32, name: &str) -> String {
    lsf.get(node, name)
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Reads what the browser needs to know about the save in `folder`.
pub fn summarize(folder: &Path) -> Result<SaveSummary> {
    let lsv = lsv_in(folder).ok_or_else(|| anyhow!("the folder has no single .lsv file"))?;
    let name = display_name(folder);
    // The screenshot named after the save, or failing that any in the folder.
    let screenshot = std::fs::read_dir(folder)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| has_extension(p, "webp"))
        .max_by_key(|p| p.file_stem() == lsv.file_stem());

    let mut files = lspk::peek(&lsv, &[save::META, save::SAVE_INFO])?.into_iter();
    let (Some(Some(meta)), Some(info)) = (files.next(), files.next()) else {
        bail!("the save has no {}", save::META);
    };
    let meta = Lsf::parse(&meta).context(save::META)?;
    let root = meta
        .find(&["MetaData", "MetaData"])
        .ok_or_else(|| anyhow!("{} has no MetaData", save::META))?;
    let info = match info {
        Some(bytes) => SaveInfo::parse(&bytes)?,
        None => SaveInfo::default(),
    };

    let kind = match meta.get(root, "SaveGameType").and_then(|v| v.as_i64()) {
        Some(3) => SaveKind::Honour,
        Some(2) => SaveKind::Auto,
        Some(1) => SaveKind::Quick,
        // Older saves record the type differently; their names still say what they are.
        _ if name.starts_with("QuickSave") => SaveKind::Quick,
        _ if name.starts_with("AutoSave") => SaveKind::Auto,
        _ => SaveKind::Manual,
    };
    // `GameVersions` lists every version the campaign has been played with, oldest first;
    // the version that wrote this save is the last. `SaveInfo.json` names it directly.
    let game_version = if info.game_version.is_empty() {
        meta.find(&["MetaData", "MetaData", "GameVersions"])
            .and_then(|list| meta.children_named(list, "GameVersion").last())
            .map(|node| text(&meta, node, "Object"))
            .unwrap_or_default()
    } else {
        info.game_version.clone()
    };
    let (major, minor) = meta.header.engine_major_minor();
    let support = if (major, minor) == SUPPORTED_ENGINE {
        Support::Editable
    } else {
        Support::ReadOnly(format!(
            "Made with game version {game_version}. Only Patch {} saves can be edited: load \
             it in the current game and save again.",
            SUPPORTED_ENGINE.1
        ))
    };
    let mods = meta
        .find(&["MetaData", "MetaData", "ModuleSettings", "Mods"])
        .map(|list| {
            meta.children_named(list, "ModuleShortDesc")
                .filter(|&m| {
                    !crate::mods::BASE_MODULES.contains(&text(&meta, m, "Folder").as_str())
                })
                .map(|m| text(&meta, m, "Name"))
                .collect()
        })
        .unwrap_or_default();

    // `GameSessionID` stays the same for a whole playthrough. `GameID` does not: saves of
    // one playthrough made in different sittings carry different ones.
    let campaign = match text(&meta, root, "GameSessionID") {
        id if id.is_empty() => text(&meta, root, "GameID"),
        id => id,
    };

    Ok(SaveSummary {
        folder: folder.to_owned(),
        lsv,
        screenshot,
        name,
        campaign,
        leader: text(&meta, root, "LeaderName"),
        kind,
        saved_at: meta
            .get(root, "SaveTime")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as u64,
        game_version,
        support,
        level: text(&meta, root, "Level"),
        honour_mode: kind == SaveKind::Honour || info.is_honour_mode(),
        party: info.party,
        mods,
    })
}

/// Every save folder directly inside `dir`, newest first.
pub fn scan(dir: &Path) -> Result<Vec<Listed>> {
    let mut listed = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| dir.display().to_string())? {
        let folder = entry?.path();
        if !is_save_folder(&folder) {
            continue;
        }
        listed.push(match summarize(&folder) {
            Ok(summary) => Listed::Save(Box::new(summary)),
            Err(error) => Listed::Unreadable(Unreadable {
                name: display_name(&folder),
                error: format!("{error:#}"),
                folder,
            }),
        });
    }
    listed.sort_by(|a, b| {
        let key = |l: &Listed| match l {
            Listed::Save(s) => (0, std::cmp::Reverse(s.saved_at), s.name.clone()),
            Listed::Unreadable(u) => (1, std::cmp::Reverse(0), u.name.clone()),
        };
        key(a).cmp(&key(b))
    });
    Ok(listed)
}

/// One playthrough's saves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Campaign {
    pub id: String,
    /// The party leader's name in the newest save.
    pub leader: String,
    /// Newest first.
    pub saves: Vec<SaveSummary>,
}

impl Campaign {
    pub fn honour_mode(&self) -> bool {
        self.saves.first().is_some_and(|s| s.honour_mode)
    }
}

/// Groups saves into campaigns, the campaign played most recently first.
pub fn campaigns(listed: &[Listed]) -> Vec<Campaign> {
    let mut campaigns: Vec<Campaign> = Vec::new();
    // `listed` is newest first, so campaigns come out in order of their newest save, and
    // each campaign's saves newest first.
    for entry in listed {
        let Listed::Save(summary) = entry else {
            continue;
        };
        match campaigns.iter_mut().find(|c| c.id == summary.campaign) {
            Some(campaign) => campaign.saves.push((**summary).clone()),
            None => campaigns.push(Campaign {
                id: summary.campaign.clone(),
                leader: summary.leader.clone(),
                saves: vec![(**summary).clone()],
            }),
        }
    }
    campaigns
}

/// What a path someone dropped on the window or typed in refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dropped {
    /// A save: its folder.
    Save(PathBuf),
    /// A folder of saves.
    SavesFolder(PathBuf),
}

/// Works out what `path` is: a `.lsv`, a save folder, or a folder of save folders.
pub fn resolve(path: &Path) -> Option<Dropped> {
    if path.is_file() {
        let folder = path.parent()?;
        return (has_extension(path, "lsv") && is_save_folder(folder))
            .then(|| Dropped::Save(folder.to_owned()));
    }
    if is_save_folder(path) {
        return Some(Dropped::Save(path.to_owned()));
    }
    let holds_saves = std::fs::read_dir(path)
        .ok()?
        .flatten()
        .any(|e| is_save_folder(&e.path()));
    holds_saves.then(|| Dropped::SavesFolder(path.to_owned()))
}

const GAME_FOLDER: [&str; 2] = ["Larian Studios", "Baldur's Gate 3"];
const STEAM_APP_ID: &str = "1086940";

/// The `Savegames/Story` folder of every player profile under a `Baldur's Gate 3` data
/// folder, the default profile first.
fn profile_story_folders(game_data: &Path) -> Vec<PathBuf> {
    let profiles = game_data.join("PlayerProfiles");
    let mut names: Vec<String> = std::fs::read_dir(&profiles)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by_key(|n| (n != "Public", n.clone()));
    names
        .into_iter()
        .map(|n| profiles.join(n).join("Savegames").join("Story"))
        .collect()
}

/// Where the game keeps its data on this system, most likely first. `home` is the user's
/// home folder and `local_app_data` is Windows' `%LOCALAPPDATA%`.
fn game_data_folders(home: Option<&Path>, local_app_data: Option<&Path>) -> Vec<PathBuf> {
    let game = |base: PathBuf| base.join(GAME_FOLDER[0]).join(GAME_FOLDER[1]);
    let mut out = Vec::new();
    if let Some(local) = local_app_data {
        out.push(game(local.to_owned()));
    }
    if let Some(home) = home {
        // Windows, when %LOCALAPPDATA% is not set.
        out.push(game(home.join("AppData").join("Local")));
        // macOS.
        out.push(game(home.join("Documents")));
        // Linux and Steam Deck: the game runs under Proton, in Steam's Windows prefix.
        for steam in [
            ".local/share/Steam",
            ".steam/steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        ] {
            out.push(game(
                home.join(steam)
                    .join("steamapps/compatdata")
                    .join(STEAM_APP_ID)
                    .join("pfx/drive_c/users/steamuser/AppData/Local"),
            ));
        }
    }
    out
}

fn first_existing(home: Option<&Path>, local_app_data: Option<&Path>) -> Option<PathBuf> {
    game_data_folders(home, local_app_data)
        .iter()
        .flat_map(|data| profile_story_folders(data))
        .find(|story| story.is_dir())
}

/// The game's saves folder on this computer, if it can be found.
pub fn detect() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let home = var("HOME").or_else(|| var("USERPROFILE"));
    first_existing(home.as_deref(), var("LOCALAPPDATA").as_deref())
}

/// "3 minutes ago", for a time `seconds` in the past.
pub fn ago(seconds: u64) -> String {
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match seconds {
        0..=59 => "Just now".to_owned(),
        60..=3_599 => plural(seconds / 60, "minute"),
        3_600..=86_399 => plural(seconds / 3_600, "hour"),
        86_400..=172_799 => "Yesterday".to_owned(),
        172_800..=2_591_999 => plural(seconds / 86_400, "day"),
        2_592_000..=31_535_999 => plural(seconds / 2_592_000, "month"),
        _ => plural(seconds / 31_536_000, "year"),
    }
}

/// Builds save folders on disk. Used by tests, which have no real saves.
pub mod build {
    use super::*;
    use crate::lsf::build::{self as tree, Node};
    use crate::value::{DataType, Value};

    /// What to put in a test save.
    #[derive(Clone)]
    pub struct SaveSpec {
        pub prefix: String,
        pub name: String,
        pub campaign: String,
        pub leader: String,
        /// `SaveGameType`: 0 manual, 1 quick, 2 auto, 3 Honour Mode.
        pub save_type: u8,
        pub saved_at: u64,
        pub mods: Vec<String>,
        /// Extra files for the package, after `meta.lsf` and `SaveInfo.json`.
        pub files: Vec<(String, Vec<u8>)>,
        /// The `.WebP` to put beside the `.lsv`, if any.
        pub screenshot: Option<Vec<u8>>,
    }

    impl SaveSpec {
        pub fn new(leader: &str, name: &str) -> SaveSpec {
            SaveSpec {
                prefix: format!("{leader}-1234"),
                name: name.to_owned(),
                campaign: "04f94982-eff9-ed85-72ae-f2bfb78b94b3".to_owned(),
                leader: leader.to_owned(),
                save_type: 0,
                saved_at: 1_794_494_357,
                mods: Vec::new(),
                files: Vec::new(),
                screenshot: None,
            }
        }
    }

    /// The identifier of the `n`-th sample mod.
    pub fn mod_uuid(n: usize) -> crate::value::Uuid {
        let mut id = [0x5au8; 16];
        id[0] = n as u8 + 1;
        crate::value::Uuid(id)
    }

    /// A `meta.lsf` like the game's, with the fields the browser reads.
    pub fn meta(spec: &SaveSpec) -> Result<Vec<u8>> {
        let text = |v: &str| Value::Text(v.to_owned());
        let mut mods = Node::new("Mods").child(
            Node::new("ModuleShortDesc")
                .attr("Folder", DataType::LSString, text("GustavX"))
                .attr("Name", DataType::LSString, text("GustavX")),
        );
        for (n, name) in spec.mods.iter().enumerate() {
            mods = mods.child(
                Node::new("ModuleShortDesc")
                    .attr("UUID", DataType::Uuid, Value::Uuid(mod_uuid(n)))
                    .attr(
                        "Folder",
                        DataType::LSString,
                        text(&format!("{name}_folder")),
                    )
                    .attr("Name", DataType::LSString, text(name))
                    .attr(
                        "Version64",
                        DataType::Int64,
                        Value::Int64(36028797018963968),
                    ),
            );
        }
        let inner = Node::new("MetaData")
            .attr("Level", DataType::FixedString, text("WLD_Main_A"))
            .attr("GameSessionID", DataType::FixedString, text(&spec.campaign))
            // Different in every save, as it can be within one playthrough.
            .attr(
                "GameID",
                DataType::FixedString,
                text(&format!("game-id-of-{}", spec.name)),
            )
            .attr("LeaderName", DataType::LSString, text(&spec.leader))
            .attr("SaveGameType", DataType::Byte, Value::Byte(spec.save_type))
            .attr(
                "SaveTime",
                DataType::ULongLong,
                Value::ULongLong(spec.saved_at),
            )
            .child(Node::new("ModuleSettings").child(mods))
            .child(
                Node::new("GameVersions").child(Node::new("GameVersion").attr(
                    "Object",
                    DataType::LSString,
                    text("4.1.1.7631656"),
                )),
            );
        tree::lsf(&[Node::new("MetaData").child(inner)])
    }

    /// A `SaveInfo.json` with one level 5 character.
    pub fn save_info(spec: &SaveSpec) -> Vec<u8> {
        let difficulty = if spec.save_type == 3 {
            r#""Difficulty":["DifficultyHard","RulesetHonour"],"#
        } else {
            ""
        };
        format!(
            r#"{{"Active Party":{{"Characters":[{{"Classes":[{{"Main":"Sorcerer","Sub":"DraconicBloodline"}}],
            "Experience Points (Current level)":2345,"Experience Points (Total)":5595,"Level":5,
            "Origin":"Generic","Race":"Drow_LolthSworn"}}]}},"Current Level":"WLD_Main_A",{difficulty}
            "Game Version":"4.1.1.7631656","Platform":"Steam","Save Name":"{}"}}"#,
            spec.name
        )
        .into_bytes()
    }

    /// Writes the save folder for `spec` under `dir` and returns its path.
    pub fn save_folder(dir: &Path, spec: &SaveSpec) -> Result<PathBuf> {
        let meta = meta(spec)?;
        let info = save_info(spec);
        let mut files: Vec<(&str, &[u8])> = vec![(save::META, &meta), (save::SAVE_INFO, &info)];
        for (name, contents) in &spec.files {
            files.push((name, contents));
        }
        let folder = dir.join(format!("{}__{}", spec.prefix, spec.name));
        std::fs::create_dir_all(&folder)?;
        std::fs::write(
            folder.join(format!("{}.lsv", spec.name)),
            lspk::build(&files)?,
        )?;
        if let Some(image) = &spec.screenshot {
            std::fs::write(folder.join(format!("{}.WebP", spec.name)), image)?;
        }
        Ok(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::build::{save_folder, SaveSpec};
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nautiloid-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn summarizes_a_save() {
        let dir = temp_dir("summary");
        let mut spec = SaveSpec::new("Nelthryiis", "Shattered Sanctum - 8h 02m");
        spec.mods = vec!["Tav's Hair Salon".into(), "Faces of Faerun".into()];
        spec.screenshot = Some(b"picture".to_vec());
        let folder = save_folder(&dir, &spec).unwrap();

        let summary = summarize(&folder).unwrap();
        assert_eq!(summary.name, "Shattered Sanctum - 8h 02m");
        assert_eq!(summary.leader, "Nelthryiis");
        assert_eq!(summary.campaign, spec.campaign);
        assert_eq!(summary.kind, SaveKind::Manual);
        assert_eq!(summary.saved_at, spec.saved_at);
        assert_eq!(summary.game_version, "4.1.1.7631656");
        assert_eq!(summary.support, Support::Editable);
        assert_eq!(summary.level, "WLD_Main_A");
        assert_eq!(summary.party_level(), 5);
        assert!(!summary.honour_mode);
        // The game's own module is not a mod.
        assert_eq!(summary.mods, ["Tav's Hair Salon", "Faces of Faerun"]);
        assert_eq!(
            summary.screenshot.as_deref(),
            Some(folder.join("Shattered Sanctum - 8h 02m.WebP").as_path())
        );
        assert_eq!(summary.lsv, folder.join("Shattered Sanctum - 8h 02m.lsv"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scans_and_groups_by_campaign() {
        let dir = temp_dir("scan");
        let mut older = SaveSpec::new("Nelthryiis", "AutoSave_19");
        older.save_type = 2;
        older.saved_at = 1_000;
        let mut newer = SaveSpec::new("Nelthryiis", "Shattered Sanctum - 8h 02m");
        newer.saved_at = 3_000;
        let mut other = SaveSpec::new("Gale", "HonourMode");
        other.prefix = "4e21cb43-8d41-1e40-8a3b-9c743dc183e0".into();
        other.campaign = "8eee0a6a-21d0-7edf-a0fd-fe044b712240".into();
        other.save_type = 3;
        other.saved_at = 2_000;
        for spec in [&older, &newer, &other] {
            save_folder(&dir, spec).unwrap();
        }
        // A folder with a broken .lsv, a folder with none, and a stray file.
        let broken = dir.join("Tav-1__Broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(
            broken.join("Broken.lsv"),
            b"not a package, but long enough to be read",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("NotASave")).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();

        let listed = scan(&dir).unwrap();
        let names: Vec<String> = listed
            .iter()
            .map(|l| match l {
                Listed::Save(s) => s.name.clone(),
                Listed::Unreadable(u) => format!("unreadable {}", u.name),
            })
            .collect();
        assert_eq!(
            names,
            [
                "Shattered Sanctum - 8h 02m",
                "HonourMode",
                "AutoSave_19",
                "unreadable Broken"
            ]
        );
        let Listed::Unreadable(broken) = &listed[3] else {
            panic!("expected the broken save last");
        };
        assert!(broken.error.contains("LSPK"), "{}", broken.error);

        let campaigns = campaigns(&listed);
        assert_eq!(campaigns.len(), 2);
        assert_eq!(campaigns[0].leader, "Nelthryiis");
        let saves: Vec<&str> = campaigns[0].saves.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(saves, ["Shattered Sanctum - 8h 02m", "AutoSave_19"]);
        assert_eq!(campaigns[0].saves[1].kind, SaveKind::Auto);
        assert!(!campaigns[0].honour_mode());
        assert_eq!(campaigns[1].leader, "Gale");
        assert!(campaigns[1].honour_mode());
        assert_eq!(campaigns[1].saves[0].kind, SaveKind::Honour);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn works_out_what_a_path_is() {
        let dir = temp_dir("resolve");
        let folder = save_folder(&dir, &SaveSpec::new("Tav", "QuickSave_3")).unwrap();
        assert_eq!(resolve(&folder), Some(Dropped::Save(folder.clone())));
        assert_eq!(
            resolve(&folder.join("QuickSave_3.lsv")),
            Some(Dropped::Save(folder.clone()))
        );
        assert_eq!(resolve(&dir), Some(Dropped::SavesFolder(dir.clone())));
        let empty = dir.join("Empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(resolve(&empty), None);
        assert_eq!(resolve(&dir.join("missing")), None);
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert_eq!(resolve(&dir.join("notes.txt")), None);

        assert_eq!(display_name(&folder), "QuickSave_3");
        assert_eq!(display_name(Path::new("/x/NoSeparator")), "NoSeparator");
        // A quick save from before the type was recorded is still recognised by its name.
        assert_eq!(summarize(&folder).unwrap().kind, SaveKind::Quick);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_the_saves_folder() {
        let home = temp_dir("detect");
        assert_eq!(first_existing(Some(&home), None), None);

        // Steam on Linux, with a second profile beside the default one.
        let data = home
            .join(".local/share/Steam/steamapps/compatdata/1086940")
            .join("pfx/drive_c/users/steamuser/AppData/Local/Larian Studios/Baldur's Gate 3");
        for profile in ["Alt", "Public"] {
            std::fs::create_dir_all(
                data.join("PlayerProfiles")
                    .join(profile)
                    .join("Savegames/Story"),
            )
            .unwrap();
        }
        assert_eq!(
            first_existing(Some(&home), None),
            Some(data.join("PlayerProfiles/Public/Savegames/Story"))
        );

        // Windows' own folder wins when it exists.
        let local = home.join("AppData/Local");
        let windows =
            local.join("Larian Studios/Baldur's Gate 3/PlayerProfiles/Public/Savegames/Story");
        std::fs::create_dir_all(&windows).unwrap();
        assert_eq!(first_existing(Some(&home), Some(&local)), Some(windows));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn says_how_long_ago() {
        assert_eq!(ago(5), "Just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(3 * 3_600), "3 hours ago");
        assert_eq!(ago(100_000), "Yesterday");
        assert_eq!(ago(4 * 86_400), "4 days ago");
        assert_eq!(ago(70 * 86_400), "2 months ago");
        assert_eq!(ago(800 * 86_400), "2 years ago");
    }
}
