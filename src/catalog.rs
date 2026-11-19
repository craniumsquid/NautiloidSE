//! The game's items, with their English names, read from the player's own install.
//!
//! Nothing from the game is shipped with the editor. The catalog is built when it is needed
//! from files in the game's `Data` folder:
//!
//! - `Public/<module>/RootTemplates/_merged.lsf` in `Shared.pak`, `Gustav.pak` and
//!   `GustavX.pak`: every template, with `MapKey` (the identifier saves use as `TemplateID`),
//!   `Name`, `Type`, `Stats`, `DisplayName` and `Description` (handles into the language
//!   file), `Icon` and `ParentTemplateId`, whose values fill in any the template leaves out.
//! - `Public/<module>/Stats/Generated/Data/Object.txt`, `Armor.txt` and `Weapon.txt`: an
//!   entry per game name, `new entry "NAME"` then `type`, `using "PARENT"` (whose values fill
//!   in any it leaves out) and `data "Key" "Value"` lines, among them `RootTemplate`, `Rarity`
//!   and `ObjectCategory`.
//! - `Localization/English/english.loca` in `Localization/English.pak`: `LOCA`, the number
//!   of entries, where the texts start, then per entry a 64-byte handle, a u16 version and a
//!   u32 length, then the texts in the same order, each ending in a NUL.
//!
//! Modules are read in the game's order, and a later one's template or stats entry replaces
//! an earlier one's of the same identifier or name.

use crate::lsf::{Lsf, NodeId};
use crate::lspk;
use crate::modfiles::{ModFiles, ModSetup};
use anyhow::{anyhow, ensure, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The game's modules that hold items, in load order.
pub const MODULES: [&str; 7] = [
    "Shared",
    "SharedDev",
    "Gustav",
    "GustavDev",
    "GustavX",
    "Honour",
    "HonourX",
];
const STATS_FILES: [&str; 3] = ["Object", "Armor", "Weapon"];
const LANGUAGE: &str = "Localization/English/english.loca";

/// Every file the catalog reads, as packages name them.
pub fn wanted_files() -> Vec<String> {
    let mut out = Vec::new();
    for module in MODULES {
        out.push(format!("Public/{module}/RootTemplates/_merged.lsf"));
        for file in STATS_FILES {
            out.push(format!("Public/{module}/Stats/Generated/Data/{file}.txt"));
        }
    }
    out.push(LANGUAGE.to_owned());
    out
}

/// What kind of item it is, from its stats entry's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Weapon,
    Armor,
    Object,
    /// No stats entry.
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Weapon => "Weapon",
            Kind::Armor => "Armour",
            Kind::Object => "Object",
            Kind::Other => "Other",
        }
    }
}

/// One item template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogItem {
    /// The template's identifier, in lower case.
    pub template: String,
    /// The template's own name, for example `WPN_HUM_Flail_A_Pact`.
    pub name: String,
    /// The game name of its stats, for example `WPN_Flail_Pact`. Saves store this as `Stats`.
    pub stats: String,
    /// The name players see, or empty if the language file has none.
    pub display: String,
    pub description: String,
    pub icon: String,
    pub kind: Kind,
    pub rarity: String,
    /// `ObjectCategory` for objects, `Slot` for armour, `WeaponType`... whatever the stats
    /// entry says the item is, when it says.
    pub category: String,
    /// Whether the game marks it a story item: one its quests may count on.
    pub story: bool,
    /// The spell it casts when used, by the game's name for it, for a scroll:
    /// `Projectile_Fireball`. Empty for anything else.
    pub spell: String,
    /// How many can be in one stack: 1 for an item that doesn't stack.
    pub max_stack: u32,
    /// Whether it holds other items: a bag, a pouch, a chest.
    pub container: bool,
    /// Whether it can be picked up (`CanBePickedUp`, from its templates). Every item the
    /// sample saves have in an inventory can; one that can't shows as a blank slot.
    pub pickable: bool,
    /// The mod whose template it is, by name, or empty for the game's own.
    pub module: String,
    /// That mod's identifier, in lower case.
    pub module_id: String,
}

/// The game's item templates.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub items: Vec<CatalogItem>,
    by_template: HashMap<String, usize>,
    by_stats: HashMap<String, usize>,
    /// One of each item players can tell apart: the main template of each game name that
    /// has an English name, by name.
    listed: Vec<usize>,
    /// The mods read, by name, in load order.
    pub mods: Vec<String>,
    /// Active mods with no package in the `Mods` folder, and mod files that couldn't be read.
    pub mod_problems: Vec<String>,
}

/// A template as read, before its parent fills it in.
#[derive(Clone, Debug, Default)]
struct Template {
    name: String,
    kind: String,
    stats: String,
    display: String,
    description: String,
    icon: String,
    /// `StoryItem`, when the template says.
    story: Option<bool>,
    /// The spell its first use action casts.
    spell: String,
    /// `maxStackAmount`, when the template says.
    max_stack: Option<u32>,
    /// `InventoryType`, when the template says: 0 for an item with no inventory of its own,
    /// anything else for a container.
    inventory_type: Option<i64>,
    /// Whether the template has a bag's settings for what it takes in. Used only when no
    /// template in the line says `InventoryType`.
    bag: bool,
    /// `CanBePickedUp`, when the template says.
    pickable: Option<bool>,
    parent: String,
    /// The mod it comes from, by name and identifier, or empty.
    module: String,
    module_id: String,
}

/// A stats entry as read, before its `using` fills it in.
#[derive(Clone, Debug, Default)]
struct StatsEntry {
    file: Option<Kind>,
    using: String,
    data: HashMap<String, String>,
}

fn text_of(lsf: &Lsf, node: NodeId, name: &str) -> String {
    lsf.get(node, name)
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The spell a template's first use action casts: `SkillID` in
/// `OnUsePeaceActions/Action/Attributes`.
fn use_spell(lsf: &Lsf, template: NodeId) -> String {
    for actions in lsf.children_named(template, "OnUsePeaceActions") {
        for action in lsf.children_named(actions, "Action") {
            for attributes in lsf.children_named(action, "Attributes") {
                let spell = text_of(lsf, attributes, "SkillID");
                if !spell.is_empty() {
                    return spell;
                }
            }
        }
    }
    String::new()
}

/// Reads every template in a root template list.
fn read_templates(bytes: &[u8], into: &mut HashMap<String, Template>) -> Result<()> {
    read_template_list(&Lsf::parse(bytes)?, None, into);
    Ok(())
}

/// Reads every template in a root template list, as from `module`.
fn read_template_list(lsf: &Lsf, module: Option<&ModFiles>, into: &mut HashMap<String, Template>) {
    let Some(root) = lsf.root("Templates") else {
        return;
    };
    for node in lsf.children_named(root, "GameObjects") {
        let key = text_of(lsf, node, "MapKey").to_lowercase();
        if key.is_empty() {
            continue;
        }
        into.insert(
            key,
            Template {
                name: text_of(lsf, node, "Name"),
                kind: text_of(lsf, node, "Type"),
                stats: text_of(lsf, node, "Stats"),
                display: text_of(lsf, node, "DisplayName"),
                description: text_of(lsf, node, "Description"),
                icon: text_of(lsf, node, "Icon"),
                story: lsf
                    .get(node, "StoryItem")
                    .and_then(|v| v.as_i64())
                    .map(|v| v != 0),
                spell: use_spell(lsf, node),
                inventory_type: lsf.get(node, "InventoryType").and_then(|v| v.as_i64()),
                bag: lsf.get(node, "ContainerAutoAddOnPickup").is_some()
                    || lsf.get(node, "ContainerContentFilterCondition").is_some(),
                pickable: lsf
                    .get(node, "CanBePickedUp")
                    .and_then(|v| v.as_i64())
                    .map(|v| v != 0),
                max_stack: lsf
                    .get(node, "maxStackAmount")
                    .and_then(|v| v.as_i64())
                    .map(|v| v.clamp(1, u32::MAX as i64) as u32),
                parent: text_of(lsf, node, "ParentTemplateId").to_lowercase(),
                module: module.map(|m| m.name.clone()).unwrap_or_default(),
                module_id: module.map(|m| m.uuid.clone()).unwrap_or_default(),
            },
        );
    }
}

/// The quoted strings on a line of a stats file.
fn quoted(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

/// Reads every entry in a stats file. An entry's kind is the file's, `file`, unless its
/// `type` line says (mods put every kind in files with names of their own).
fn read_stats(text: &str, file: Option<Kind>, into: &mut HashMap<String, StatsEntry>) {
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("new entry ") {
            // An entry declared again is added to, not replaced: mods change one value of
            // the game's `ARM_Robe_Body` by declaring it again, `using` itself, with that
            // value alone.
            let name = quoted(rest).first().map(|s| s.to_string());
            if let Some(name) = &name {
                into.entry(name.clone()).or_insert(StatsEntry {
                    file,
                    ..StatsEntry::default()
                });
            }
            current = name;
            continue;
        }
        let Some(entry) = current.as_ref().and_then(|name| into.get_mut(name)) else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("type ") {
            entry.file = match quoted(rest).first().copied() {
                Some("Object") => Some(Kind::Object),
                Some("Armor") => Some(Kind::Armor),
                Some("Weapon") => Some(Kind::Weapon),
                Some(_) => None,
                None => entry.file,
            };
        } else if let Some(rest) = line.strip_prefix("using ") {
            match quoted(rest).first() {
                // Using itself: what it already is.
                Some(parent) if Some(*parent) == current.as_deref() => {}
                Some(parent) => entry.using = parent.to_string(),
                None => {}
            }
        } else if let Some(rest) = line.strip_prefix("data ") {
            if let [key, value, ..] = quoted(rest)[..] {
                entry.data.insert(key.to_owned(), value.to_owned());
            }
        }
    }
}

/// A stats entry's value for `key`, from it or the entries it uses.
fn stats_value<'a>(
    stats: &'a HashMap<String, StatsEntry>,
    name: &str,
    key: &str,
) -> Option<&'a str> {
    let mut current = stats.get(name)?;
    // Deeper than this would be a loop.
    for _ in 0..32 {
        if let Some(value) = current.data.get(key) {
            return Some(value);
        }
        current = stats.get(&current.using)?;
    }
    None
}

/// Whether the stats entry `name` is `base` or fills itself in from it, however far back.
fn stats_uses(stats: &HashMap<String, StatsEntry>, name: &str, base: &str) -> bool {
    let mut current = name;
    for _ in 0..32 {
        if current == base {
            return true;
        }
        match stats.get(current) {
            Some(entry) => current = &entry.using,
            None => return false,
        }
    }
    false
}

/// The stats entry that things made for throwing fill themselves in from.
const THROWN: &str = "_Thrown";

/// The texts for the handles in `wanted`, from a language file.
pub fn read_language(bytes: &[u8], wanted: &HashSet<String>) -> Result<HashMap<String, String>> {
    ensure!(
        bytes.len() >= 12 && &bytes[0..4] == b"LOCA",
        "not a language file (no LOCA signature)"
    );
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4]));
    let count = u32_at(4) as usize;
    let mut text_at = u32_at(8) as usize;
    ensure!(
        12usize
            .checked_add(count.saturating_mul(70))
            .is_some_and(|end| end <= text_at && text_at <= bytes.len()),
        "the language file's entries don't fit"
    );
    let mut out = HashMap::new();
    for i in 0..count {
        let entry = &bytes[12 + i * 70..12 + i * 70 + 70];
        let len = u32::from_le_bytes([entry[66], entry[67], entry[68], entry[69]]) as usize;
        let end = text_at
            .checked_add(len)
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| anyhow!("the language file's texts run past its end"))?;
        let key_len = entry[..64].iter().position(|&b| b == 0).unwrap_or(64);
        let key = std::str::from_utf8(&entry[..key_len]).unwrap_or_default();
        if wanted.contains(key) {
            let text = &bytes[text_at..end];
            let text = text.strip_suffix(&[0]).unwrap_or(text);
            out.insert(key.to_owned(), String::from_utf8_lossy(text).into_owned());
        }
        text_at = end;
    }
    Ok(out)
}

/// English text in the XML form mods carry: `<content contentuid="h…">text</content>`.
pub fn read_language_xml(text: &str) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    let mut handle: Option<String> = None;
    let mut body = String::new();
    for event in crate::xml::read(text)? {
        match event {
            crate::xml::Event::Start { ref name, .. } if name == "content" => {
                handle = event.attribute("contentuid").map(str::to_owned);
                body.clear();
            }
            crate::xml::Event::Text(text) if handle.is_some() => body.push_str(&text),
            crate::xml::Event::End { name } if name == "content" => {
                if let Some(handle) = handle.take() {
                    out.insert(handle, std::mem::take(&mut body));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

impl Catalog {
    /// Builds the catalog from the game's files, each fetched by the name packages give it.
    pub fn build(fetch: impl FnMut(&str) -> Result<Option<Vec<u8>>>) -> Result<Catalog> {
        Catalog::build_with(fetch, &[])
    }

    /// Builds the catalog from the game's files and then the mods', in load order: a mod's
    /// template, stats entry or text replaces the game's, or an earlier mod's, of the same
    /// identifier, name or handle. A mod file that can't be read is passed over and noted.
    pub fn build_with(
        mut fetch: impl FnMut(&str) -> Result<Option<Vec<u8>>>,
        mods: &[ModFiles],
    ) -> Result<Catalog> {
        let mut mod_problems = Vec::new();
        let mut templates = HashMap::new();
        let mut stats = HashMap::new();
        let mut found_templates = false;
        for module in MODULES {
            let name = format!("Public/{module}/RootTemplates/_merged.lsf");
            if let Some(bytes) = fetch(&name)? {
                read_templates(&bytes, &mut templates).with_context(|| name.clone())?;
                found_templates = true;
            }
            for (file, kind) in STATS_FILES
                .iter()
                .zip([Kind::Object, Kind::Armor, Kind::Weapon])
            {
                let name = format!("Public/{module}/Stats/Generated/Data/{file}.txt");
                if let Some(bytes) = fetch(&name)? {
                    read_stats(&String::from_utf8_lossy(&bytes), Some(kind), &mut stats);
                }
            }
        }
        ensure!(
            found_templates,
            "none of the game's item templates were found"
        );
        for module in mods {
            for (name, bytes) in &module.templates {
                let doc = match name.to_lowercase().ends_with(".lsx") {
                    true => crate::lsx::read(&String::from_utf8_lossy(bytes)),
                    false => Lsf::parse(bytes),
                };
                match doc {
                    Ok(doc) => read_template_list(&doc, Some(module), &mut templates),
                    Err(e) => mod_problems.push(format!("{}: {name}: {e:#}", module.name)),
                }
            }
            for (_, bytes) in &module.stats {
                read_stats(&String::from_utf8_lossy(bytes), None, &mut stats);
            }
        }

        // A template's missing values come from its parents.
        let inherited = |key: &str, field: fn(&Template) -> &str| -> String {
            let mut current = templates.get(key);
            for _ in 0..32 {
                let Some(template) = current else { break };
                let value = field(template);
                if !value.is_empty() {
                    return value.to_owned();
                }
                current = templates.get(&template.parent);
            }
            String::new()
        };
        // A value the template leaves out comes from its parents.
        fn first<T: Copy>(
            templates: &HashMap<String, Template>,
            key: &str,
            field: fn(&Template) -> Option<T>,
        ) -> Option<T> {
            let mut current = templates.get(key);
            for _ in 0..32 {
                let template = current?;
                if let Some(value) = field(template) {
                    return Some(value);
                }
                current = templates.get(&template.parent);
            }
            None
        }
        let story = |key: &str| first(&templates, key, |t| t.story).unwrap_or(false);
        let max_stack = |key: &str| first(&templates, key, |t| t.max_stack).unwrap_or(1);
        let pickable = |key: &str| first(&templates, key, |t| t.pickable).unwrap_or(false);
        // Whether it comes from the game's base template for containers.
        let from_container_base = |key: &str| {
            first(&templates, key, |t| {
                t.name.starts_with("BASE_CONT").then_some(true)
            })
            .unwrap_or(false)
        };
        // A container if the nearest `InventoryType` in its line says so. In the saves,
        // items have an inventory of their own when it does, give or take a few broken crates
        // and the Astral Prism: a Runepowder Barrel comes from `CONT_` barrels but sets it to
        // 0, and carried ones have no inventory. Failing that, if it has a bag's settings. A
        // `CONT_` name alone doesn't make one: throwable Spider Egg Sacs and Bags of Rotten
        // Food are named so, say nothing of `InventoryType`, and have no inventory.
        let container = |key: &str| match first(&templates, key, |t| t.inventory_type) {
            Some(kind) => kind != 0,
            None => first(&templates, key, |t| t.bag.then_some(true)).unwrap_or(false),
        };
        let mut items = Vec::new();
        for (key, template) in &templates {
            if template.kind != "item" {
                continue;
            }
            let stats_name = inherited(key, |t| &t.stats);
            let entry = stats.get(&stats_name);
            let value = |k: &str| {
                stats_value(&stats, &stats_name, k)
                    .unwrap_or_default()
                    .to_owned()
            };
            let category = ["ObjectCategory", "Slot", "WeaponType"]
                .iter()
                .map(|k| value(k))
                .find(|v| !v.is_empty())
                .unwrap_or_default();
            let kind = entry.and_then(|e| e.file).unwrap_or(Kind::Other);
            items.push(CatalogItem {
                template: key.clone(),
                name: template.name.clone(),
                stats: stats_name.clone(),
                display: inherited(key, |t| &t.display),
                description: inherited(key, |t| &t.description),
                icon: inherited(key, |t| &t.icon),
                kind,
                rarity: value("Rarity"),
                category,
                story: story(key),
                spell: inherited(key, |t| &t.spell),
                max_stack: max_stack(key),
                // A weapon can inherit a container's settings without being one. Something
                // that stacks is one only if it comes from the base template for containers,
                // as a Ribcage does, which bg3.wiki lists as a container. Wood Bark, an
                // ingredient, says it has an inventory, and the game gives it one more often
                // than not, but there's nothing to open, and stacks split from it have none.
                // Nor is anything made for throwing: a Firewine Barrel's template says it
                // has an inventory, but none of the 315 in the Patch 8 samples has one, and
                // one added without one works in the game.
                container: kind == Kind::Object
                    && container(key)
                    && (max_stack(key) <= 1 || from_container_base(key))
                    && !stats_uses(&stats, &stats_name, THROWN),
                pickable: pickable(key),
                module: template.module.clone(),
                module_id: template.module_id.clone(),
            });
        }

        // Names, for the handles the items use.
        let wanted: HashSet<String> = items
            .iter()
            .flat_map(|i| [i.display.clone(), i.description.clone()])
            .filter(|h| !h.is_empty())
            .collect();
        let mut texts = match fetch(LANGUAGE)? {
            Some(bytes) => read_language(&bytes, &wanted).context(LANGUAGE)?,
            None => HashMap::new(),
        };
        for module in mods {
            for (name, bytes) in &module.texts {
                let read = match name.to_lowercase().ends_with(".loca") {
                    true => read_language(bytes, &wanted),
                    false => read_language_xml(&String::from_utf8_lossy(bytes)),
                };
                match read {
                    Ok(read) => texts.extend(read.into_iter().filter(|(h, _)| wanted.contains(h))),
                    Err(e) => mod_problems.push(format!("{}: {name}: {e:#}", module.name)),
                }
            }
        }
        for item in &mut items {
            item.display = texts.get(&item.display).cloned().unwrap_or_default();
            item.description = texts.get(&item.description).cloned().unwrap_or_default();
        }
        items.sort_by(|a, b| {
            (&a.display, &a.name, &a.template).cmp(&(&b.display, &b.name, &b.template))
        });

        let by_template: HashMap<String, usize> = items
            .iter()
            .enumerate()
            .map(|(i, item)| (item.template.clone(), i))
            .collect();
        // For a game name, the template its stats entry names, else any that uses it; but
        // one that can be picked up before one that can't. A thrown Bag of Rotten Food is a
        // world object that can't be, and added to an inventory it shows as a blank slot.
        let mut by_stats: HashMap<String, usize> = HashMap::new();
        for (i, item) in items.iter().enumerate() {
            if !item.stats.is_empty() {
                let entry = by_stats.entry(item.stats.clone()).or_insert(i);
                if item.pickable && !items[*entry].pickable {
                    *entry = i;
                }
            }
        }
        for (name, entry) in &stats {
            let root = stats_value(&stats, name, "RootTemplate").map(str::to_lowercase);
            if let Some(&i) = root.and_then(|root| by_template.get(&root)) {
                let current = by_stats.get(name).map(|&c| items[c].pickable);
                // A root template the entry only has from the one it uses is that one's:
                // mods' Fey Coin uses the game's gold coin, and is its own template.
                let own = entry.data.contains_key("RootTemplate");
                if (own || current.is_none()) && (items[i].pickable || current != Some(true)) {
                    by_stats.insert(name.clone(), i);
                }
            }
        }
        let mut listed: Vec<usize> = by_stats
            .values()
            .copied()
            // Names that start "%%%" mark templates the game doesn't use.
            .filter(|&i| {
                let item = &items[i];
                !item.display.is_empty()
                    && !item.display.starts_with("%%%")
                    && item.kind != Kind::Other
                    && item.pickable
            })
            .collect::<HashSet<usize>>()
            .into_iter()
            .collect();
        listed.sort_by(|&a, &b| {
            (&items[a].display, &items[a].stats).cmp(&(&items[b].display, &items[b].stats))
        });
        Ok(Catalog {
            items,
            by_template,
            by_stats,
            listed,
            mods: mods.iter().map(|m| m.name.clone()).collect(),
            mod_problems,
        })
    }

    /// Builds the catalog from the game's `Data` folder and the player's active mods.
    /// Mods that can't be read at all leave the game's items, with the reason noted.
    pub fn from_game_and_mods(data: &Path, mods: &ModSetup) -> Result<Catalog> {
        let loaded = match crate::modfiles::read(mods) {
            Ok(loaded) => loaded,
            Err(e) => {
                let mut catalog = Catalog::from_game(data)?;
                catalog
                    .mod_problems
                    .push(format!("the mods couldn't be read: {e:#}"));
                return Ok(catalog);
            }
        };
        let mut catalog = Catalog::from_game_with(data, &loaded.mods)?;
        let missing = loaded
            .missing
            .into_iter()
            .map(|m| format!("{m}: not in the Mods folder"));
        catalog.mod_problems.splice(0..0, missing);
        Ok(catalog)
    }

    /// Builds the catalog from the packages in the game's `Data` folder, reading only the
    /// files it needs out of them.
    pub fn from_game(data: &Path) -> Result<Catalog> {
        Catalog::from_game_with(data, &[])
    }

    /// Builds the catalog from the game's `Data` folder and mods already read.
    pub fn from_game_with(data: &Path, mods: &[ModFiles]) -> Result<Catalog> {
        let wanted = wanted_files();
        let mut packages: Vec<PathBuf> = std::fs::read_dir(data)
            .with_context(|| data.display().to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pak")))
            .collect();
        packages.push(data.join("Localization").join("English.pak"));
        packages.sort();
        // Which package holds each file. A later package (a hotfix) wins.
        let mut holder: HashMap<String, PathBuf> = HashMap::new();
        for package in &packages {
            let Ok(listing) = lspk::list(package) else {
                continue;
            };
            for file in listing.files {
                if file.len > 0 && wanted.contains(&file.name) {
                    holder.insert(file.name, package.clone());
                }
            }
        }
        let mut by_package: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for (name, package) in &holder {
            by_package
                .entry(package.clone())
                .or_default()
                .push(name.clone());
        }
        let mut contents: HashMap<String, Vec<u8>> = HashMap::new();
        for (package, names) in by_package {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let read =
                lspk::peek(&package, &refs).with_context(|| package.display().to_string())?;
            for (name, bytes) in names.into_iter().zip(read) {
                if let Some(bytes) = bytes {
                    contents.insert(name, bytes);
                }
            }
        }
        Catalog::build_with(|name| Ok(contents.remove(name)), mods)
    }

    /// Builds the catalog from a folder holding the same files loose, at the paths the
    /// packages give them (as the command-line tool's `catalog-files` writes them).
    pub fn from_folder(folder: &Path) -> Result<Catalog> {
        Catalog::from_folder_with(folder, &[])
    }

    /// The same, with mods already read.
    pub fn from_folder_with(folder: &Path, mods: &[ModFiles]) -> Result<Catalog> {
        Catalog::build_with(
            |name| {
                let path = folder.join(name);
                match path.is_file() {
                    true => Ok(Some(std::fs::read(&path)?)),
                    false => Ok(None),
                }
            },
            mods,
        )
    }

    /// One of each item players can tell apart, by name: what to offer when adding items.
    pub fn listed(&self) -> impl Iterator<Item = &CatalogItem> {
        self.listed.iter().map(|&i| &self.items[i])
    }

    /// The listed items whose name or game name holds `text`, ignoring case.
    pub fn search<'a>(&'a self, text: &str) -> impl Iterator<Item = &'a CatalogItem> + 'a {
        let text = text.trim().to_lowercase();
        self.listed().filter(move |item| {
            item.display.to_lowercase().contains(&text) || item.stats.to_lowercase().contains(&text)
        })
    }

    /// The item made from template `template`.
    pub fn by_template(&self, template: &str) -> Option<&CatalogItem> {
        self.items
            .get(*self.by_template.get(&template.to_lowercase())?)
    }

    /// The item whose stats are called `stats`.
    pub fn by_stats(&self, stats: &str) -> Option<&CatalogItem> {
        self.items.get(*self.by_stats.get(stats)?)
    }

    /// The name players see for an item in a save, from its template or else its game name.
    pub fn display_name(&self, template: &str, stats: &str) -> Option<&str> {
        [self.by_template(template), self.by_stats(stats)]
            .into_iter()
            .flatten()
            .map(|item| item.display.as_str())
            .find(|name| !name.is_empty())
    }
}

/// Where the game's `Data` folder is, if it can be found: Steam's default library, then the
/// other libraries Steam lists.
pub fn detect_game() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut steam_roots = Vec::new();
    if let Some(x86) = var("ProgramFiles(x86)") {
        steam_roots.push(x86.join("Steam"));
    }
    if let Some(home) = var("HOME") {
        steam_roots.push(home.join(".local/share/Steam"));
        steam_roots.push(home.join(".steam/steam"));
        steam_roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        steam_roots.push(home.join("Library/Application Support/Steam"));
    }
    let mut libraries = Vec::new();
    for root in &steam_roots {
        libraries.push(root.clone());
        if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            libraries.extend(steam_library_paths(&text));
        }
    }
    libraries
        .into_iter()
        .map(|library| library.join("steamapps/common/Baldurs Gate 3/Data"))
        .find(|data| data.join("Shared.pak").is_file())
}

/// The library folders in Steam's `libraryfolders.vdf`: each `"path"` value.
pub fn steam_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let parts = quoted(line.trim());
            match parts[..] {
                ["path", path, ..] => Some(PathBuf::from(path.replace("\\\\", "\\"))),
                _ => None,
            }
        })
        .collect()
}

/// Builds a small game `Data` folder. Used by tests, which have no game install to read.
pub mod build {
    use super::*;
    use crate::lsf::build::{lsf, Node};
    use crate::value::{DataType, Value};

    /// A `GameObjects` node: identifier, name, then other fields. `DisplayName` and
    /// `Description` take a handle into the language file; the rest are text.
    pub fn template(key: &str, name: &str, fields: &[(&str, &str)]) -> Node {
        let mut node = Node::new("GameObjects")
            .attr("MapKey", DataType::FixedString, Value::Text(key.into()))
            .attr("Name", DataType::LSString, Value::Text(name.into()));
        for &(field, value) in fields {
            node = match field {
                "DisplayName" | "Description" => node.attr(
                    field,
                    DataType::TranslatedString,
                    Value::Translated {
                        version: 1,
                        handle: value.into(),
                    },
                ),
                "StoryItem" | "ContainerAutoAddOnPickup" | "CanBePickedUp" => {
                    node.attr(field, DataType::Bool, Value::Bool(value == "true"))
                }
                "maxStackAmount" => {
                    node.attr(field, DataType::Int, Value::Int(value.parse().unwrap_or(1)))
                }
                "InventoryType" => node.attr(
                    field,
                    DataType::Byte,
                    Value::Byte(value.parse().unwrap_or(0)),
                ),
                _ => node.attr(field, DataType::FixedString, Value::Text(value.into())),
            };
        }
        node
    }

    /// A language file with these handles and texts.
    pub fn language(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut head = b"LOCA".to_vec();
        head.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        head.extend_from_slice(&((12 + 70 * entries.len()) as u32).to_le_bytes());
        let mut texts = Vec::new();
        for (key, text) in entries {
            let mut k = [0u8; 64];
            k[..key.len()].copy_from_slice(key.as_bytes());
            head.extend_from_slice(&k);
            head.extend_from_slice(&1u16.to_le_bytes());
            head.extend_from_slice(&((text.len() + 1) as u32).to_le_bytes());
            texts.extend_from_slice(text.as_bytes());
            texts.push(0);
        }
        head.extend(texts);
        head
    }

    /// An item for [`game_folder_with`].
    pub struct GameItem<'a> {
        pub template: &'a str,
        pub stats: &'a str,
        pub display: &'a str,
        pub kind: Kind,
        /// `ObjectCategory` for an object, `Slot` for armour.
        pub category: &'a str,
        /// How many fit in one stack: 1 for an item that doesn't stack.
        pub max_stack: u32,
    }

    /// Writes `<dir>/Data` with `Shared.pak` and `Localization/English.pak` holding these
    /// items, each given as template identifier, game name and English name, all objects
    /// of no category, and returns the `Data` folder.
    pub fn game_folder(dir: &Path, items: &[(&str, &str, &str)]) -> Result<PathBuf> {
        let items: Vec<GameItem> = items
            .iter()
            .map(|&(template, stats, display)| GameItem {
                template,
                stats,
                display,
                kind: Kind::Object,
                category: "",
                max_stack: 1,
            })
            .collect();
        game_folder_with(dir, &items)
    }

    /// Writes `<dir>/Data` holding these items, and returns the `Data` folder.
    pub fn game_folder_with(dir: &Path, items: &[GameItem]) -> Result<PathBuf> {
        let data = dir.join("Data");
        std::fs::create_dir_all(data.join("Localization"))?;
        let mut templates = Node::new("Templates");
        let mut stats_files: HashMap<&str, String> = HashMap::new();
        let mut texts = Vec::new();
        let handles: Vec<String> = (0..items.len()).map(|i| format!("hname{i}")).collect();
        for (item, handle) in items.iter().zip(&handles) {
            let stack = item.max_stack.to_string();
            let mut fields = vec![
                ("Type", "item"),
                ("Stats", item.stats),
                ("DisplayName", handle.as_str()),
                ("CanBePickedUp", "true"),
            ];
            if item.max_stack > 1 {
                fields.push(("maxStackAmount", &stack));
            }
            if item.category == "Container" {
                fields.push(("ContainerAutoAddOnPickup", "true"));
            }
            templates = templates.child(template(item.template, item.stats, &fields));
            let (file, category_key) = match item.kind {
                Kind::Weapon => ("Weapon", ""),
                Kind::Armor => ("Armor", "Slot"),
                _ => ("Object", "ObjectCategory"),
            };
            let entry = stats_files.entry(file).or_default();
            entry.push_str(&format!(
                "new entry \"{}\"\ntype \"{file}\"\ndata \"RootTemplate\" \"{}\"\n",
                item.stats, item.template
            ));
            if !item.category.is_empty() && !category_key.is_empty() {
                entry.push_str(&format!("data \"{category_key}\" \"{}\"\n", item.category));
            }
            entry.push('\n');
            texts.push((handle.as_str(), item.display));
        }
        let templates = lsf(&[templates])?;
        let names: Vec<String> = stats_files
            .keys()
            .map(|file| format!("Public/Shared/Stats/Generated/Data/{file}.txt"))
            .collect();
        let mut files: Vec<(&str, &[u8])> =
            vec![("Public/Shared/RootTemplates/_merged.lsf", &templates)];
        for (name, file) in names.iter().zip(stats_files.keys()) {
            files.push((name, stats_files[file].as_bytes()));
        }
        std::fs::write(data.join("Shared.pak"), lspk::build(&files)?)?;
        let english = lspk::build(&[(LANGUAGE, &language(&texts))])?;
        std::fs::write(data.join("Localization").join("English.pak"), english)?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::build::{language, template};
    use super::*;
    use crate::lsf::build::{lsf, Node};

    fn files() -> HashMap<String, Vec<u8>> {
        let shared = lsf(&[Node::new("Templates")
            .child(template(
                "AAAA0000-0000-0000-0000-000000000001",
                "BASE_Potion",
                &[
                    ("Type", "item"),
                    ("Icon", "Item_Potion"),
                    ("DisplayName", "h1"),
                ],
            ))
            .child(template(
                "aaaa0000-0000-0000-0000-000000000002",
                "OBJ_Potion_Healing_A",
                &[
                    ("Type", "item"),
                    ("Stats", "OBJ_Potion_Healing"),
                    ("ParentTemplateId", "aaaa0000-0000-0000-0000-000000000001"),
                    ("DisplayName", "h2"),
                    ("Description", "h3"),
                ],
            ))
            .child(template(
                "aaaa0000-0000-0000-0000-000000000003",
                "S_Some_Character",
                &[("Type", "character"), ("DisplayName", "h4")],
            ))])
        .unwrap();
        // A later module replaces a template and adds one.
        let gustav = lsf(&[Node::new("Templates")
            .child(template(
                "aaaa0000-0000-0000-0000-000000000001",
                "BASE_Potion",
                &[
                    ("Type", "item"),
                    ("CanBePickedUp", "true"),
                    ("Icon", "Item_Potion_New"),
                    ("StoryItem", "true"),
                    ("maxStackAmount", "100"),
                ],
            ))
            .child(template(
                "aaaa0000-0000-0000-0000-000000000004",
                "WPN_Dagger_A",
                &[
                    ("Type", "item"),
                    ("CanBePickedUp", "true"),
                    ("Stats", "WPN_Dagger"),
                    ("DisplayName", "h5"),
                ],
            ))])
        .unwrap();
        let objects = "new entry \"_Potion\"\ntype \"Object\"\ndata \"Rarity\" \"Common\"\n\
                       data \"ObjectCategory\" \"Potion\"\n\n\
                       new entry \"OBJ_Potion_Healing\"\ntype \"Object\"\nusing \"_Potion\"\n\
                       data \"RootTemplate\" \"aaaa0000-0000-0000-0000-000000000002\"\n";
        let weapons = "new entry \"WPN_Dagger\"\ntype \"Weapon\"\ndata \"Rarity\" \"Uncommon\"\n";
        HashMap::from([
            ("Public/Shared/RootTemplates/_merged.lsf".to_owned(), shared),
            ("Public/Gustav/RootTemplates/_merged.lsf".to_owned(), gustav),
            (
                "Public/Shared/Stats/Generated/Data/Object.txt".to_owned(),
                objects.as_bytes().to_vec(),
            ),
            (
                "Public/Gustav/Stats/Generated/Data/Weapon.txt".to_owned(),
                weapons.as_bytes().to_vec(),
            ),
            (
                LANGUAGE.to_owned(),
                language(&[
                    ("h1", "Potion"),
                    ("h2", "Potion of Healing"),
                    ("h3", "Drink it."),
                    ("h4", "Someone"),
                    ("h5", "Dagger"),
                ]),
            ),
        ])
    }

    #[test]
    fn builds_items_with_their_names() {
        let mut files = files();
        let catalog = Catalog::build(|name| Ok(files.remove(name))).unwrap();
        // Items only, by English name.
        let names: Vec<&str> = catalog.items.iter().map(|i| i.display.as_str()).collect();
        assert_eq!(names, ["", "Dagger", "Potion of Healing"]);

        let potion = catalog.by_stats("OBJ_Potion_Healing").unwrap();
        assert_eq!(potion.template, "aaaa0000-0000-0000-0000-000000000002");
        assert_eq!(potion.description, "Drink it.");
        // From its parent, as the later module has the parent.
        assert_eq!(potion.icon, "Item_Potion_New");
        assert_eq!(
            (potion.kind, potion.rarity.as_str()),
            (Kind::Object, "Common")
        );
        assert_eq!(potion.category, "Potion");
        assert!(potion.story, "a story item, as its parent is");
        assert_eq!(potion.max_stack, 100, "as its parent says");

        let dagger = catalog
            .by_template("AAAA0000-0000-0000-0000-000000000004")
            .unwrap();
        assert_eq!(
            (dagger.kind, dagger.rarity.as_str()),
            (Kind::Weapon, "Uncommon")
        );
        assert!(!dagger.story);
        assert_eq!(dagger.max_stack, 1, "nothing says it stacks");
        // The replaced parent has no name of its own any more.
        assert_eq!(
            catalog
                .by_template("aaaa0000-0000-0000-0000-000000000001")
                .unwrap()
                .display,
            ""
        );
        assert_eq!(
            catalog.display_name("unknown", "OBJ_Potion_Healing"),
            Some("Potion of Healing")
        );
        assert_eq!(catalog.display_name("unknown", "unknown"), None);
        // One of each named item, by name, for adding.
        let listed: Vec<&str> = catalog.listed().map(|i| i.display.as_str()).collect();
        assert_eq!(listed, ["Dagger", "Potion of Healing"]);
        let found: Vec<&str> = catalog.search("POTION").map(|i| i.stats.as_str()).collect();
        assert_eq!(found, ["OBJ_Potion_Healing"]);
    }

    #[test]
    fn containers_follow_their_inventory_type() {
        let key = |n: u32| format!("cccc0000-0000-0000-0000-{n:012}");
        let keys: Vec<String> = (1..=11).map(key).collect();
        let item = |k: usize,
                    name: &'static str,
                    parent: Option<usize>,
                    extra: &[(&'static str, &'static str)]| {
            let mut fields: Vec<(&str, &str)> = vec![("Type", "item"), ("Stats", name)];
            if let Some(parent) = parent {
                fields.push(("ParentTemplateId", keys[parent].as_str()));
            }
            fields.extend_from_slice(extra);
            template(&keys[k], name, &fields)
        };
        let shared = lsf(&[Node::new("Templates")
            // A barrel that holds things, as the game's are.
            .child(item(0, "CONT_Barrel_A", None, &[("InventoryType", "1")]))
            // A barrel of smokepowder made from it says it holds nothing, and so do the
            // barrels made from that one in turn.
            .child(item(
                1,
                "CONT_Barrel_Smokepowder_A",
                Some(0),
                &[("InventoryType", "0")],
            ))
            .child(item(2, "UNI_Runepowder_Barrel", Some(1), &[]))
            // A barrel that says nothing holds what its parent says.
            .child(item(3, "CONT_Barrel_Firewine_A", Some(0), &[]))
            // With no InventoryType anywhere, a bag's settings decide, not the name.
            .child(item(4, "CONT_Egg_Sac_CombatThrowable", None, &[]))
            .child(item(
                5,
                "Pouch",
                None,
                &[("ContainerAutoAddOnPickup", "true")],
            ))
            .child(item(6, "Potion", None, &[]))
            // An ingredient that says it has an inventory, but stacks.
            .child(item(
                7,
                "Wood_Bark",
                None,
                &[("InventoryType", "9"), ("maxStackAmount", "99")],
            ))
            // A ribcage stacks too, but comes from the base template for containers.
            .child(item(8, "BASE_CONT", None, &[("InventoryType", "4")]))
            .child(item(
                9,
                "Ribcage",
                Some(8),
                &[("InventoryType", "7"), ("maxStackAmount", "99")],
            ))
            // A barrel of firewine says it has an inventory, but is made for throwing.
            .child(item(
                10,
                "CONT_Barrel_Alcohol_A",
                Some(0),
                &[("InventoryType", "1")],
            ))])
        .unwrap();
        let objects: String = [
            "CONT_Barrel_A",
            "CONT_Barrel_Smokepowder_A",
            "UNI_Runepowder_Barrel",
            "CONT_Barrel_Firewine_A",
            "CONT_Egg_Sac_CombatThrowable",
            "Pouch",
            "Potion",
            "Wood_Bark",
            "BASE_CONT",
            "Ribcage",
        ]
        .iter()
        .map(|name| format!("new entry \"{name}\"\ntype \"Object\"\n\n"))
        .chain([
            "new entry \"_Thrown\"\ntype \"Object\"\n\n".to_owned(),
            "new entry \"OBJ_Barrel_Filled\"\ntype \"Object\"\nusing \"_Thrown\"\n\n".to_owned(),
            "new entry \"CONT_Barrel_Alcohol_A\"\ntype \"Object\"\n\
             using \"OBJ_Barrel_Filled\"\n\n"
                .to_owned(),
        ])
        .collect();
        let mut files = HashMap::from([
            ("Public/Shared/RootTemplates/_merged.lsf".to_owned(), shared),
            (
                "Public/Shared/Stats/Generated/Data/Object.txt".to_owned(),
                objects.into_bytes(),
            ),
        ]);
        let catalog = Catalog::build(|name| Ok(files.remove(name))).unwrap();
        let container = |stats: &str| catalog.by_stats(stats).unwrap().container;
        assert!(container("CONT_Barrel_A"));
        assert!(!container("CONT_Barrel_Smokepowder_A"));
        assert!(
            !container("UNI_Runepowder_Barrel"),
            "an explosive, not a container"
        );
        assert!(container("CONT_Barrel_Firewine_A"));
        assert!(!container("CONT_Egg_Sac_CombatThrowable"), "a throwable");
        assert!(container("Pouch"));
        assert!(!container("Potion"));
        assert!(!container("Wood_Bark"), "it stacks");
        assert!(container("Ribcage"), "it stacks, but comes from BASE_CONT");
        assert!(!container("CONT_Barrel_Alcohol_A"), "made for throwing");
    }

    #[test]
    fn offers_only_items_that_can_be_picked_up() {
        let thing = |key: &str, name: &str, stats: &str, handle: &str, pickable: Option<&str>| {
            let mut fields = vec![("Type", "item"), ("Stats", stats), ("DisplayName", handle)];
            if let Some(pickable) = pickable {
                fields.push(("CanBePickedUp", pickable));
            }
            template(key, name, &fields)
        };
        let shared = lsf(&[Node::new("Templates")
            // A bag of rotten food to throw in a fight, which nobody can pick up, is the
            // template the game name's stats entry names; one that can be carried uses the
            // same game name.
            .child(thing(
                "dddd0000-0000-0000-0000-000000000001",
                "Bag_Thrown",
                "OBJ_Bag_Rotten",
                "h1",
                None,
            ))
            .child(thing(
                "dddd0000-0000-0000-0000-000000000002",
                "Bag_Carried",
                "OBJ_Bag_Rotten",
                "h1",
                Some("true"),
            ))
            // A statue nobody can pick up has no other template.
            .child(thing(
                "dddd0000-0000-0000-0000-000000000003",
                "Statue",
                "OBJ_Statue",
                "h2",
                Some("false"),
            ))])
        .unwrap();
        let objects = "new entry \"OBJ_Bag_Rotten\"\ntype \"Object\"\n\
                       data \"RootTemplate\" \"dddd0000-0000-0000-0000-000000000001\"\n\n\
                       new entry \"OBJ_Statue\"\ntype \"Object\"\n";
        let mut files = HashMap::from([
            ("Public/Shared/RootTemplates/_merged.lsf".to_owned(), shared),
            (
                "Public/Shared/Stats/Generated/Data/Object.txt".to_owned(),
                objects.as_bytes().to_vec(),
            ),
            (
                LANGUAGE.to_owned(),
                language(&[("h1", "Bag of Rotten Food"), ("h2", "Statue")]),
            ),
        ]);
        let catalog = Catalog::build(|name| Ok(files.remove(name))).unwrap();
        let bag = catalog.by_stats("OBJ_Bag_Rotten").unwrap();
        assert_eq!(bag.name, "Bag_Carried", "the one that can be carried");
        assert!(bag.pickable);
        assert!(!catalog.by_stats("OBJ_Statue").unwrap().pickable);
        let listed: Vec<&str> = catalog.listed().map(|i| i.name.as_str()).collect();
        assert_eq!(listed, ["Bag_Carried"]);
    }

    #[test]
    fn reads_from_packages_in_a_data_folder() {
        let dir = std::env::temp_dir().join(format!("nautiloid-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Localization")).unwrap();
        let files = files();
        let in_package = |names: &[&str]| {
            let list: Vec<(&str, &[u8])> =
                names.iter().map(|n| (*n, files[*n].as_slice())).collect();
            lspk::build(&list).unwrap()
        };
        std::fs::write(
            dir.join("Shared.pak"),
            in_package(&[
                "Public/Shared/RootTemplates/_merged.lsf",
                "Public/Shared/Stats/Generated/Data/Object.txt",
            ]),
        )
        .unwrap();
        std::fs::write(
            dir.join("Gustav.pak"),
            in_package(&[
                "Public/Gustav/RootTemplates/_merged.lsf",
                "Public/Gustav/Stats/Generated/Data/Weapon.txt",
            ]),
        )
        .unwrap();
        std::fs::write(
            dir.join("Localization/English.pak"),
            in_package(&[LANGUAGE]),
        )
        .unwrap();
        // Something that isn't a package is passed over.
        std::fs::write(dir.join("Textures_1.pak"), b"no header").unwrap();
        let catalog = Catalog::from_game(&dir).unwrap();
        assert_eq!(catalog.items.len(), 3);
        assert_eq!(
            catalog.display_name("aaaa0000-0000-0000-0000-000000000004", ""),
            Some("Dagger")
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(Catalog::from_game(&dir).is_err());
    }

    #[test]
    fn builds_a_game_folder_for_tests() {
        let dir = std::env::temp_dir().join(format!("nautiloid-game-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = build::game_folder(
            &dir,
            &[(
                "00000000-0000-0000-0000-00000000dddd",
                "OBJ_Potion_Healing",
                "Potion of Healing",
            )],
        )
        .unwrap();
        let catalog = Catalog::from_game(&data).unwrap();
        assert_eq!(
            catalog.display_name("00000000-0000-0000-0000-00000000DDDD", ""),
            Some("Potion of Healing")
        );
        assert_eq!(
            catalog.display_name("", "OBJ_Potion_Healing"),
            Some("Potion of Healing")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_steam_libraries() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            steam_library_paths(vdf),
            [
                PathBuf::from("C:\\Program Files (x86)\\Steam"),
                PathBuf::from("D:\\SteamLibrary")
            ]
        );
    }
}
