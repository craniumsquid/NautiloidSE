//! Round-trip checks against real saves.
//!
//! Real saves are private and are not in the repository. Point `NAUTILOID_SAVES` at a folder
//! of save folders (the game's `Savegames/Story`, or a copy) to run these; without it they
//! pass without checking anything.

use nautiloid_save_editor::lsf::{Lsf, Sections};
use nautiloid_save_editor::lspk::{Method, Package};
use nautiloid_save_editor::newage::NewAge;
use nautiloid_save_editor::save::{self, Save, Support};
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
        let mut found: Vec<(u32, u64)> = save::party(&globals, &entities)
            .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()))
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
    }
}
