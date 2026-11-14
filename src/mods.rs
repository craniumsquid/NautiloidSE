//! The mods a save lists as required, which of them the game has active, and taking one off
//! the list.
//!
//! A save lists its mods in `meta.lsf`, and again in `Globals.lsf` and every level cache, as
//! `ModuleSettings/Mods/ModuleShortDesc` nodes. The game reads the list when a save is
//! loaded and warns about any mod that is missing. Taking a mod off the list takes it out of
//! every document; whatever the mod added to the world stays in the save.

use crate::lsf::Lsf;
use crate::value::Value;
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The game's own modules. Every save lists some of these; none can be taken off.
pub const BASE_MODULES: [&str; 14] = [
    "Gustav",
    "GustavDev",
    "GustavX",
    "Shared",
    "SharedDev",
    "Honour",
    "HonourX",
    "FW3",
    "Engine",
    "Game",
    "DiceSet_01",
    "DiceSet_02",
    "DiceSet_03",
    "ModBrowser",
];

/// One mod a save lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    /// The mod's identifier, in lower case, or empty if the save gives none.
    pub uuid: String,
    pub folder: String,
    pub name: String,
    /// `Version64`, packed as major, minor, revision and build.
    pub version: i64,
    /// Whether it is part of the game.
    pub base: bool,
}

impl Module {
    /// "1.2.0.3".
    pub fn version_text(&self) -> String {
        let v = self.version as u64;
        format!(
            "{}.{}.{}.{}",
            v >> 55,
            (v >> 47) & 0xff,
            (v >> 31) & 0xffff,
            v & 0x7fff_ffff
        )
    }

    /// What identifies the mod across documents: its identifier, or its folder when the save
    /// gives no identifier.
    pub fn key(&self) -> String {
        match self.uuid.is_empty() {
            true => self.folder.to_lowercase(),
            false => self.uuid.clone(),
        }
    }
}

fn text(lsf: &Lsf, node: u32, name: &str) -> String {
    match lsf.get(node, name) {
        Some(Value::Uuid(id)) => id.to_string(),
        Some(value) => value.as_str().unwrap_or_default().to_owned(),
        None => String::new(),
    }
}

fn module(lsf: &Lsf, node: u32) -> Module {
    let folder = text(lsf, node, "Folder");
    Module {
        uuid: text(lsf, node, "UUID").to_lowercase(),
        name: match text(lsf, node, "Name") {
            name if name.is_empty() => folder.clone(),
            name => name,
        },
        version: match lsf.get(node, "Version64") {
            Some(Value::Int64(v)) => v,
            Some(Value::ULongLong(v)) => v as i64,
            _ => 0,
        },
        base: BASE_MODULES.contains(&folder.as_str()),
        folder,
    }
}

/// Every `ModuleShortDesc` node in a document's mod lists.
fn module_nodes(lsf: &Lsf) -> Vec<u32> {
    (0..lsf.node_count() as u32)
        .filter(|&node| lsf.node_name(node) == "ModuleShortDesc")
        .filter(|&node| lsf.parent(node).is_some_and(|p| lsf.node_name(p) == "Mods"))
        .collect()
}

/// The mods `meta.lsf` lists, in its order.
pub fn read(meta: &Lsf) -> Vec<Module> {
    let Some(list) = meta.find(&["MetaData", "MetaData", "ModuleSettings", "Mods"]) else {
        return Vec::new();
    };
    meta.children_named(list, "ModuleShortDesc")
        .map(|node| module(meta, node))
        .collect()
}

/// Takes the mods whose [`Module::key`] is in `keys` off every list in the document, and
/// returns how many entries went.
pub fn remove(lsf: &mut Lsf, keys: &HashSet<String>) -> Result<usize> {
    let doomed: Vec<u32> = module_nodes(lsf)
        .into_iter()
        .filter(|&node| keys.contains(&module(lsf, node).key()))
        .collect();
    if !doomed.is_empty() {
        lsf.remove_nodes(&doomed)?;
    }
    Ok(doomed.len())
}

/// The mods the game has active, from the profile's `modsettings.lsx`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Active {
    /// Identifiers, in lower case.
    uuids: HashSet<String>,
    /// Folders, in lower case, for entries that give no identifier.
    folders: HashSet<String>,
    /// Where the list was read from.
    pub path: PathBuf,
}

impl Active {
    pub fn contains(&self, module: &Module) -> bool {
        (!module.uuid.is_empty() && self.uuids.contains(&module.uuid))
            || self.folders.contains(&module.folder.to_lowercase())
    }
}

/// The profile's `modsettings.lsx` for a save in the game's own folders:
/// `<profile>/Savegames/Story/<save>` has it at `<profile>/modsettings.lsx`.
pub fn settings_for(save_folder: &Path) -> Option<PathBuf> {
    let story = save_folder.parent()?;
    let savegames = story.parent()?;
    if !savegames
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("Savegames")
    {
        return None;
    }
    let path = savegames.parent()?.join("modsettings.lsx");
    path.is_file().then_some(path)
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The value of `<attribute id="{id}" ... value="..."/>` in a piece of `.lsx` text.
fn attribute(xml: &str, id: &str) -> Option<String> {
    let at = xml.find(&format!("id=\"{id}\""))?;
    let rest = &xml[at..];
    let end = rest.find("/>").unwrap_or(rest.len());
    let tag = &rest[..end];
    let start = tag.find("value=\"")? + "value=\"".len();
    let len = tag[start..].find('"')?;
    Some(unescape(&tag[start..start + len]))
}

/// Reads which mods the game has active from the text of a `modsettings.lsx`.
pub fn parse_settings(xml: &str, path: PathBuf) -> Active {
    let mut active = Active {
        path,
        ..Active::default()
    };
    // Each entry is a `ModuleShortDesc` node with no children.
    for entry in xml.split("<node id=\"ModuleShortDesc\"").skip(1) {
        let entry = entry.split("</node>").next().unwrap_or(entry);
        match attribute(entry, "UUID") {
            Some(uuid) if !uuid.is_empty() => {
                active.uuids.insert(uuid.to_lowercase());
            }
            _ => {
                if let Some(folder) = attribute(entry, "Folder") {
                    active.folders.insert(folder.to_lowercase());
                }
            }
        }
    }
    active
}

/// The mods the game has active, for a save in the game's own folders.
pub fn active_for(save_folder: &Path) -> Option<Active> {
    let path = settings_for(save_folder)?;
    let xml = std::fs::read_to_string(&path).ok()?;
    Some(parse_settings(&xml, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsf::build::{lsf, Node};
    use crate::value::{DataType, Uuid};

    fn entry(folder: &str, uuid: u8) -> Node {
        Node::new("ModuleShortDesc")
            .attr("UUID", DataType::Uuid, Value::Uuid(Uuid([uuid; 16])))
            .attr("Folder", DataType::LSString, Value::Text(folder.into()))
            .attr("Name", DataType::LSString, Value::Text(folder.into()))
            .attr(
                "Version64",
                DataType::Int64,
                Value::Int64(36028797018963968),
            )
    }

    fn meta() -> Lsf {
        let mods = Node::new("Mods")
            .child(entry("GustavX", 1))
            .child(entry("FaerunColors", 2))
            .child(entry("ImpUI", 3));
        let inner = Node::new("MetaData")
            .child(Node::new("ModuleSettings").child(mods))
            .child(Node::new("Other").child(Node::new("ModuleShortDesc")));
        Lsf::parse(&lsf(&[Node::new("MetaData").child(inner)]).unwrap()).unwrap()
    }

    #[test]
    fn reads_and_removes_mods() {
        let mut meta = meta();
        let mods = read(&meta);
        let folders: Vec<&str> = mods.iter().map(|m| m.folder.as_str()).collect();
        assert_eq!(folders, ["GustavX", "FaerunColors", "ImpUI"]);
        assert!(mods[0].base && !mods[1].base);
        assert_eq!(mods[1].version_text(), "1.0.0.0");
        assert_eq!(mods[1].key(), Uuid([2; 16]).to_string());

        let keys = HashSet::from([mods[1].key()]);
        assert_eq!(remove(&mut meta, &keys).unwrap(), 1);
        let meta = Lsf::parse(&meta.write().unwrap()).unwrap();
        let left: Vec<String> = read(&meta).into_iter().map(|m| m.folder).collect();
        assert_eq!(left, ["GustavX", "ImpUI"]);
        // A node of the same name outside a mod list is not touched.
        assert!(meta
            .find(&["MetaData", "MetaData", "Other", "ModuleShortDesc"])
            .is_some());
    }

    #[test]
    fn reads_the_games_active_mods() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<save>
  <version major="4" minor="8" revision="0" build="200"/>
  <region id="ModuleSettings">
    <node id="root">
      <children>
        <node id="Mods">
          <children>
            <node id="ModuleShortDesc">
              <attribute id="Folder" type="LSString" value="GustavX"/>
              <attribute id="UUID" type="guid" value="CB555EFE-2D9E-131F-8195-A89329D218EA"/>
            </node>
            <node id="ModuleShortDesc">
              <attribute id="Folder" type="LSString" value="Tav&apos;s Hair"/>
              <attribute id="Name" type="LSString" value="Tav's Hair"/>
            </node>
          </children>
        </node>
      </children>
    </node>
  </region>
</save>"#;
        let active = parse_settings(xml, PathBuf::from("modsettings.lsx"));
        let module = |uuid: &str, folder: &str| Module {
            uuid: uuid.into(),
            folder: folder.into(),
            name: folder.into(),
            version: 0,
            base: false,
        };
        assert!(active.contains(&module("cb555efe-2d9e-131f-8195-a89329d218ea", "GustavX")));
        assert!(active.contains(&module("", "Tav's Hair")));
        assert!(!active.contains(&module("00000000-0000-0000-0000-000000000000", "Other")));
    }

    #[test]
    fn finds_the_settings_beside_the_savegames_folder() {
        let root = std::env::temp_dir().join(format!("nautiloid-mods-{}", std::process::id()));
        let save = root.join("Public/Savegames/Story/Tav-1__Save");
        std::fs::create_dir_all(&save).unwrap();
        assert_eq!(settings_for(&save), None, "no file yet");
        std::fs::write(root.join("Public/modsettings.lsx"), "<save/>").unwrap();
        assert_eq!(
            settings_for(&save),
            Some(root.join("Public/modsettings.lsx"))
        );
        // A save somewhere else has none.
        assert_eq!(settings_for(&root.join("Public/Tav-1__Save")), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
