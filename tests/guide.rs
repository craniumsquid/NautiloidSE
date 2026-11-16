//! The pictures in `docs/HOW-TO-USE.md`, taken by going through the guide's steps on the
//! sample saves. Every run checks the steps still work and saves the pictures to
//! `target/ui-snapshots/`; with `NAUTILOID_GUIDE_IMAGES` set, they also replace the ones in
//! `docs/images/guide/`:
//!
//! ```bash
//! NAUTILOID_GUIDE_IMAGES=1 cargo test --test guide
//! ```

mod support;

use eframe::egui;
use nautiloid_save_editor::catalog::build::{game_folder_with, GameItem};
use nautiloid_save_editor::catalog::Kind;
use std::path::Path;
use support::{sample_saves, temp_dir, Harness};

const WINDOW: [f32; 2] = [1280.0, 800.0];

// The plus buttons on the Party tab, top to bottom: Inspiration, then for the first
// character experience, level-ups and the six abilities.
const PLUS_EXPERIENCE: usize = 1;
const PLUS_LEVEL_UPS: usize = 2;
const PLUS_STRENGTH: usize = 3;

/// Saves a picture for the guide.
fn picture(ui: &mut Harness, name: &str) {
    let snapshot = format!("guide-{name}");
    ui.snapshot(&snapshot);
    if std::env::var_os("NAUTILOID_GUIDE_IMAGES").is_some() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = root.join("docs/images/guide");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(
            root.join(format!("target/ui-snapshots/{snapshot}.png")),
            dir.join(format!("{name}.png")),
        )
        .unwrap();
    }
}

/// A game install with the sample saves' items and some to add, named as the game names
/// them.
fn game(dir: &Path) -> std::path::PathBuf {
    let item = |template, stats, display, kind, category, max_stack| GameItem {
        template,
        stats,
        display,
        kind,
        category,
        max_stack,
    };
    game_folder_with(
        &dir.join("Baldurs Gate 3"),
        &[
            item(
                "00000000-0000-0000-0000-00000000dddd",
                "OBJ_Potion_Healing",
                "Potion of Healing",
                Kind::Object,
                "PotionHealing",
                99,
            ),
            item(
                "00000000-0000-0000-0000-00000000cccc",
                "OBJ_Rope",
                "Rope",
                Kind::Object,
                "Tool",
                1,
            ),
            item(
                "00000000-0000-0000-0000-00000000bbbb",
                "OBJ_Bag_AlchemyPouch",
                "Alchemy Pouch",
                Kind::Object,
                "Container",
                1,
            ),
            item(
                "00000000-0000-0000-0000-00000000eeee",
                "CONS_FOOD_Fruit_Apple",
                "Red Apple",
                Kind::Object,
                "Food",
                99,
            ),
            item(
                "00000000-0000-0000-0000-000000005555",
                "WPN_Longsword",
                "Longsword",
                Kind::Weapon,
                "",
                1,
            ),
            item(
                "00000000-0000-0000-0000-0000000001a1",
                "OBJ_Potion_Healing_Greater",
                "Potion of Greater Healing",
                Kind::Object,
                "PotionHealing",
                99,
            ),
            item(
                "00000000-0000-0000-0000-0000000001a2",
                "OBJ_Potion_Of_Speed",
                "Potion of Speed",
                Kind::Object,
                "PotionUtility",
                99,
            ),
            item(
                "00000000-0000-0000-0000-0000000001a3",
                "MAG_Mobility_MomentumOnCombatStart_Helmet",
                "Haste Helm",
                Kind::Armor,
                "Helmet",
                1,
            ),
            item(
                "00000000-0000-0000-0000-0000000001a4",
                "WPN_Longsword_1",
                "Longsword +1",
                Kind::Weapon,
                "",
                1,
            ),
            item(
                "00000000-0000-0000-0000-0000000001a5",
                "OBJ_Scroll_Fireball",
                "Scroll of Fireball",
                Kind::Object,
                "MagicScroll_3",
                20,
            ),
        ],
    )
    .unwrap()
}

/// The sample saves where the game keeps saves, with the profile's list of active mods
/// beside them: all but the last two of the sample mods. The folder's name is fixed, so the
/// pictures come out the same on every run.
fn saves_folder() -> std::path::PathBuf {
    use nautiloid_save_editor::saves::build::mod_uuid;
    let root = std::env::temp_dir().join("Larian Studios");
    let _ = std::fs::remove_dir_all(&root);
    let profile = root.join("Baldur's Gate 3/PlayerProfiles/Public");
    let story = profile.join("Savegames/Story");
    std::fs::create_dir_all(&story).unwrap();
    sample_saves(&story);
    let entries: String = (0..10)
        .map(|n| {
            format!(
                "<node id=\"ModuleShortDesc\"><attribute id=\"UUID\" type=\"guid\" value=\"{}\"/></node>",
                mod_uuid(n)
            )
        })
        .collect();
    std::fs::write(
        profile.join("modsettings.lsx"),
        format!(
            "<save><region id=\"ModuleSettings\"><node id=\"root\"><children><node id=\"Mods\"><children>{entries}</children></node></children></node></region></save>"
        ),
    )
    .unwrap();
    story
}

#[test]
fn guide_pictures() {
    let dir = temp_dir("guide");
    let story = saves_folder();
    let data = game(&dir);
    let mut ui = Harness::with_game(Some(story.clone()), Some(data), WINDOW);
    ui.set_scale(2.0);

    // Choosing a save.
    assert!(ui.sees("Choose a save"));
    picture(&mut ui, "01-choose-a-save");

    // The Party tab, with experience raised, a level-up and a point of Strength.
    ui.click("Shattered Sanctum - 8h 02m");
    ui.click_nth("+", PLUS_EXPERIENCE);
    ui.click_nth("+", PLUS_LEVEL_UPS);
    ui.click_nth("+", PLUS_STRENGTH);
    assert_eq!(ui.app.session().unwrap().changes().len(), 3);
    picture(&mut ui, "02-party");

    // The Inventory tab.
    ui.click("Inventory");
    assert!(ui.sees("Potion of Healing"));
    picture(&mut ui, "03-inventory");

    // Filtering for something Tav doesn't have finds it among the game's items.
    ui.click("Filter Tav's items");
    ui.type_text("potion");
    assert!(ui.sees("Potion of Greater Healing"));
    picture(&mut ui, "04-filter-and-add");
    ui.click_nth("Add", 0);

    // The Add items dialog.
    ui.click("potion");
    ui.press(egui::Key::A, egui::Modifiers::COMMAND);
    ui.press(egui::Key::Backspace, egui::Modifiers::NONE);
    ui.click("Add items to Tav…");
    ui.type_text("haste");
    assert!(ui.sees("Haste Helm"));
    ui.click("Add");
    assert!(ui.sees("1 added"));
    picture(&mut ui, "05-add-items");
    ui.click("Done");

    // What is being added, with the potions made a stack of five.
    for _ in 0..4 {
        ui.click_nth("+", 0);
    }
    assert!(ui.sees("ADDING (2)"));
    picture(&mut ui, "06-adding");

    // Approval and mods.
    ui.click("Approval");
    picture(&mut ui, "07-approval");
    ui.click("Mods");
    assert_eq!(ui.find_all("NOT ACTIVE").len(), 2);
    picture(&mut ui, "08-mods");

    // Exporting.
    ui.click("Export as a new save…");
    picture(&mut ui, "09-export");
    ui.click("Export");
    assert!(ui.sees("Exported as"));
    picture(&mut ui, "10-exported");

    // Without the game's files, items go by the game's internal names until the game's
    // folder is chosen.
    let mut ui = Harness::new(Some(story.clone()), WINDOW);
    ui.set_scale(2.0);
    ui.click("Shattered Sanctum - 8h 02m");
    ui.click("Inventory");
    assert!(ui.sees("Choose game folder…"));
    picture(&mut ui, "11-no-game-folder");
    std::fs::remove_dir_all(&dir).unwrap();
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("Larian Studios"));
}
