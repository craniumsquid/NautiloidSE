# How Nautiloid works

For anyone forking or extending the editor: how the code is laid out, how an edit gets from
the window into a save the game accepts, and the rules the game turned out to enforce. Each
rule was found by trying it in the game, on build `4.1.1.7631656`, and most of them cost a
failed load to learn.

The other documents:

- [FORMAT.md](FORMAT.md): what is known about the save format, byte by byte, and a table of
  every change tried in the game and what happened.
- [SCOPE.md](../SCOPE.md): what the editor does and won't do, and where it stands.
- The module comments in `src/lspk.rs`, `src/lsf.rs`, `src/newage.rs`, `src/party.rs` and
  `src/items.rs` give the exact layouts.

## Ground rules

These shaped the code. A fork can change them, but should know what they protect.

- **The save that was opened is never changed.** An export writes a new save folder beside
  it, reads the new save back, and checks that it holds exactly the changes asked for and
  nothing else. If the check fails, the new folder is deleted and the export reports why.
- **Nothing from the game is shipped.** Item names, templates and stats are read at run time
  from the player's own install. Game files used in development stay out of the repository,
  and so do real saves (`*.lsv` is in `.gitignore`); the tests build their own sample saves
  in code.
- **Every change is listed and can be reverted on its own**, and undo and redo work across
  all of them.
- **No code from other save editors.** The NewAge checksum's function and seed come from
  the notes in the [SaveForge](https://github.com/danaketh/SaveForge) project; the code is
  this project's own.
- Licence: GPL-3.0-only.

## Layout

| Path | What it does |
| --- | --- |
| `src/lspk.rs` | The `.lsv` package (LSPK version 18): reading, replacing a file, writing. Files that aren't replaced are written back with their exact bytes |
| `src/lsf.rs` | The binary document format (LSF version 6 and 7): a tree of nodes with typed attributes. Reading, editing values, copying and removing nodes, writing |
| `src/value.rs` | Attribute types and values, including the game's byte order for UUIDs |
| `src/newage.rs` | The entity data inside `Globals.lsf` (the `NewAge` blob): tables, owners, offsets, the checksum, and adding entities, rows and data without moving anything |
| `src/save.rs` | Opening a save, which build made it, exporting a copy under a new name, the save type |
| `src/saves.rs` | Finding saves and reading their summaries quickly, for the browser |
| `src/party.rs` | Reading the party from the entity data (characters, experience, abilities, gold, inventories, stacks, approval, camp supplies) and writing single values back |
| `src/items.rs` | Adding items: copying an item from anywhere in the save into an inventory, making the copy into another item, stacks, the pointer check |
| `src/catalog.rs` | The game's items, read from the player's install: names, kinds, stack sizes, scroll spells |
| `src/mods.rs` | The save's mod list, compared with the mods the game has active |
| `src/session.rs` | An open save being edited: the fields, their ranges, undo and redo, the items to add, and the export with its check |
| `src/diff.rs` | Comparing two saves, for research |
| `src/ui/` | The interface, in eframe and egui 0.29 |
| `src/bin/nautiloid-cli.rs` | The command-line tool used to work the format out and to make test saves |
| `tests/` | Tests against sample saves built in code, against real saves when pointed at them, and of the interface without a window |
| `packaging/`, `build.rs` | The icon for each platform, and the Windows executable's resources |

## From a save folder to an edit

### The save folder and package

A save is a folder `<Leader>-<digits>__<SaveName>` holding `<SaveName>.lsv` and
`<SaveName>.WebP`. The game lists a save by the folder's name after `__`, and the two files
inside must be named to match it, or loading hangs at 0%.

The `.lsv` is a Larian package: a header with an MD5 of every file's uncompressed contents, the
files (each Zstandard-compressed in Patch 8), then an LZ4-compressed file list. It holds
`meta.lsf` (game version, mods, difficulty, save type), `SaveInfo.json`, `Globals.lsf` (the
characters, items and global state), a document per visited level, and `StorySave.bin` (the
story script's state, which the editor never touches). `lspk.rs` reads all of this and, on
writing, keeps every file it wasn't asked to replace byte for byte the same.

### Documents

`.lsf` files are trees of nodes with typed attributes, in five LZ4-compressed sections. The
writer reproduces the game's sections exactly, which the tests check on every sample: the
order of attributes in the file is kept even where it isn't the order of their nodes. In
`Globals.lsf`, `Items/ItemFactory/Creators/Creator` and `Items/ItemFactory/Items/Item` list
every item in the same order: the creator holds the item's `Entity` identifier and template,
the item node its `Stats` (the game's name for it, such as `OBJ_Potion_Healing`), current
template and flags.

### The entity data (NewAge)

Most of what a player would want to edit isn't in the node tree. It is in one attribute of
`Globals.lsf`'s `NewAge` node: a memory image of the game's entity-component data, with the
signature `LSMF`. `newage.rs` reads and writes it.

- **Layout.** A 48-byte header, a data section, and a directory of tables at the end. Every
  offset in the data is relative to byte 48. A table is an array of fixed-size rows; where
  the game had a pointer, a row holds a 64-bit offset into the data section.
- **Two kinds of table.** A *component* table has a row per entity that has the component;
  the *owner map* (514 slots of 32 bytes) says which entity owns each row, as a list of
  entity indices kept in ascending order. A *data* table holds records that other rows point
  at, such as an inventory's slot records.
- **Entities.** Table 1, `core.v0.EntityId`, starts with the entity list. The rest of that
  table, and anywhere else, holds copies of identifiers that rows point at when they mean an
  entity: the game reads the sixteen bytes at the offset.
- **Loose data.** After the owner map come arrays, names and lists that rows point at with a
  pair of offsets, outside any table.
- **The checksum.** The header carries the first half of MurmurHash3 x64_128 over everything
  after it, both halves of the state seeded with `0xf0015eed5eedf001`. The game checks it: a
  save with a stale checksum loads to 100% and drops to the title screen with "Server
  shutting down". `NewAge` recomputes it whenever the blob is written out.

A character's data is reached as: template → `Creator.Entity` in the node tree → index in
the entity list → that entity's row in each table. `party.rs` names the tables and fields it
uses, with their layouts, in its `tables` module.

### Reading the party

`party.rs` reads a `Party`: the characters (whoever has experience is in the party; anyone
else with a character creation record, a level and ability scores is a companion waiting at
camp), their experience, level, entitled level, ability
points, gold and approval; Inspiration; camp supplies; and every inventory. An item's holder
is found by following its `MemberComponent` to the inventory it is in, the inventory's
`IsOwnedComponent` to its owner, and on up through bags until a character or the camp chest
is reached.

### Editing: the session

`session.rs` holds an open save. Each editable value is a `Field` (experience, a stack's
amount, an approval rating, how many of an item to add, and so on), with a range, a value as
the save has it and a value now. The changes are the fields that differ; undo and redo work
on steps, and quick repeated edits to one field merge into one step. Nothing is written
until export, which:

1. applies every changed field to a copy of the entity data, overwriting values in place;
2. adds the items asked for (below);
3. writes the entity data back into `Globals.lsf`, with a new checksum, and the document
   back into a copy of the package;
4. makes a quicksave or autosave a manual save (see below);
5. writes the package as `<prefix>__<name>/<name>.lsv`, with the screenshot copied as
   `<name>.WebP`;
6. reads the new save back and checks it: every change in place, every new item in the right
   inventory as the right item and amount, the party otherwise as edited, and every other
   file in the package unchanged.

### The interface

The app is immediate mode (egui): every frame draws everything from the session. Drawing
never changes the session; a click pushes an `Action`, and the actions are applied after the
frame. File dialogs and "show in folder" go through a `Platform` trait, which the tests
replace. The game's files are read on a background thread at start (`ui/game.rs`), and the
session is given the catalog when it arrives.

## What the game accepts

These are the rules for changing the entity data. Each one is in the "Tried in the game"
table in [FORMAT.md](FORMAT.md) with the test that showed it.

- **The checksum must match.** Any change, even to an unused byte, needs it recomputed.
- **Overwrite a value in place where the game keeps it.** Some values are worked out again on
  load and a change to them is discarded: ability totals (`StatsComponent`) and the camp
  supplies total. Change what they are worked out from instead: the points put into each
  ability at character creation, and the food stacks.
- **Experience doesn't level a character up.** The game offers a level-up when the entitled
  level (`AvailableLevelComponent`) is above the character level.
- **Nothing that is there may move.** Anything new is appended to the end of the data
  section and pointed at. A table that gains a row is copied whole to the end with the new
  row, and its directory record (and owner map slot) is pointed at the copy; the old copy is
  left where it was, so nothing pointing into it breaks.
- **Records of a type must stay inside their table.** A `MemberData` or slot record put in
  the loose data, outside its table, makes the load hang at 75%. To add one, the table is
  copied to the end with the new record, and every pointer into the old copy is moved by the
  same distance. That needs every pointer into the table to be known (see the pointer check).
- **Identifiers can be anywhere.** A pointer to an entity can point at any sixteen bytes that
  hold its identifier. Once the entity list has been moved to the end (the `core.v0.Level`
  row points at it), most rows of where it was are pointed at by nothing and can be reused
  for new identifiers.
- **A new entity needs both halves.** An item with only its node-tree entries crashes the
  game to the desktop; it needs its rows in the entity data too, and the reverse.
- **Owner lists stay in entity order.** New entities go at the end of the entity list, so
  their rows can be appended to each table without re-sorting.

## Adding items

### Why every new item is a copy

An item has around forty component rows, most of whose contents aren't documented, and an
entry in the node tree. Building one from nothing isn't possible with what is known. But the
game rebuilds most of an item from its template when a save loads: a copy of a dagger whose
template and game name are changed to a Longsword +1's loads as a working Longsword +1,
special effects included. So every new item is a copy of an item already in the save (the
*donor*), turned into the item asked for.

Weapons, armour and most objects have almost the same tables, and a donor of another kind
works: a Spear +1 made from an oil, the Haste Helm made from a quarterstaff, a Scroll of Haste
made from an oil.

### Choosing the donor

`Session::pattern_for` picks the closest donor in the whole save, from every item in the
item factory that is in an inventory and isn't a container (`items::donors`). It ranks them:

1. the same item, if anyone has it, so the copy needs no changing;
2. for a scroll, another scroll; else an item of the same kind and slot or category, then of
   the same kind, then any item;
3. within each, the holder's own before the party's, and the party's before anyone else's
   (NPCs and traders carry hundreds of items, many equipped);
4. an item that isn't equipped, stacked or a story item before one that is.

Only an item made from one of the game's own templates (`TemplateType` 0 in its creator) can
be made into another.

### Copying (`items::copy_into`)

The copy goes into the holder's main inventory, the one their `OwnerComponent` names, and
belongs to them; in the camp chest it belongs to nobody, as the chest's items do.

1. A new entity is added to the entity list, and its identifier written into a free row of
   the entity table. The inventory and the owner are already pointed at elsewhere; those
   references are shared, as the game's own items share them.
2. A `MemberData` record (the inventory and a slot number after the last) and a
   `ContainerSlotData` record (the item and a position after the last) are added inside their
   tables.
3. The inventory's two lists, of slot numbers and of slot records, are replaced by longer
   copies at the end of the data.
4. The owner's three lists of what it owns (`IsCurrentOwner`, `IsLatestOwner`,
   `IsOriginalOwner`) are replaced by longer copies.
5. Every row the donor has is copied for the new entity, except: its `MemberComponent` (a
   new one points at the new record), `WieldingComponent` (the copy isn't equipped),
   `IsStoryItemComponent`, its stack rows, and `WieldedComponent` when the owner changes. The
   sight row gets a new random identifier, and the ownership rows (`OwneeCurrent`,
   `OwneeHistory`) name the new owner.
6. The donor's creator and item nodes are copied with the new `Entity` and a new creator
   `UUID`, and the item's wares mark (bit 33 of its `Flags`) is cleared.

### Making the copy into another item (`items::make_into`)

The creator's `TemplateID`, the item node's `CurrentTemplate` and `Stats`, and the
`TemplateComponent` and `GameObjectVisualComponent` rows (the template identifier as text,
appended) are set to the new item's. The game fills in the rest from the template.

### Stacks

A stack is one item with an amount. The item has a `NewStackComponent` pointing at a `Stack`
record (a list of the stack's items, and a pair of offsets to its `StackEntry` records, each
an index and an amount) and a `StackMemberComponent` pointing at its own identifier. A new
stack gets a record and an entry of its own, added inside their tables like the inventory
records. An item stacks if its template's `maxStackAmount` is above 1.

### Scrolls

A scroll's spell is in records of its own: `SpellBookComponent` → `SpellData` → `SpellId` →
`MetaId`. Giving a copy a `SpellData` and `SpellId` of its own naming the new spell loads, but
the scroll has no description and can't be cast. What works is leaving the spell out: the copy
is made without its `SpellBookComponent` and `InventoryItemDataPopulatedComponent`, and the
game fills the spell in from the template when the save loads.

### The pointer check (`items::pointers_into`)

Adding a record means moving its table, and every pointer into the table has to move with
it. The editor knows where they are for each table it adds to (the `*_SOURCES` lists in
`items.rs`). Before moving one, it checks the whole save for any other eight bytes that read
as a pointer into the table, and refuses to add if it finds one, rather than leave it
pointing at the old copy. To tell real pointers from numbers that happen to look like one:

- inside tables, only whole eight-byte fields are looked at, and a field counts only if it
  reads as a pointer into the table in every row of its table;
- outside tables, only lists that some row points at are looked at (so stale copies of
  moved tables are ignored), skipping lists of slot numbers, and a list counts only if every
  entry reads as a pointer;
- the owner map, and pointers known to point into a neighbouring table (the end of one list
  can be the start of the next table), are left out.

Across the sample saves it finds one real unknown: a list of `MetaId` pointers in a level-up
record. The editor never adds to `MetaId`. `nautiloid-cli pointer-check <save>...` runs the
check for every table on each save given.

### The save type

`meta.lsf`'s `SaveGameType` is 0 for a manual save, 1 quick, 2 auto, 3 Honour Mode. The game
lists its own quick and autosaves as "Autosave 4 - Wilderness" and the like, whatever the
file is called, so exports made from one all look alike in the load menu. An export is
written as a manual save, and is then listed by its name. An Honour Mode save keeps its type.

## The item catalog

`catalog.rs` reads, from the player's `Data` folder (found through Steam's library list, or
chosen in the app):

- each module's root templates, `Public/<module>/RootTemplates/_merged.lsf`, in the order of
  `MODULES`, a later module's template replacing an earlier one's. A template's `Type`,
  `Stats`, `DisplayName`, `Description`, `Icon`, `StoryItem`, `maxStackAmount` and use-action
  `SkillID` (a scroll's spell) come from its parents through `ParentTemplateId` when it leaves
  them out;
- the item stats, `Public/<module>/Stats/Generated/Data/{Object,Armor,Weapon}.txt`, whose
  entries fill in what they leave out from the entry they name with `using`: the kind of
  item, rarity and category, and the main template of each game name (`RootTemplate`);
- the English text, `Localization/English/english.loca`, for the names and descriptions.

An item is a container, and can't be added, when the nearest `InventoryType` in its line of
templates isn't 0; with none in the line, when it has a bag's `ContainerAutoAddOnPickup` or
`ContainerContentFilterCondition`. Names don't decide it:

- Explosive barrels (Runepowder, Smokepowder, Oil, Acid) come from `CONT_Barrel_A` but set
  `InventoryType` to 0, and the ones the party carries in the sample saves have no inventory
  (`game.inventory.v0.OwnerComponent`).
- Throwable Spider Egg Sacs and Bags of Rotten Food are `CONT_..._CombatThrowable`, say
  nothing of `InventoryType`, and have no inventory.
- Across 46,000 items in the sample saves, `InventoryType` matched whether an item had an
  inventory for all but 70, mostly remains of broken crates. The Astral Prism ("Mysterious
  Artefact") is one exception: `InventoryType` 4, no inventory in any save. It stays refused,
  as a second Prism has no use and its quest might react to one. The Firewine Barrel is a
  container by this rule, and 31 of the 34 carried in the samples have an inventory.
- An item that stacks is a container only if it comes from `BASE_CONT`, the base template
  for containers. Only two items in the game stack and have an `InventoryType`: a Ribcage,
  from `BASE_CONT`, which bg3.wiki lists as a container; and Wood Bark, an ingredient that
  sets `InventoryType` 9 itself. The game gives Wood Bark an inventory more often than not,
  but there is nothing to open, and stacks split from it have none.
- bg3.wiki's Containers page agrees with this for every container whose template is in the
  catalog. The ones it calls static can't be picked up and aren't offered.

It reads only those files out of the packages, not the packages whole. The list offered for
adding is one item per game name that has an English name and can be picked up, leaving out
templates whose names start with `%%%`, which the game doesn't use. `CanBePickedUp` comes
from the templates like the rest: all 45,398 items carried in the sample saves have it, and
an item without it, such as the Bag of Rotten Food that's thrown in a fight, shows as a blank
slot in an inventory. Where several templates share a game name, one that can be picked up
is preferred. Items added by mods aren't in it: their
templates and text are in the mods' own packages, which the editor doesn't read yet.

## Testing

```bash
cargo test
NAUTILOID_SAVES="path/to/Savegames/Story" NAUTILOID_GAME_FILES="path/to/Baldurs Gate 3/Data" cargo test
```

- **Sample saves built in code.** `party::build` makes a small entity blob and `Globals.lsf`
  with a party, a trader with an equipped sword, stacks, bags, the camp chest and ownership;
  `saves::build` and `session::build` make whole save folders; `catalog::build` makes a game
  `Data` folder with packages. The unit tests and the interface tests use these, so the
  repository needs no real data.
- **Real saves and game files.** With `NAUTILOID_SAVES` pointed at a folder of save folders,
  `tests/real_saves.rs` checks that every save reads, writes back byte for byte, and gives
  the party the game's own summary gives; with `NAUTILOID_GAME_FILES` too,
  `tests/real_game.rs` adds items of every kind to every save but Honour Mode ones and
  checks the result. Without them these tests pass without checking anything, as they do in
  CI.
- **The interface.** `tests/ui.rs` runs the real app headless: it clicks, types and reads
  back what is drawn, and saves a picture of each screen to `target/ui-snapshots/`.
- **The user guide's pictures.** `tests/guide.rs` goes through the steps of
  [HOW-TO-USE.md](HOW-TO-USE.md) on the sample saves. To refresh the pictures after the
  interface changes: `NAUTILOID_GUIDE_IMAGES=1 cargo test --test guide`, then
  `python3 packaging/shrink-images.py docs/images/guide/*.png`.
- **The game.** None of this proves a save loads. Every kind of change was tried in the game
  first, from test saves the command-line tool writes, and the result recorded in
  FORMAT.md. A fork that changes how saves are written needs to do the same.

## Working things out with the command-line tool

```bash
cargo run --release --bin nautiloid-cli -- <command>
```

Run it without arguments for every command and its options. The ones used most:

| Command | For |
| --- | --- |
| `info`, `list`, `dump`, `find` | What a save holds; a document as text; finding a value |
| `diff <save> <save>` | What changed between two saves: made in the game before and after doing something, this is how most fields were found |
| `tables`, `rows`, `entity`, `at` | The entity data: its tables, a table's rows, every row an entity has, what an offset points at |
| `party`, `items` | The party and every inventory as the editor reads them |
| `donors`, `owners`, `pointers`, `pointer-check` | Research for adding items: every item and where it is, who owns what, what points into a table |
| `edit`, `set` | Write a copy with party values or raw 32-bit fields changed |
| `experiment` | Write a test save that tries one way of adding something |
| `add <save> <folder> <name> <holder> <item>[*N]...` | Add items exactly as the app does, and export |
| `survey`, `catalog-files`, `catalog` | What the game's packages hold, copying out the catalog's files for development, and the catalog itself |
| `check`, `rewrite` | Read every save and write it back, in memory or to a file, to check the round trip |

The usual way a field was found: save in the game, do one thing, save again, `diff` the two,
find the table with `tables` and `rows`, change it in a copy with `set`, and load that copy in
the game.

## When the game is patched

- `save.rs` only edits saves whose `meta.lsf` engine version is `SUPPORTED_ENGINE` (4.8, Patch
  8); others are listed but read only. Raise it only after the checks below.
- Table names carry versions (`game.inventory.v0.MemberComponent`). A patch can rename a
  table, change its row, or add tables that point into one the editor adds to. Run the real
  save tests and `pointer-check` on saves from the new build, and compare `tables` with an
  old save.
- New content can come in new modules: add them to `catalog::MODULES`.
- Then try each kind of change in the game again before trusting it.

## Building and releasing

The app needs Rust 1.88 or later: `cargo build --release`. CI (`.github/workflows/ci.yml`)
checks formatting, lints with warnings as errors, and runs the tests on every push. It builds
the app for Windows, Linux and macOS for `main`, for version tags, when run by hand, and for
any push whose commit message contains `[build]`, and attaches the builds to the run.
`packaging/make-icons.sh` makes the icons from `assets/icon/source.webp`.

Every download includes two more files, both made by CI's `extras` job on every push:

- `How to use Nautiloid.pdf`, the user guide. `packaging/make-guide.py` turns
  `docs/HOW-TO-USE.md` into one HTML page, with the pictures and the app's fonts inside it,
  and prints that with Chrome. To make it locally: `pip install markdown==3.7`, then
  `python3 packaging/make-guide.py <folder>`, with `--browser <path>` if Chrome or Chromium
  isn't on the path. The guide's text shouldn't say where the app is hosted: it goes out with
  downloads from wherever the app is published.
- `THIRD-PARTY-LICENSES.txt`, the licences of the libraries and fonts built into the
  programs. `packaging/make-notices.sh` writes it with `cargo-about` (`about.toml` and
  `packaging/third-party.hbs` say what it collects), adding the two font notices egui's
  default fonts carry beyond their declared licences. A new dependency under a licence not in
  `about.toml`'s list stops it, so the list is checked whenever dependencies change.

## Not done

- Names for items that mods add.
- Converting an Honour Mode save to another difficulty (a stretch goal in SCOPE.md).
- The macOS build hasn't been run on a Mac.
- A level-up or a change of gold for a companion at camp is the same field as a party
  member's but hasn't been tried in the game.
