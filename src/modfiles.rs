//! The player's mods, read for the items they add: which are active and in what order (the
//! profile's `modsettings.lsx`), which package in the `Mods` folder holds each (its
//! `Mods/<folder>/meta.lsx`), and the files of each that an item catalog reads.

use crate::lspk;
use crate::xml::{self, Event};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where the game keeps a profile's mods: the `Mods` folder and the profile's mod list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModSetup {
    pub folder: PathBuf,
    pub settings: PathBuf,
}

impl ModSetup {
    /// For a saves folder where the game keeps them,
    /// `<game data>/PlayerProfiles/<profile>/Savegames/Story`: the `Mods` folder in the game
    /// data folder and the profile's `modsettings.lsx`, when both are there.
    pub fn for_saves(story: &Path) -> Option<ModSetup> {
        let profile = story.parent()?.parent()?;
        let settings = profile.join("modsettings.lsx");
        let folder = profile.parent()?.parent()?.join("Mods");
        (settings.is_file() && folder.is_dir()).then_some(ModSetup { folder, settings })
    }
}

/// A mod in the mod list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// In lower case.
    pub uuid: String,
    pub folder: String,
    pub name: String,
}

/// The mods a `modsettings.lsx` lists, in load order. The game's own modules are listed too.
pub fn order(settings: &str) -> Result<Vec<Listed>> {
    let mut out = Vec::new();
    let mut entry: Option<HashMap<String, String>> = None;
    for event in xml::read(settings)? {
        match &event {
            Event::Start { name, .. } if name == "node" => {
                entry = (event.attribute("id") == Some("ModuleShortDesc")).then(HashMap::new);
            }
            Event::Start { name, .. } if name == "attribute" => {
                if let (Some(entry), Some(id)) = (entry.as_mut(), event.attribute("id")) {
                    let value = event.attribute("value").unwrap_or_default();
                    entry.insert(id.to_owned(), value.to_owned());
                }
            }
            Event::End { name } if name == "node" => {
                if let Some(entry) = entry.take() {
                    let get = |k: &str| entry.get(k).cloned().unwrap_or_default();
                    out.push(Listed {
                        uuid: get("UUID").to_lowercase(),
                        folder: get("Folder"),
                        name: get("Name"),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A module's description, from its `meta.lsx`: identifier, folder and name.
fn describe(meta: &str) -> Option<Listed> {
    let events = xml::read(meta).ok()?;
    let mut in_info = false;
    let mut found: HashMap<String, String> = HashMap::new();
    for event in &events {
        match event {
            Event::Start { name, .. } if name == "node" => {
                in_info = event.attribute("id") == Some("ModuleInfo");
            }
            Event::Start { name, .. } if name == "attribute" && in_info => {
                if let Some(id) = event.attribute("id") {
                    found
                        .entry(id.to_owned())
                        .or_insert_with(|| event.attribute("value").unwrap_or_default().to_owned());
                }
            }
            _ => {}
        }
    }
    Some(Listed {
        uuid: found.get("UUID")?.to_lowercase(),
        folder: found.get("Folder").cloned().unwrap_or_default(),
        name: found.get("Name").cloned().unwrap_or_default(),
    })
}

/// What an item catalog reads from one mod, each file by its name in the package.
#[derive(Clone, Debug, Default)]
pub struct ModFiles {
    pub name: String,
    /// In lower case.
    pub uuid: String,
    pub folder: String,
    /// Root templates: `.lsf` documents, and `.lsx` ones where no `.lsf` of the same name is.
    pub templates: Vec<(String, Vec<u8>)>,
    /// Stats files.
    pub stats: Vec<(String, Vec<u8>)>,
    /// English text: `.xml` (`.loca.xml`) or `.loca`.
    pub texts: Vec<(String, Vec<u8>)>,
}

/// The active mods' files, in load order, and the active mods no package was found for.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub mods: Vec<ModFiles>,
    pub missing: Vec<String>,
}

/// Which of a package's files a catalog reads for the module in `folder`.
fn wanted(names: &[String], folder: &str) -> Vec<String> {
    let folder = folder.to_lowercase();
    let templates = format!("public/{folder}/roottemplates/");
    let stats = format!("public/{folder}/stats/generated/data/");
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let mut out = Vec::new();
    for (name, low) in names.iter().zip(&lower) {
        let template =
            low.starts_with(&templates) && (low.ends_with(".lsf") || low.ends_with(".lsx"));
        // An `.lsx` beside an `.lsf` of the same name is the same template written out.
        let duplicate =
            low.ends_with(".lsx") && lower.contains(&format!("{}.lsf", &low[..low.len() - 4]));
        let stat = low.starts_with(&stats) && low.ends_with(".txt");
        let text = (low.contains("/localization/english/")
            || low.starts_with("localization/english/"))
            && (low.ends_with(".xml") || low.ends_with(".loca"));
        if (template && !duplicate) || stat || text {
            out.push(name.clone());
        }
    }
    out
}

/// Reads the active mods' files, in the order the mod list gives.
pub fn read(setup: &ModSetup) -> Result<Loaded> {
    let settings = std::fs::read_to_string(&setup.settings)
        .with_context(|| setup.settings.display().to_string())?;
    let order = order(&settings).with_context(|| setup.settings.display().to_string())?;
    // Each module's package, by identifier, with the package's file names.
    let mut packages: HashMap<String, (PathBuf, Listed, Vec<String>)> = HashMap::new();
    let mut paks: Vec<PathBuf> = std::fs::read_dir(&setup.folder)
        .with_context(|| setup.folder.display().to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pak")))
        .collect();
    paks.sort();
    for pak in paks {
        let Ok(listing) = lspk::list(&pak) else {
            continue;
        };
        let names: Vec<String> = listing.files.into_iter().map(|f| f.name).collect();
        let metas: Vec<&str> = names
            .iter()
            .filter(|n| {
                let parts: Vec<&str> = n.split('/').collect();
                parts.len() == 3 && parts[0] == "Mods" && parts[2].eq_ignore_ascii_case("meta.lsx")
            })
            .map(String::as_str)
            .collect();
        let Ok(read) = lspk::peek(&pak, &metas) else {
            continue;
        };
        for bytes in read.into_iter().flatten() {
            if let Some(module) = describe(&String::from_utf8_lossy(&bytes)) {
                packages
                    .entry(module.uuid.clone())
                    .or_insert((pak.clone(), module, names.clone()));
            }
        }
    }
    let mut loaded = Loaded::default();
    for listed in order {
        let Some((pak, module, names)) = packages.get(&listed.uuid) else {
            // The game's own modules have no package here; anything else is missing.
            if !crate::mods::BASE_MODULES.contains(&listed.folder.as_str()) {
                loaded.missing.push(listed.name.clone());
            }
            continue;
        };
        let files = wanted(names, &module.folder);
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        let read = lspk::peek(pak, &refs).with_context(|| pak.display().to_string())?;
        let mut out = ModFiles {
            name: if module.name.is_empty() {
                listed.name.clone()
            } else {
                module.name.clone()
            },
            uuid: module.uuid.clone(),
            folder: module.folder.clone(),
            ..ModFiles::default()
        };
        for (name, bytes) in files.into_iter().zip(read) {
            let Some(bytes) = bytes else { continue };
            let low = name.to_lowercase();
            if low.contains("/roottemplates/") {
                out.templates.push((name, bytes));
            } else if low.ends_with(".txt") {
                out.stats.push((name, bytes));
            } else {
                out.texts.push((name, bytes));
            }
        }
        loaded.mods.push(out);
    }
    Ok(loaded)
}

/// Builds a mod's package, for tests.
pub mod build {
    /// A `meta.lsx` for a module.
    pub fn meta(uuid: &str, folder: &str, name: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<save>
    <version major="4" minor="7" revision="1" build="3"/>
    <region id="Config">
        <node id="root">
            <children>
                <node id="ModuleInfo">
                    <attribute id="Author" type="LSString" value="Someone"/>
                    <attribute id="Folder" type="LSString" value="{folder}"/>
                    <attribute id="Name" type="LSString" value="{name}"/>
                    <attribute id="UUID" type="FixedString" value="{uuid}"/>
                    <attribute id="Version64" type="int64" value="36028797018963968"/>
                </node>
            </children>
        </node>
    </region>
</save>"#
        )
    }

    /// A `modsettings.lsx` listing these modules, as identifier, folder and name.
    pub fn settings(modules: &[(&str, &str, &str)]) -> String {
        let mut entries = String::new();
        for (uuid, folder, name) in modules {
            entries.push_str(&format!(
                r#"
                        <node id="ModuleShortDesc">
                            <attribute id="Folder" type="LSString" value="{folder}"/>
                            <attribute id="Name" type="LSString" value="{name}"/>
                            <attribute id="UUID" type="guid" value="{uuid}"/>
                        </node>"#
            ));
        }
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<save>
    <version major="4" minor="8" revision="0" build="700"/>
    <region id="ModuleSettings">
        <node id="root">
            <children>
                <node id="Mods">
                    <children>{entries}
                    </children>
                </node>
            </children>
        </node>
    </region>
</save>"#
        )
    }

    /// English text in the XML form mods use.
    pub fn loca_xml(entries: &[(&str, &str)]) -> String {
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<contentList>\n");
        for (handle, text) in entries {
            out.push_str(&format!(
                "\t<content contentuid=\"{handle}\" version=\"1\">{}</content>\n",
                text.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            ));
        }
        out.push_str("</contentList>\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_mod_list_and_a_modules_description() {
        let list = order(&build::settings(&[
            ("cb555efe-2d9e-131f-8195-a89329d218ea", "GustavX", "GustavX"),
            (
                "C304469F-0000-0000-0000-000000000001",
                "AMP",
                "Ancient Mega Pack",
            ),
        ]))
        .unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].uuid, "c304469f-0000-0000-0000-000000000001");
        assert_eq!(list[1].name, "Ancient Mega Pack");
        let module = describe(&build::meta("AAAA-1", "AMP_Folder", "Pack")).unwrap();
        assert_eq!(
            (module.uuid.as_str(), module.folder.as_str()),
            ("aaaa-1", "AMP_Folder")
        );
    }

    const TEMPLATE_LSX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<save>
  <version major="4" minor="0" revision="9" build="331" />
  <region id="Templates">
    <node id="Templates">
      <children>
        <node id="GameObjects">
          <attribute id="MapKey" type="FixedString" value="c304469f-6132-4304-a5a2-882a2820be3b" />
          <attribute id="Name" type="LSString" value="AMP_FeyCoin" />
          <attribute id="Type" type="FixedString" value="item" />
          <attribute id="Stats" type="FixedString" value="AMP_FeyCoin" />
          <attribute id="DisplayName" type="TranslatedString" handle="hfey" version="1" />
          <attribute id="ParentTemplateId" type="FixedString" value="t-coin" />
        </node>
      </children>
    </node>
  </region>
</save>"#;

    #[test]
    fn reads_items_from_the_active_mods() {
        use crate::catalog::{build::game_folder, Catalog, Kind};
        let root = std::env::temp_dir().join(format!("nautiloid-modfiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // The game's folders: its Data, and a profile with saves and a mod list.
        let data = game_folder(&root.join("game"), &[("t-coin", "OBJ_Coin", "Gold Coin")]).unwrap();
        let larian = root.join("Baldur's Gate 3");
        let profile = larian.join("PlayerProfiles").join("Public");
        let story = profile.join("Savegames").join("Story");
        std::fs::create_dir_all(&story).unwrap();
        let mods = larian.join("Mods");
        std::fs::create_dir_all(&mods).unwrap();
        const AMP: &str = "c6c0d2bd-6198-de9e-30ad-e8cda1793025";
        std::fs::write(
            profile.join("modsettings.lsx"),
            build::settings(&[
                ("cb555efe-2d9e-131f-8195-a89329d218ea", "GustavX", "GustavX"),
                (&AMP.to_uppercase(), "AMP", "Ancient Mega Pack"),
                ("0000aaaa-0000-0000-0000-000000000000", "Gone", "Gone Mod"),
            ]),
        )
        .unwrap();
        let meta = build::meta(AMP, "AMP", "Ancient Mega Pack");
        let stats = "new entry \"AMP_FeyCoin\"\ntype \"Object\"\nusing \"OBJ_Coin\"\n\
                     data \"RootTemplate\" \"c304469f-6132-4304-a5a2-882a2820be3b\"\n\n\
                     new entry \"OBJ_Coin\"\ntype \"Object\"\nusing \"OBJ_Coin\"\n\
                     data \"Rarity\" \"Rare\"\n\n\
                     new entry \"Shout_Fey\"\ntype \"SpellData\"\n";
        let text = build::loca_xml(&[("hfey", "Fey Coin")]);
        let pak = crate::lspk::build(&[
            ("Mods/AMP/meta.lsx", meta.as_bytes()),
            ("Public/AMP/RootTemplates/fey.lsx", TEMPLATE_LSX.as_bytes()),
            (
                "Public/AMP/Stats/Generated/Data/Items.txt",
                stats.as_bytes(),
            ),
            (
                "Mods/AMP/Localization/English/items.loca.xml",
                text.as_bytes(),
            ),
        ])
        .unwrap();
        std::fs::write(mods.join("AMP.pak"), pak).unwrap();
        // Installed but not active.
        let idle = build::meta("1111bbbb-0000-0000-0000-000000000000", "Idle", "Idle Mod");
        let idle_pak = crate::lspk::build(&[
            ("Mods/Idle/meta.lsx", idle.as_bytes()),
            (
                "Public/Idle/Stats/Generated/Data/Items.txt",
                b"new entry \"OBJ_Coin\"\nusing \"OBJ_Coin\"\ndata \"Rarity\" \"Legendary\"\n",
            ),
        ])
        .unwrap();
        std::fs::write(mods.join("Idle.pak"), idle_pak).unwrap();

        let setup = ModSetup::for_saves(&story).unwrap();
        assert_eq!(setup.folder, mods);
        let loaded = read(&setup).unwrap();
        assert_eq!(loaded.missing, ["Gone Mod"]);
        assert_eq!(loaded.mods.len(), 1);
        assert_eq!(loaded.mods[0].uuid, AMP);

        let catalog = Catalog::from_game_and_mods(&data, &setup).unwrap();
        assert_eq!(catalog.mods, ["Ancient Mega Pack"]);
        assert_eq!(catalog.mod_problems, ["Gone Mod: not in the Mods folder"]);
        let fey = catalog.by_stats("AMP_FeyCoin").unwrap();
        assert_eq!(fey.display, "Fey Coin");
        assert_eq!(fey.kind, Kind::Object);
        assert_eq!(fey.module, "Ancient Mega Pack");
        assert_eq!(fey.module_id, AMP);
        // What the template leaves out comes from the game's own parent template.
        assert!(fey.pickable);
        assert!(catalog.search("fey").any(|i| i.stats == "AMP_FeyCoin"));
        // The mod adds to the game's entry; the inactive mod changes nothing.
        let coin = catalog.by_stats("OBJ_Coin").unwrap();
        assert_eq!(coin.rarity, "Rare");
        assert_eq!(coin.module, "");
        assert_eq!(coin.display, "Gold Coin");
        // Mods that can't be read leave the game's items.
        let broken = ModSetup {
            folder: root.join("nowhere"),
            settings: profile.join("modsettings.lsx"),
        };
        let catalog = Catalog::from_game_and_mods(&data, &broken).unwrap();
        assert!(catalog.by_stats("AMP_FeyCoin").is_none());
        assert!(catalog.mod_problems[0].starts_with("the mods couldn't be read"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn picks_the_files_a_catalog_reads() {
        let names: Vec<String> = [
            "Mods/AMP/meta.lsx",
            "Mods/AMP/Localization/English/items.loca.xml",
            "Mods/AMP/Localization/German/items.loca.xml",
            "Localization/English/other.loca",
            "Public/AMP/RootTemplates/one.lsf",
            "Public/AMP/RootTemplates/one.lsx",
            "Public/AMP/RootTemplates/two.lsx",
            "Public/AMP/RootTemplates/_merged.lsf",
            "Public/AMP/Stats/Generated/Data/Items.txt",
            "Public/AMP/Stats/Generated/TreasureTable.txt",
            "Public/Other/RootTemplates/three.lsf",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let picked = wanted(&names, "AMP");
        assert_eq!(
            picked,
            [
                "Mods/AMP/Localization/English/items.loca.xml",
                "Localization/English/other.loca",
                "Public/AMP/RootTemplates/one.lsf",
                "Public/AMP/RootTemplates/two.lsx",
                "Public/AMP/RootTemplates/_merged.lsf",
                "Public/AMP/Stats/Generated/Data/Items.txt",
            ]
        );
    }
}
