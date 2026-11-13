//! Round-trip checks against real saves.
//!
//! Real saves are private and are not in the repository. Point `NAUTILOID_SAVES` at a folder
//! of save folders (the game's `Savegames/Story`, or a copy) to run these; without it they
//! pass without checking anything.

use nautiloid_save_editor::lsf::{Lsf, Sections};
use nautiloid_save_editor::lspk::{Method, Package};
use nautiloid_save_editor::newage::NewAge;
use nautiloid_save_editor::party;
use nautiloid_save_editor::save::{self, Save, Support};
use nautiloid_save_editor::saves::{self, Listed};
use nautiloid_save_editor::session::{Field, Session};
use nautiloid_save_editor::value;
use std::path::PathBuf;

fn saves() -> Vec<PathBuf> {
    let Some(root) = std::env::var_os("NAUTILOID_SAVES") else {
        eprintln!("NAUTILOID_SAVES is not set; skipping the real save checks");
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::from(root)];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lsv"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    assert!(!found.is_empty(), "NAUTILOID_SAVES has no .lsv files");
    found
}

#[test]
fn unchanged_saves_write_back_identical() {
    for path in saves() {
        let bytes = std::fs::read(&path).unwrap();
        let package =
            Package::read(bytes.clone()).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        assert!(
            package.write().unwrap() == bytes,
            "{}: rewritten package differs",
            path.display()
        );
        assert_eq!(
            package.compute_md5().unwrap(),
            package.stored_md5(),
            "{}: header checksum",
            path.display()
        );
    }
}

#[test]
fn every_document_and_value_round_trips() {
    for path in saves() {
        let package = Package::read(std::fs::read(&path).unwrap()).unwrap();
        let mut documents = 0;
        let mut attributes = 0usize;
        for entry in package.entries() {
            if !entry.name().ends_with(".lsf") {
                continue;
            }
            let label = format!("{} / {}", path.display(), entry.name());
            let bytes = package
                .unpack(entry)
                .unwrap_or_else(|e| panic!("{label}: {e:#}"));
            let original = Sections::read(&bytes).unwrap_or_else(|e| panic!("{label}: {e:#}"));
            let lsf =
                Lsf::from_sections(original.clone()).unwrap_or_else(|e| panic!("{label}: {e:#}"));
            assert!(
                lsf.to_sections().unwrap() == original,
                "{label}: sections differ"
            );

            // Every value must decode, and encode back to the bytes it came from.
            for attr in 0..lsf.attr_count() as u32 {
                let type_id = lsf.attr_type(attr);
                let raw = lsf.attr_bytes(attr);
                let decoded = value::decode(type_id, raw).unwrap_or_else(|e| {
                    panic!(
                        "{label}: {} ({}): {e:#}",
                        lsf.attr_name(attr),
                        value::type_name(type_id)
                    )
                });
                assert!(
                    value::encode(type_id, &decoded).unwrap() == raw,
                    "{label}: {} does not encode back",
                    lsf.attr_name(attr)
                );
            }
            documents += 1;
            attributes += lsf.attr_count();
        }
        assert!(
            documents >= 3,
            "{}: only {documents} documents",
            path.display()
        );
        eprintln!(
            "{}: {documents} documents, {attributes} attributes",
            path.display()
        );
    }
}

#[test]
fn re_encoded_saves_hold_the_same_contents() {
    for path in saves() {
        let original = Package::read(std::fs::read(&path).unwrap()).unwrap();
        // Only Patch 8 saves are written; older ones use zlib and are read-only.
        if original
            .entries()
            .iter()
            .any(|e| e.method() != Method::Zstd)
        {
            continue;
        }
        let mut package = original.clone();
        for entry in original.entries() {
            if entry.name().ends_with(".lsf") {
                let lsf = Lsf::parse(&original.unpack(entry).unwrap()).unwrap();
                package
                    .replace(entry.name(), &lsf.write().unwrap())
                    .unwrap();
            }
        }
        let written = Package::read(package.write().unwrap()).unwrap();
        assert_eq!(written.entries().len(), original.entries().len());
        for (before, after) in original.entries().iter().zip(written.entries()) {
            assert_eq!(before.name(), after.name());
            let label = format!("{} / {}", path.display(), before.name());
            let old = original.unpack(before).unwrap();
            let new = written.unpack(after).unwrap();
            if before.name().ends_with(".lsf") {
                // The compressed bytes differ; what they hold must not.
                assert!(
                    Sections::read(&old).unwrap() == Sections::read(&new).unwrap(),
                    "{label}: sections differ"
                );
            } else {
                assert!(old == new, "{label}: contents differ");
            }
        }
        assert_eq!(
            written.compute_md5().unwrap(),
            written.stored_md5(),
            "{}",
            path.display()
        );
    }
}

#[test]
fn entity_data_parses_and_matches_the_summary() {
    for path in saves() {
        let save = Save::open(&path).unwrap();
        let mut blobs = 0;
        for name in save.lsf_names() {
            let lsf = save.lsf(name).unwrap();
            if NewAge::attribute(&lsf).is_none() {
                continue;
            }
            let label = format!("{} / {name}", path.display());
            let blob = NewAge::from_lsf(&lsf).unwrap_or_else(|e| panic!("{label}: {e:#}"));
            assert!(
                blob.tables().len() > 50,
                "{label}: only {} tables",
                blob.tables().len()
            );
            // Storing an unchanged blob leaves the document as it was.
            let mut copy = lsf.clone();
            blob.store(&mut copy).unwrap();
            assert!(
                copy.to_sections().unwrap() == lsf.to_sections().unwrap(),
                "{label}"
            );
            blobs += 1;
        }
        assert!(blobs >= 2, "{}: {blobs} blobs", path.display());

        // The party's level and experience in the entity data are the ones the game wrote
        // in its own summary. Older saves lay the tables out differently and are read-only.
        if save.support() != Support::Editable {
            continue;
        }
        let globals = save.lsf(save::GLOBALS).unwrap();
        let entities = NewAge::from_lsf(&globals).unwrap();
        let read = party::read(&globals, &entities)
            .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        let mut found: Vec<(u32, u64)> = read
            .characters
            .iter()
            .map(|c| (c.level, c.experience as u64))
            .collect();
        let mut expected: Vec<(u32, u64)> = save
            .info()
            .unwrap()
            .party
            .iter()
            .filter(|m| !m.classes.is_empty())
            .map(|m| (m.level, m.experience_total))
            .collect();
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "{}", path.display());

        // In every sample no level-up is pending and each party member has a character
        // creation record. A character played as an origin has no name stored, and a game
        // that has just begun has no Inspiration yet.
        for c in &read.characters {
            assert_eq!(c.entitled_level, c.level, "{}", path.display());
            assert!(c.allocated.is_some(), "{}", path.display());
        }
        if let Some(inspiration) = read.inspiration {
            assert!(inspiration.amount <= inspiration.max, "{}", path.display());
        }
    }
}

#[test]
fn party_edits_read_back_from_a_written_save() {
    for path in saves() {
        let save = Save::open(&path).unwrap();
        if save.support() != Support::Editable {
            continue;
        }
        let label = path.display().to_string();
        let mut globals = save.lsf(save::GLOBALS).unwrap();
        let mut entities = NewAge::from_lsf(&globals).unwrap();
        let before = party::read(&globals, &entities).unwrap();
        let first = &before.characters[0];

        // One of each edit, on the first character.
        party::set_experience(&mut entities, first.entity, first.experience + 123).unwrap();
        party::set_entitled_level(&mut entities, first.entity, first.level + 1).unwrap();
        let points = first.allocated.unwrap()[0];
        party::set_allocated(&mut entities, first.entity, 0, points + 1).unwrap();
        let gold = before.characters.iter().flat_map(|c| &c.gold).next();
        if let Some(stack) = gold {
            party::set_gold(&mut entities, stack.item, stack.amount + 1000).unwrap();
        }
        let fewer = before.inspiration.map(|i| i.amount.saturating_sub(1));
        if let Some(fewer) = fewer {
            party::set_inspiration(&mut entities, fewer).unwrap();
        }

        // Through the whole export path: into the document, into the package, out to bytes
        // and back.
        entities.store(&mut globals).unwrap();
        let mut package = save.package.clone();
        package
            .replace(save::GLOBALS, &globals.write().unwrap())
            .unwrap();
        let written = Package::read(package.write().unwrap()).unwrap();
        assert_eq!(
            written.compute_md5().unwrap(),
            written.stored_md5(),
            "{label}"
        );
        let globals = Lsf::parse(&written.read_file(save::GLOBALS).unwrap()).unwrap();
        let entities = NewAge::from_lsf(&globals).unwrap_or_else(|e| panic!("{label}: {e:#}"));
        let after = party::read(&globals, &entities).unwrap();

        let mut expected = before.clone();
        expected.characters[0].experience += 123;
        expected.characters[0].entitled_level += 1;
        expected.characters[0].allocated.as_mut().unwrap()[0] += 1;
        if let Some(stack) = gold {
            for c in &mut expected.characters {
                for g in &mut c.gold {
                    if g.item == stack.item {
                        g.amount += 1000;
                    }
                }
                c.gold.sort_by(|a, b| {
                    (a.in_container, std::cmp::Reverse(a.amount), a.item.0).cmp(&(
                        b.in_container,
                        std::cmp::Reverse(b.amount),
                        b.item.0,
                    ))
                });
            }
        }
        if let (Some(inspiration), Some(fewer)) = (expected.inspiration.as_mut(), fewer) {
            inspiration.amount = fewer;
        }
        assert_eq!(after, expected, "{label}");
        // Every other file in the save is untouched.
        for entry in save.package.entries() {
            if entry.name() != save::GLOBALS {
                assert!(
                    save.package.unpack(entry).unwrap() == written.read_file(entry.name()).unwrap(),
                    "{label}: {}",
                    entry.name()
                );
            }
        }
    }
}

#[test]
fn the_browser_summary_agrees_with_the_full_read() {
    let Some(root) = std::env::var_os("NAUTILOID_SAVES") else {
        return;
    };
    let listed = saves::scan(std::path::Path::new(&root)).unwrap();
    assert!(!listed.is_empty());
    for entry in &listed {
        let Listed::Save(summary) = entry else {
            panic!("a sample save could not be summarized: {entry:?}");
        };
        // The summary reads two small files; opening the save reads all of it. They must
        // tell the same story.
        let save = Save::open(&summary.folder).unwrap();
        let info = save.info().unwrap();
        assert_eq!(summary.lsv, save.path);
        assert_eq!(summary.game_version, info.game_version, "{}", summary.name);
        assert_eq!(summary.party, info.party, "{}", summary.name);
        assert_eq!(
            summary.support == Support::Editable,
            save.support() == Support::Editable
        );
        assert!(
            !summary.campaign.is_empty() && !summary.leader.is_empty(),
            "{}",
            summary.name
        );
        assert!(
            summary.saved_at > 1_600_000_000,
            "{}: {}",
            summary.name,
            summary.saved_at
        );
        assert!(summary.screenshot.is_some(), "{}", summary.name);
    }
    let campaigns = saves::campaigns(&listed);
    let total: usize = campaigns.iter().map(|c| c.saves.len()).sum();
    assert_eq!(total, listed.len());
    for campaign in &campaigns {
        assert!(campaign
            .saves
            .windows(2)
            .all(|w| w[0].saved_at >= w[1].saved_at));
    }
}

#[test]
fn a_session_exports_a_real_save_that_reopens_with_its_changes() {
    let out = std::env::temp_dir().join(format!("nautiloid-real-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    for path in saves() {
        let folder = path.parent().unwrap();
        let Ok(mut session) = Session::open(folder) else {
            // Saves from older patches are refused, with the reason.
            let save = Save::open(&path).unwrap();
            assert_ne!(save.support(), Support::Editable, "{}", path.display());
            continue;
        };
        let before = std::fs::read(&path).unwrap();
        let experience = session.value(Field::Experience(0)).unwrap();
        session.set(Field::Experience(0), experience + 500, 0.0);
        session.set(Field::LevelUps(0), 1, 10.0);
        if let Some(points) = session.value(Field::Allocated(0, 3)) {
            session.set(Field::Allocated(0, 3), points + 2, 20.0);
        }
        let expected = session.party().clone();

        let name = session.suggested_name(&out);
        let exported = session
            .export(&out, &name)
            .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        assert!(
            std::fs::read(&path).unwrap() == before,
            "{}: original changed",
            path.display()
        );

        let reopened = Session::open(&exported).unwrap();
        assert!(!reopened.is_dirty());
        assert_eq!(reopened.value(Field::Experience(0)), Some(experience + 500));
        assert_eq!(reopened.value(Field::LevelUps(0)), Some(1));
        assert_eq!(reopened.party().characters.len(), expected.characters.len());
        // The browser lists it under its new name, in the same campaign.
        let summary = saves::summarize(&exported).unwrap();
        assert_eq!(summary.name, name);
        assert_eq!(summary.campaign, session.summary().campaign);
        std::fs::remove_dir_all(&exported).unwrap();
    }
    std::fs::remove_dir_all(&out).unwrap();
}
