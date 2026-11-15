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
        // One of each kind made from another item, a plain copy, a scroll given another
        // spell, and a stack of five of something that stacks.
        let mut wanted: Vec<(String, u32)> = Vec::new();
        let mut take = |found: Option<&nautiloid_save_editor::catalog::CatalogItem>, n: u32| {
            if let Some(item) = found {
                if !wanted.iter().any(|(s, _)| *s == item.stats) {
                    wanted.push((item.stats.clone(), n));
                }
            }
        };
        for kind in [Kind::Weapon, Kind::Armor, Kind::Object] {
            take(
                catalog.listed().find(|item| {
                    item.kind == kind
                        && item.max_stack == 1
                        && session.pattern_for(holder, item).is_ok_and(|p| !p.same)
                }),
                1,
            );
        }
        take(
            catalog.listed().find(|item| {
                item.max_stack == 1 && session.pattern_for(holder, item).is_ok_and(|p| p.same)
            }),
            1,
        );
        take(
            catalog.listed().find(|item| {
                session
                    .pattern_for(holder, item)
                    .is_ok_and(|p| p.spell.is_some())
            }),
            1,
        );
        take(
            catalog.listed().find(|item| {
                item.max_stack >= 5
                    && item.spell.is_empty()
                    && session.pattern_for(holder, item).is_ok()
            }),
            5,
        );
        assert!(
            wanted.len() >= 5,
            "{}: only {wanted:?}",
            entry.path().display()
        );
        let mut time = 0.0;
        for (stats, n) in &wanted {
            for _ in 0..*n {
                session.add_item(holder, stats, time).unwrap();
                time += 10.0;
            }
        }
        let exported = session
            .export(&out, &format!("Added {tried}"))
            .unwrap_or_else(|e| panic!("{}: {e:#}", entry.path().display()));
        let reopened = Session::open(&exported).unwrap();
        let items = &reopened
            .original()
            .inventories
            .iter()
            .find(|i| i.holder == holder)
            .unwrap()
            .items;
        for (stats, n) in &wanted {
            let count = items.iter().filter(|i| i.stats == *stats).count();
            let was = before.iter().filter(|s| *s == stats).count();
            assert_eq!(count, was + 1, "{}: {stats}", entry.path().display());
            // A stack is one new item holding them all.
            if *n > 1 {
                assert!(
                    items
                        .iter()
                        .any(|i| i.stats == *stats && i.stacked && i.amount == *n),
                    "{}: {stats} is not a stack of {n}",
                    entry.path().display()
                );
            }
        }
        // Scrolls given another spell cast it.
        let globals = reopened
            .save()
            .lsf(nautiloid_save_editor::save::GLOBALS)
            .unwrap();
        let entities = NewAge::from_lsf(&globals).unwrap();
        for item in items {
            if let Some(known) = catalog.by_stats(&item.stats) {
                if !known.spell.is_empty() && !before.contains(&item.stats) {
                    assert_eq!(
                        nautiloid_save_editor::items::spell_of(&entities, item.entity),
                        Some(known.spell.clone()),
                        "{}: {}",
                        entry.path().display(),
                        item.stats
                    );
                }
            }
        }
        tried += 1;
    }
    std::fs::remove_dir_all(&out).unwrap();
    assert!(tried > 0, "no save had items to add");
}
