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
fn exporting_an_honour_mode_save_says_it_is_untried() {
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
