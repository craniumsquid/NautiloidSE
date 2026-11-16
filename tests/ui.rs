//! Drives the real interface headlessly and saves a picture of each screen to
//! `target/ui-snapshots/`. The checks are about behaviour; the pictures are for people.

mod support;

use eframe::egui;
use support::{sample_saves, temp_dir, Harness};

const WINDOW: [f32; 2] = [1440.0, 900.0];

#[test]
fn browser_lists_campaigns_and_saves() {
    let dir = temp_dir("browser");
    sample_saves(&dir);
    let mut ui = Harness::new(Some(dir.clone()), WINDOW);
    assert!(ui.sees("Choose a save"));
    assert!(ui.sees("6 saves in 2 campaigns"));
    assert!(ui.sees("Shattered Sanctum - 8h 02m"));
    assert!(ui.sees("5 saves · 15 minutes ago"));
    ui.snapshot("01-browser");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn opens_a_save_and_shows_the_party() {
    let dir = temp_dir("party");
    sample_saves(&dir);
    let mut ui = Harness::new(Some(dir.clone()), WINDOW);
    ui.click("Shattered Sanctum - 8h 02m");
    assert!(ui.app.session().is_some(), "clicking a card opens the save");
    assert!(ui.sees("Party"));
    assert!(ui.sees("Tav"));
    assert!(ui.sees("Nothing changed yet."));
    ui.snapshot("02-party");
    // The second character is further down.
    ui.scroll(egui::pos2(600.0, 500.0), -900.0);
    assert!(ui.sees("Karlach"));
    assert!(ui.sees("Carries none."));
    ui.snapshot("02b-party-scrolled");
    // Below the party, the companions waiting at camp.
    ui.scroll(egui::pos2(600.0, 500.0), -900.0);
    assert!(ui.sees("At camp"));
    assert!(ui.sees("Gale"));
    assert!(ui.sees("Level 4 · At camp"));
    ui.snapshot("02c-party-at-camp");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn approval_has_its_own_tab_and_can_go_below_zero() {
    use nautiloid_save_editor::session::Field;
    let (dir, mut ui) = open_sample("approval");
    ui.click("Approval");
    assert!(ui.sees("What each companion thinks of your character, from −50 to 100."));
    assert!(ui.sees("Gale"));
    assert!(ui.sees("At camp"));
    // Eleven steps down from 10.
    for _ in 0..11 {
        ui.click("−");
    }
    assert_eq!(
        ui.app.session().unwrap().value(Field::Approval(2, 0)),
        Some(-1)
    );
    assert!(ui.sees("10 ➡ −1") || ui.sees("−1"));
    ui.snapshot("12-approval");
    // The change is counted on this tab, not on Party.
    ui.click("Party");
    assert_eq!(ui.app.session().unwrap().changes().len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn inventory_lists_holders_and_changes_a_stack() {
    use nautiloid_save_editor::session::{Field, Session};
    let (dir, mut ui) = open_sample("inventory");
    ui.click("Inventory");
    // The hero's things: one stack and two single items.
    assert!(ui.sees("STACKS (1)"));
    assert!(ui.sees("Potion Healing"));
    assert!(ui.sees("OBJ_Potion_Healing"));
    assert!(ui.sees("SINGLE ITEMS (2)"));
    assert!(ui.sees("Rope"));
    ui.click("+");
    assert_eq!(
        ui.app.session().unwrap().value(Field::Amount(0, 1)),
        Some(4)
    );
    ui.snapshot("13-inventory");

    // The camp chest, where food shows what it is worth.
    ui.click("Camp chest");
    assert!(ui.sees("Fruit Apple"));
    assert!(ui.sees("CONS_FOOD_Fruit_Apple · 3 camp supplies each"));
    ui.click("+");
    assert_eq!(
        ui.app.session().unwrap().value(Field::Amount(1, 0)),
        Some(5)
    );
    ui.snapshot("13b-inventory-camp-chest");

    // Searching narrows the list.
    ui.click("Tav");
    ui.click("Filter Tav's items");
    ui.type_text("rope");
    assert!(ui.sees("STACKS (0)"));
    assert!(ui.sees("SINGLE ITEMS (1)"));

    // Both changes are on this tab's count and survive an export.
    assert_eq!(ui.app.session().unwrap().changes().len(), 2);
    ui.click("Export as a new save…");
    ui.click("Export");
    let exported =
        Session::open(&dir.join("Tav-1234__Shattered Sanctum - 8h 02m - Edited")).unwrap();
    assert_eq!(exported.value(Field::Amount(0, 1)), Some(4));
    assert_eq!(exported.value(Field::Amount(1, 0)), Some(5));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mods_are_listed_with_whether_the_game_has_them_active() {
    use nautiloid_save_editor::saves::build::mod_uuid;
    // The game's own layout: the profile's mod list beside its Savegames folder.
    let root = temp_dir("mods");
    let story = root.join("Public/Savegames/Story");
    std::fs::create_dir_all(&story).unwrap();
    sample_saves(&story);
    let entry = |uuid: String| {
        format!(
            "<node id=\"ModuleShortDesc\"><attribute id=\"UUID\" type=\"guid\" value=\"{uuid}\"/></node>"
        )
    };
    std::fs::write(
        root.join("Public/modsettings.lsx"),
        format!(
            "<save><region id=\"ModuleSettings\"><node id=\"root\"><children><node id=\"Mods\"><children>{}{}</children></node></children></node></region></save>",
            entry(mod_uuid(0).to_string().to_uppercase()),
            entry(mod_uuid(1).to_string()),
        ),
    )
    .unwrap();

    let mut ui = Harness::new(Some(story.clone()), WINDOW);
    ui.click("Shattered Sanctum - 8h 02m");
    ui.click("Mods");
    assert!(ui.sees("ADDED MODS (12)"));
    assert!(ui.sees("Sample mod 1"));
    assert!(ui.find_all("ACTIVE").len() >= 2);
    assert_eq!(ui.find_all("NOT ACTIVE").len(), 10);
    // Nothing here changes the save.
    assert!(!ui.sees("Remove"));
    assert!(!ui.app.session().unwrap().is_dirty());
    ui.snapshot("14-mods");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn items_take_the_names_from_the_game_once_it_is_found() {
    use nautiloid_save_editor::catalog::build::game_folder;
    let (dir, mut ui) = open_sample("names");
    // A game install somewhere else, with the sample's items in it.
    let game = dir.join("Baldurs Gate 3");
    game_folder(
        &game,
        &[
            (
                "00000000-0000-0000-0000-00000000dddd",
                "OBJ_Potion_Healing",
                "Potion of Healing",
            ),
            (
                "00000000-0000-0000-0000-00000000cccc",
                "OBJ_Rope",
                "Rope of Climbing",
            ),
        ],
    )
    .unwrap();

    // Without it, the game's internal names, made readable, and a way to point at it.
    ui.click("Inventory");
    assert!(ui.sees("the game wasn't found"));
    assert!(ui.sees("Potion Healing"));
    ui.platform.borrow_mut().folder_to_pick = Some(game.clone());
    ui.click("Choose game folder…");
    ui.settle();
    assert!(!ui.sees("the game wasn't found"));
    assert!(ui.sees("Potion of Healing"));
    assert!(ui.sees("Rope of Climbing"));
    // The internal name is still there, beneath.
    assert!(ui.sees("OBJ_Potion_Healing"));
    // An item the game's files don't have keeps its readable internal name.
    assert!(ui.sees("Bag Alchemy Pouch"));
    ui.click("+");
    let change = ui.app.session().unwrap().changes().pop().unwrap();
    assert_eq!(change.what, "Potion of Healing");
    ui.snapshot("15-inventory-names");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn adds_items_from_the_games_files() {
    use nautiloid_save_editor::catalog::build::{game_folder_with, GameItem};
    use nautiloid_save_editor::catalog::Kind;
    use nautiloid_save_editor::party::Holder;
    use nautiloid_save_editor::session::Session;
    let (dir, mut ui) = open_sample("add-items");
    let item = |template, stats, display, kind, category| GameItem {
        template,
        stats,
        display,
        kind,
        category,
        max_stack: 1,
    };
    let game = dir.join("Baldurs Gate 3");
    game_folder_with(
        &game,
        &[
            item(
                "00000000-0000-0000-0000-00000000cccc",
                "OBJ_Rope",
                "Rope",
                Kind::Object,
                "Tool",
            ),
            item(
                "00000000-0000-0000-0000-00000000ffff",
                "OBJ_Rope_Silk",
                "Silk Rope",
                Kind::Object,
                "Tool",
            ),
            item(
                "00000000-0000-0000-0000-000000001111",
                "WPN_Longsword_1",
                "Longsword +1",
                Kind::Weapon,
                "",
            ),
            item(
                "00000000-0000-0000-0000-000000005555",
                "WPN_Longsword",
                "Longsword",
                Kind::Weapon,
                "",
            ),
        ],
    )
    .unwrap();

    // Without the game's files there is nothing to choose from.
    ui.click("Inventory");
    ui.click("Add items to Tav…");
    assert!(ui.app.dialog().is_none());
    ui.platform.borrow_mut().folder_to_pick = Some(game.clone());
    ui.click("Choose game folder…");
    ui.settle();

    ui.click("Add items to Tav…");
    assert!(ui.sees("Add items for Tav"));
    assert!(ui.sees("search the game's 4 items"));
    ui.type_text("rope");
    assert!(ui.sees("Silk Rope"));
    assert!(ui.sees("Object · Tool · OBJ_Rope_Silk"));
    // The rope Tav holds first, then the silk rope made from it.
    ui.click_nth("Add", 0);
    ui.click_nth("Add", 0);
    ui.click_nth("Add", 1);
    assert!(ui.sees("2 added"));
    assert!(ui.sees("1 added"));
    ui.snapshot("16-add-items");

    // Nobody in the party has a weapon, but a sword can still be added: it is made from
    // the one the trader has equipped.
    ui.press(egui::Key::A, egui::Modifiers::COMMAND);
    ui.type_text("long");
    assert!(ui.sees("Longsword +1"));
    ui.click_nth("Add", 1);
    assert!(ui.sees("1 added"));
    ui.type_text("zzz");
    assert!(ui.sees("No item in the game's files is called that."));
    ui.click("Done");
    assert!(ui.app.dialog().is_none());

    // What is to be added is listed, can be changed, and is on this tab's changes.
    assert!(ui.sees("ADDING (3)"));
    assert!(ui.sees("A copy of the one Tav holds · OBJ_Rope"));
    assert!(ui.sees("Made from a Rope Tav holds · OBJ_Rope_Silk"));
    assert!(ui.sees("Made from a Longsword someone outside the party has · WPN_Longsword_1"));
    // A single item says why its count can't change, and has a + to add more instead: the
    // pluses are the three being added, the stack's, the bag's and the rope's.
    assert!(ui.sees("the save keeps no count for these"));
    // The rope Tav has is marked as wares; the game's mark shows beside it.
    assert!(ui.sees("OBJ_Rope · wares"));
    ui.click_nth("+", 5);
    assert_eq!(
        ui.app
            .session()
            .unwrap()
            .added(Holder::Character(0), "OBJ_Rope"),
        3
    );
    let whats: Vec<String> = ui
        .app
        .session()
        .unwrap()
        .changes()
        .iter()
        .map(|c| c.what.clone())
        .collect();
    assert_eq!(whats, ["Add Rope", "Add Silk Rope", "Add Longsword +1"]);
    ui.snapshot("16b-adding");

    // Filtering for something Tav doesn't have finds it among the game's items, to add
    // there and then, or in the dialog with the search already made.
    ui.click("Filter Tav's items");
    ui.type_text("silk");
    assert!(ui.sees("SINGLE ITEMS (0)"));
    assert!(ui.sees("ADD TO TAV FROM THE GAME'S ITEMS"));
    assert!(ui.sees("Silk Rope"));
    ui.snapshot("16c-filter-finds-the-game's-items");
    ui.click("Add");
    assert_eq!(
        ui.app
            .session()
            .unwrap()
            .added(Holder::Character(0), "OBJ_Rope_Silk"),
        2
    );
    ui.click("Add items to Tav…");
    assert!(ui.sees("Silk Rope"));
    assert!(ui.sees("2 added"));
    ui.click("Done");

    ui.click("Export as a new save…");
    ui.click("Export");
    let exported =
        Session::open(&dir.join("Tav-1234__Shattered Sanctum - 8h 02m - Edited")).unwrap();
    let tav: Vec<&str> = exported.original().inventories[0]
        .items
        .iter()
        .map(|i| i.stats.as_str())
        .collect();
    assert_eq!(tav.iter().filter(|s| **s == "OBJ_Rope").count(), 4);
    assert_eq!(tav.iter().filter(|s| **s == "OBJ_Rope_Silk").count(), 2);
    assert_eq!(tav.iter().filter(|s| **s == "WPN_Longsword_1").count(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exporting_an_honour_mode_save_says_how_to_load_it() {
    let dir = temp_dir("honour");
    sample_saves(&dir);
    let mut ui = Harness::new(Some(dir.clone()), WINDOW);
    ui.click("Gale");
    ui.click("HonourMode");
    assert!(ui.app.session().is_some());
    ui.click("Export as a new save…");
    assert!(ui.sees("This is an Honour Mode save."));
    ui.snapshot("11-export-honour");
    ui.click("Cancel");

    // An ordinary save gets no such note.
    ui.click("⬅  All saves");
    ui.click("Tav");
    ui.click("Shattered Sanctum - 8h 02m");
    ui.click("Export as a new save…");
    assert!(ui.sees("Export as a new save"));
    assert!(!ui.sees("This is an Honour Mode save."));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Opens the newest sample save.
fn open_sample(tag: &str) -> (std::path::PathBuf, Harness) {
    let dir = temp_dir(tag);
    sample_saves(&dir);
    let mut ui = Harness::new(Some(dir.clone()), WINDOW);
    ui.click("Shattered Sanctum - 8h 02m");
    assert!(ui.app.session().is_some());
    (dir, ui)
}

// The plus buttons on the Party tab, top to bottom: Inspiration, then for the first
// character experience, level-ups and the six abilities.
const PLUS_INSPIRATION: usize = 0;
const PLUS_EXPERIENCE: usize = 1;
const PLUS_LEVEL_UPS: usize = 2;
const PLUS_STRENGTH: usize = 3;

#[test]
fn edits_are_listed_and_can_be_undone_and_reverted() {
    use nautiloid_save_editor::session::Field;
    let (dir, mut ui) = open_sample("edit");
    for _ in 0..3 {
        ui.click_nth("+", PLUS_STRENGTH);
        // Let the edits count as separate steps.
        ui.wait(2.0);
    }
    ui.click_nth("+", PLUS_EXPERIENCE);
    ui.wait(2.0);
    ui.click_nth("+", PLUS_LEVEL_UPS);
    ui.wait(2.0);
    ui.click_nth("+", PLUS_INSPIRATION);
    let session = ui.app.session().unwrap();
    assert_eq!(session.value(Field::Allocated(0, 0)), Some(3));
    assert_eq!(session.value(Field::Experience(0)), Some(5695));
    assert_eq!(session.value(Field::LevelUps(0)), Some(1));
    assert_eq!(session.value(Field::Inspiration), Some(4));
    assert!(ui.sees("4 changes not exported"));
    assert!(ui.sees("Strength points"));
    assert!(ui.sees("The game will offer level 6."));
    ui.snapshot("03-party-edited");

    // Undo and redo, from the keyboard.
    ui.press(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(ui.app.session().unwrap().value(Field::Inspiration), Some(3));
    ui.press(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(ui.app.session().unwrap().value(Field::LevelUps(0)), Some(0));
    ui.press(egui::Key::Y, egui::Modifiers::COMMAND);
    assert_eq!(ui.app.session().unwrap().value(Field::LevelUps(0)), Some(1));
    ui.press(
        egui::Key::Z,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
    assert_eq!(ui.app.session().unwrap().value(Field::Inspiration), Some(4));
    // And from the header.
    ui.click("Undo");
    assert_eq!(ui.app.session().unwrap().value(Field::Inspiration), Some(3));
    ui.click("Redo");
    assert_eq!(ui.app.session().unwrap().value(Field::Inspiration), Some(4));

    // Revert one change from the Changes panel, then all of them.
    assert_eq!(ui.app.session().unwrap().changes().len(), 4);
    ui.click("Revert all");
    assert!(!ui.app.session().unwrap().changes().iter().any(|_| true));
    assert!(ui.sees("Nothing changed yet."));
    ui.click("Undo");
    assert_eq!(ui.app.session().unwrap().changes().len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exports_a_new_save_and_carries_on_from_it() {
    use nautiloid_save_editor::session::{Field, Session};
    let (dir, mut ui) = open_sample("export");
    ui.click_nth("+", PLUS_EXPERIENCE);
    ui.click("Export as a new save…");
    assert!(ui.sees("Writes a new save with your 1 change."));
    ui.snapshot("05-export-dialog");

    // Replace the suggested name.
    ui.press(egui::Key::A, egui::Modifiers::COMMAND);
    ui.type_text("My edit");
    ui.click("Export");
    let exported = dir.join("Tav-1234__My edit");
    assert!(
        exported.join("My edit.lsv").is_file(),
        "the save is written"
    );
    assert!(
        exported.join("My edit.WebP").is_file(),
        "with its screenshot"
    );
    assert_eq!(ui.app.notices(), ["Exported as \"My edit\""]);
    // The exported save is the one open now, with nothing unexported.
    let session = ui.app.session().unwrap();
    assert_eq!(session.summary().name, "My edit");
    assert!(!session.is_dirty());
    assert_eq!(session.value(Field::Experience(0)), Some(5695));
    ui.snapshot("06-after-export");

    ui.click("Show in folder");
    assert_eq!(
        ui.platform.borrow().revealed,
        std::slice::from_ref(&exported)
    );
    // The original is as it was.
    let original = Session::open(&dir.join("Tav-1234__Shattered Sanctum - 8h 02m")).unwrap();
    assert_eq!(original.value(Field::Experience(0)), Some(5595));

    // A name that can't be used keeps the dialog up and says why.
    ui.click_nth("+", PLUS_EXPERIENCE);
    ui.press(egui::Key::E, egui::Modifiers::COMMAND);
    ui.press(egui::Key::A, egui::Modifiers::COMMAND);
    ui.type_text("what?");
    assert!(ui.sees("A name can't contain"));
    ui.press(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(ui.app.dialog().is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn asks_before_leaving_unexported_changes() {
    let (dir, mut ui) = open_sample("leave");
    // With nothing changed, going back needs no asking.
    ui.click("⬅  All saves");
    assert!(ui.app.session().is_none());
    ui.click("Shattered Sanctum - 8h 02m");

    ui.click_nth("+", PLUS_STRENGTH);
    ui.click("⬅  All saves");
    assert!(ui.sees("1 change not exported"));
    assert!(
        ui.app.session().is_some(),
        "still open while the dialog is up"
    );
    ui.snapshot("07-unexported-dialog");
    ui.click("Cancel");
    assert!(ui.app.dialog().is_none() && ui.app.session().is_some());

    ui.click("⬅  All saves");
    ui.click("Discard changes");
    assert!(ui.app.session().is_none());
    assert!(ui.sees("Choose a save"));

    // Closing the window is held back the same way.
    ui.click("Shattered Sanctum - 8h 02m");
    ui.click_nth("+", PLUS_STRENGTH);
    assert!(!ui.app.may_close());
    ui.request_close();
    assert!(ui.sees("If you close now they are lost."));
    ui.click("Discard changes");
    assert!(ui.app.may_close());
    assert!(
        ui.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
            || ui.app.may_close()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn says_why_a_save_cannot_be_opened() {
    use support::{write_sample, Sample};
    let dir = temp_dir("problem");
    write_sample(
        &dir,
        &Sample {
            leader: "Tav",
            name: "Bare",
            campaign: "04f94982-eff9-ed85-72ae-f2bfb78b94b3",
            save_type: 0,
            age: 60,
            colors: None,
            mods: 0,
            with_party: false,
        },
    );
    let mut ui = Harness::new(Some(dir.clone()), WINDOW);
    ui.click("Bare");
    assert!(ui.app.session().is_none());
    assert!(ui.sees("Bare can't be edited"));
    ui.snapshot("08-problem");
    ui.click("Close");
    assert!(ui.app.dialog().is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn finds_saves_by_folder_choice_drop_and_search() {
    let dir = temp_dir("folder");
    sample_saves(&dir);
    let mut ui = Harness::new(None, WINDOW);
    assert!(ui.sees("Your saves folder was not found"));
    ui.snapshot("00-browser-no-folder");

    // Choosing a folder shows its saves.
    ui.platform.borrow_mut().folder_to_pick = Some(dir.clone());
    ui.click("Choose folder…");
    assert!(ui.sees("6 saves in 2 campaigns"));

    // The other campaign.
    ui.click("Gale");
    assert!(ui.sees("HonourMode"));
    assert!(!ui.sees("AutoSave_26"));
    ui.click("Tav");

    // Searching narrows the cards.
    ui.click("Search saves");
    ui.type_text("auto");
    assert!(ui.sees("AutoSave_26"));
    assert!(!ui.sees("QuickSave_3"));

    // Dropping a save's folder on the window opens it; dropping something else says so.
    ui.drop_file(&dir.join("Tav-1234__QuickSave_3"));
    assert_eq!(ui.app.session().unwrap().summary().name, "QuickSave_3");
    ui.click("⬅  All saves");
    let stray = dir.join("notes.txt");
    std::fs::write(&stray, b"x").unwrap();
    ui.drop_file(&stray);
    assert!(ui.app.session().is_none());
    assert_eq!(ui.app.notices().len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shows_save_info_and_fits_a_small_window() {
    let (dir, mut ui) = open_sample("info");
    ui.click("Save info");
    assert!(ui.sees("Game version"));
    assert!(ui.sees("4.1.1.7631656"));
    assert!(ui.sees("MODS (12)"));
    ui.snapshot("04-save-info");
    // The list of files is below the mods.
    ui.scroll(egui::pos2(600.0, 500.0), -900.0);
    assert!(ui.sees("Globals.lsf"));

    ui.click("Party");
    // A small window hides the Changes panel and puts the way out in the header.
    ui.resize([980.0, 640.0]);
    assert!(ui.sees("Inspiration"));
    assert!(!ui.sees("Nothing changed yet."));
    assert!(ui.sees("Export…"));
    ui.snapshot("09-party-small-window");
    ui.click("Changes");
    assert!(ui.sees("Nothing changed yet."));
    ui.snapshot("09b-changes-small-window");
    ui.click("Changes");
    ui.click("⬅  All saves");
    ui.snapshot("10-browser-small-window");
    std::fs::remove_dir_all(&dir).unwrap();
}
