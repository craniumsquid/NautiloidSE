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

Built, apart from the two checks that need the game (see Status).

- Reader and writer for the save package (`.lsv`) and the Larian binary files inside it (`.lsf`).
- Reader for the entity data inside those files, where the party's numbers are kept.
- Round-trip test over the sample saves: read, write, compare.
- A command-line tool that lists, dumps, searches and compares saves, so that two saves taken
  before and after one known change in the game can be compared to locate a field.

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

## Status

Phase 0 is built and passes on all eleven sample saves:

- An unchanged save writes back byte for byte.
- Every document writes back identical sections, and all 8.3 million attribute values decode
  and encode to the bytes they came from.
- A save whose documents are all parsed, written and compressed again holds the same contents.
- The party's level and experience read from the entity data match the game's own summary in
  every Patch 8 save.

Two things can only be checked in the game, and Phase 2 depends on both:

1. That the game loads a save this editor compressed.
2. That the game loads a save with a change to the entity data. The data's header has a value
   that may be a checksum; if it is, the function has to be found first.

What was found changes two earlier assumptions:

- The party's level, experience, ability scores, approval, camp supplies and inventories are
  not in the documents' node tree. They are in a binary table store inside `Globals.lsf` (the
  `NewAge` data). It names its tables, so it can be read, but each table's fields have to be
  worked out one at a time.
- Approval is in that entity data, not in the story-script state, so it stays in Phase 2.

The details are in [docs/FORMAT.md](docs/FORMAT.md).
