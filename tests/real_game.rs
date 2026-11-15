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
