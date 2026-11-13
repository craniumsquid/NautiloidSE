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

Built, and its output loads in the game (see Status).

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

1. **Party.** Per character: experience, level-ups, ability scores and gold. Party-wide:
   Inspiration.
   - A level-up is granted, not applied: the editor entitles the character to the next level
     and the game takes the player through the choices.
   - Ability scores are changed through the points allocated to each ability; the game adds
     the character's other bonuses on top.
   - Gold is per character. A character who carries none can't be given any until items can
     be added (Phase 3).
2. **Inventory.** Stack quantities, for each character and the camp chest. Camp supplies
   belong here: the game works the total out from the food items.
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

Tried in the game so far:

- A save this code compressed, with nothing changed, loads.
- A save with anything changed in the entity data does not load unless the checksum in that
  data's header is brought up to date. The function is known and built in: it reproduces the
  stored checksum of every sample and predicts all seven in-game results.
- An edit with a correct checksum loads. A change to experience sticks. Changes to an ability
  score total and to the camp supplies total are discarded: the game recalculates both on
  load, so those edits have to be made to whatever they are calculated from.
- Ability scores are calculated from the points allocated at character creation, and a change
  to those sticks.
- Entitling a character to the next level makes the game offer the level-up. Changes to the
  party's Inspiration and to a gold stack stick.

Every value planned for the Party tab is now located, has a function in `src/party.rs` to
read and change it, and has been changed in a save that then loaded with the new value. The
functions reproduce those saves byte for byte.
- The load menu lists a save under its folder's name, so an export can be a new folder beside
  the original, as planned. The files inside have to be named after it, or the copy hangs on
  loading.

What was found changes two earlier assumptions:

- The party's level, experience, ability scores, approval, camp supplies and inventories are
  not in the documents' node tree. They are in a binary table store inside `Globals.lsf` (the
  `NewAge` data). It names its tables, so it can be read, but each table's fields have to be
  worked out one at a time.
- Approval is in that entity data, not in the story-script state, so it stays in Phase 2.

The details are in [docs/FORMAT.md](docs/FORMAT.md).
