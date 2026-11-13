//! Drives the real interface headlessly and saves a picture of each screen to
//! `target/ui-snapshots/`. The checks are about behaviour; the pictures are for people.

mod support;

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
