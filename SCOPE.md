# Scope

Nautiloid is a save editor for Baldur's Gate 3, written from scratch in Rust with egui. It takes
its interface design from other save editors: a save browser, a sidebar of
tabs, a Changes panel where every change has its own Revert, undo and redo, and an export that
always writes a new save. No code is shared with other editors, so nothing of their licences or
authorship carries over. Nautiloid is GPL-3.0-only.

## Target

- **Game version:** Patch 8. The current build in the sample saves is `4.1.1.7631656`
  (engine version 4.8). Saves written by other versions are listed but not editable until their
  format has been checked.
- **Platforms:** Windows, macOS, Linux and Steam Deck (Proton app ID 1086940). PC saves only.

## Safety rules

- The original save is never modified. Every export is a new save folder.
- After writing, the exported save is read back and compared before it is kept.
- Every change is listed and reverts exactly. Edits are all-or-nothing: if a change can't be
  made safely, the save is left as it was and the editor explains why.
- A save that is opened and exported without changes must come out byte-for-byte identical in
  its uncompressed contents. This is the test every later feature depends on.

## Build order

### Phase 0: format engine (no interface)

- Reader and writer for the save package (`.lsv`) and the Larian binary files inside it (`.lsf`).
- Round-trip test over the sample saves: read, write, compare.
- A command-line dump tool that prints a save's contents as text, so that two saves taken before
  and after one known change in the game can be compared to locate a field.

### Phase 1: browser and read-only inspector

- Save browser grouped by campaign, with thumbnails, game version, party and a "modded" marker.
- Application shell: header, sidebar, notices, dialogs, Changes panel, undo and redo.
- Save info tab, and read-only views of the party and inventories.

### Phase 2: edits, in this order

1. **Party.** Per character: level and experience, ability scores. Party-wide: Inspiration and
   camp supplies.
2. **Inventory.** Stack quantities and gold, for each character and the camp chest.
3. **Approval.** Companion approval values.
4. **Mods.** The save's list of required mods, with removal of ones that are no longer
   installed.

### Phase 3: later

- Adding items from a bundled catalog of template IDs and names. The catalog is built from the
  game's data by a script; game files are not shipped.

### Stretch goal

- **Difficulty.** Converting an Honour Mode save to Custom or Balanced, and rescuing a failed
  Honour run. None of the sample saves is an Honour Mode save, so this needs one before it can
  start.

## Out of scope

- Global and story flags, and anything else in the story-script state (`StorySave.bin`). The
  file is carried through unchanged.
- Appearance. The game's Magic Mirror covers it.
- Class, subclass and multiclass changes. Withers' respec covers it.
- Forcing story outcomes or reviving story characters.
- Console saves.

## What the sample saves show

Eight saves were examined: five from the current build, one from the Patch 8 release build
(`4.1.1.6758295`), and two from Patch 4 and Patch 5 (`4.1.1.3956130`, `4.1.1.4061076`). They
are private and are not in this repository. The scripts that produced these findings are in
`scripts/research/`.

**Save folder.** Each save is a folder named `<Leader>-<digits>__<SaveName>` that holds
`<SaveName>.lsv` and `<SaveName>.WebP`.

**Package.** Every `.lsv` is a Larian package, version 18, in one part. The header is 40 bytes:
magic `LSPK`, version, file list offset (8 bytes), file list size, flags, priority, a 16-byte
MD5 and the part count. The file list is a count, a compressed size, and an LZ4 block of
272-byte entries: a 256-byte name, a 48-bit offset, part number, flags, size on disk and
uncompressed size.

**Compression changed between patches.** The low four bits of an entry's flags give the method.
The Patch 4 and 5 saves use zlib (`0x21`). Every Patch 8 save uses Zstandard (`0x23`). The
editor writes Zstandard for Patch 8.

**Contents.** A current save holds seven files or more:

| File | What it is | Uncompressed size |
| --- | --- | --- |
| `meta.lsf` | Save metadata: game version, mod list, difficulty and custom rules, party summary | 3 to 9 KB |
| `SaveInfo.json` | Plain JSON summary of the party: classes, level, experience, origin, race, position. Also the game version and current level | under 3 KB |
| `Globals.lsf` | Party and global entity data | 2 to 5 MB |
| `LevelCache/<level>.lsf` | One per visited level | up to 23 MB |
| `StorySave.bin` | Story-script state. Out of scope; copied unchanged | 33 to 76 MB |
| `<SaveName>.WebP` | The same screenshot that sits next to the `.lsv` | 9 to 190 KB |

`SaveInfo.json` alone is enough for the save browser's party line. An edit to level or
experience has to keep it in step with the entity data.

**LSF files.** All are format version 7. The header is the magic `LSOF`, the version, a 64-bit
engine version, then five pairs of uncompressed and on-disk sizes (strings, keys, nodes,
attributes, values), a compression flag and a metadata format. In `meta.lsf` and
`Globals.lsf` of all eight saves the keys section is empty, the strings section is a plain LZ4
block and the other three are LZ4 frames, and the sizes account for every byte of the file.

**Mods.** All eight saves are modded. The mod list is in `meta.lsf` under `ModuleSettings`,
one `ModuleShortDesc` per mod with its folder, name, UUID and an MD5. The Patch 8 saves also
carry `HasUnofficialMods`, `Crossplay`, `OriginalPlatform` and a `PublishHandle` per mod.

**Difficulty.** `meta.lsf` holds `Difficulty`, `CustomRulesetValues`, `DisabledSingleSave` and
`DishonorDifficultySelection`. Five of the six Patch 8 saves carry custom ruleset values.

## Not yet known

- Where each Phase 2 value is stored: ability scores, experience, gold and stack counts,
  Inspiration, camp supplies and approval. Approval may be in the story-script state instead of
  the entity data; if so it moves out of Phase 2.
- Whether the game checks the MD5 in the package header or the per-mod MD5s on load.
- How the game names and discovers a new save folder, and whether an exported copy needs a new
  save ID to appear beside the original.
- Whether Zstandard output from a different encoder, with different compressed bytes but the
  same contents, loads without complaint. This needs one test in the game.
