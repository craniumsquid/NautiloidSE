//! Checks against the game's own files. They are the game's, so they are not in the
//! repository: set `NAUTILOID_GAME_FILES` to a folder the command-line tool's
//! `catalog-files` wrote, or to the game's `Data` folder. Without it these pass without
//! checking anything. With `NAUTILOID_SAVES` too, the items in those saves are named.

use nautiloid_save_editor::catalog::{Catalog, Kind};
use nautiloid_save_editor::newage::NewAge;
use nautiloid_save_editor::party;
use nautiloid_save_editor::save::{self, Save, Support};
use std::path::PathBuf;

fn catalog() -> Option<Catalog> {
    let folder = PathBuf::from(std::env::var_os("NAUTILOID_GAME_FILES")?);
    Some(match folder.join("Shared.pak").is_file() {
        true => Catalog::from_game(&folder).unwrap(),
        false => Catalog::from_folder(&folder).unwrap(),
    })
}

#[test]
fn the_games_items_have_names_and_kinds() {
    let Some(catalog) = catalog() else { return };
    assert!(catalog.items.len() > 5000, "{} items", catalog.items.len());
    let named = catalog
        .items
        .iter()
        .filter(|i| !i.display.is_empty())
        .count();
    assert!(named * 10 > catalog.items.len() * 9, "{named} named");
    for kind in [Kind::Weapon, Kind::Armor, Kind::Object] {
        assert!(catalog.items.iter().any(|i| i.kind == kind), "no {kind:?}");
    }
    let potion = catalog
        .by_stats("OBJ_Potion_Healing")
        .expect("healing potions");
    assert_eq!(potion.display, "Potion of Healing");
}

#[test]
fn most_items_in_the_saves_are_named() {
    let Some(catalog) = catalog() else { return };
    let Some(saves) = std::env::var_os("NAUTILOID_SAVES") else {
        return;
    };
    for entry in std::fs::read_dir(saves).unwrap().flatten() {
        let Ok(save) = Save::open(&entry.path()) else {
            continue;
        };
        if save.support() != Support::Editable {
            continue;
        }
        let globals = save.lsf(save::GLOBALS).unwrap();
        let party = party::read(&globals, &NewAge::from_lsf(&globals).unwrap()).unwrap();
        let items: Vec<&party::Item> = party.inventories.iter().flat_map(|i| &i.items).collect();
        // Items from mods are named in the mods' own files, which these are not, so only
        // items the game's files know are counted.
        let known: Vec<&&party::Item> = items
            .iter()
            .filter(|i| {
                catalog.by_template(&i.template).is_some() || catalog.by_stats(&i.stats).is_some()
            })
            .collect();
        let named = known
            .iter()
            .filter(|i| catalog.display_name(&i.template, &i.stats).is_some())
            .count();
        assert!(
            named * 100 >= known.len() * 95,
            "{}: {named} of {} named",
            entry.path().display(),
            known.len()
        );
    }
}

#[test]
fn items_can_be_added_to_the_saves() {
    use nautiloid_save_editor::session::Session;
    use std::sync::Arc;
    let Some(catalog) = catalog() else { return };
    let Some(saves) = std::env::var_os("NAUTILOID_SAVES") else {
        return;
    };
    let catalog = Arc::new(catalog);
    let out = std::env::temp_dir().join(format!("nautiloid-add-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let mut tried = 0;
    for entry in std::fs::read_dir(saves).unwrap().flatten() {
        let Ok(mut session) = Session::open(&entry.path()) else {
            continue;
        };
        if session.summary().honour_mode {
            continue;
        }
        session.set_catalog(catalog.clone());
        let Some(inventory) = session.original().inventories.first() else {
            continue;
        };
        let holder = inventory.holder;
        let before: Vec<String> = inventory.items.iter().map(|i| i.stats.clone()).collect();
        // One of each kind that can be made from something the holder carries, and a plain
        // copy of something it carries.
        let mut wanted: Vec<String> = Vec::new();
        for kind in [Kind::Weapon, Kind::Armor, Kind::Object] {
            let made = catalog.listed().find(|item| {
                item.kind == kind && matches!(session.pattern_for(holder, item), Ok((_, false)))
            });
            wanted.extend(made.map(|item| item.stats.clone()));
        }
        let copied = catalog
            .listed()
            .find(|item| matches!(session.pattern_for(holder, item), Ok((_, true))));
        wanted.extend(copied.map(|item| item.stats.clone()));
        if wanted.is_empty() {
            continue;
        }
        for (time, stats) in wanted.iter().enumerate() {
            session.add_item(holder, stats, time as f64 * 10.0).unwrap();
        }
        let exported = session
            .export(&out, &format!("Added {tried}"))
            .unwrap_or_else(|e| panic!("{}: {e:#}", entry.path().display()));
        let reopened = Session::open(&exported).unwrap();
        let after: Vec<String> = reopened
            .original()
            .inventories
            .iter()
            .find(|i| i.holder == holder)
            .unwrap()
            .items
            .iter()
            .map(|i| i.stats.clone())
            .collect();
        for stats in &wanted {
            let count = |items: &[String]| items.iter().filter(|s| *s == stats).count();
            assert_eq!(
                count(&after),
                count(&before) + 1,
                "{}: {stats}",
                entry.path().display()
            );
        }
        tried += 1;
    }
    std::fs::remove_dir_all(&out).unwrap();
    assert!(tried > 0, "no save had items to add");
}
