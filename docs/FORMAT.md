# Save format notes

What is known about Baldur's Gate 3 saves, from eleven sample saves: seven from the current
build (`4.1.1.7631656`), one of them Honour Mode; one from the Patch 8 release build
(`4.1.1.6758295`); one each from Patch 4 and Patch 5. Everything here is checked by the tests
in `tests/real_saves.rs` unless it says otherwise. The byte-level layouts are in the module
comments of `src/lspk.rs`, `src/lsf.rs` and `src/newage.rs`.

## The save folder

A save is a folder holding `<SaveName>.lsv` and `<SaveName>.WebP`.

- A normal save's folder is `<Leader>-<digits>__<SaveName>`, for example
  `Nelthryiis-3912126141017__Shattered Sanctum - 8h 02m`.
- The Honour Mode save's folder is `<GameSessionID>__HonourMode`, where the first part is the
  `GameSessionID` from `meta.lsf`.
- The game's load menu lists a save under its folder's name, the part after `__`, so a copy
  of a save folder under another name appears as a second save.
- The `.lsv` and `.WebP` inside must be named after the folder's save name. With the files
  still named after the original, the entry shows no screenshot and loading hangs at 0%; with
  them renamed to match, the copy loads.
- The names inside the package do not have to match: the copy that loads still has the
  original's screenshot entry and `Save Name`.

## The package (`.lsv`)

A Larian package, version 18, in one part: a 40-byte header, the files, then the file list.

- Each file is followed by `0xAD` bytes up to the next multiple of 64, counted from the end of
  the header.
- The file list is an LZ4 block of 272-byte entries. Its order is not the order of the files
  on disk.
- The header carries an MD5 of every file's uncompressed contents, taken in on-disk order,
  with one added to each byte of the digest.
- Patch 4 and 5 store each file with zlib; Patch 8 uses Zstandard, one frame per file with the
  content size in the frame header and no checksum.

A current save holds:

| File | What it is | Uncompressed size |
| --- | --- | --- |
| `meta.lsf` | Game version, mod list, difficulty and rules, party icons, camera | 3 to 9 KB |
| `SaveInfo.json` | The party's classes, level, experience, origin and race; game version; current level; difficulty unless the rules are custom | under 3 KB |
| `Globals.lsf` | Characters, items and global state, including the party's entity data | 2 to 5 MB |
| `LevelCache/<level>.lsf` | One per visited level, same structure | up to 23 MB |
| `StorySave.bin` | Story-script state. Not read; copied unchanged | 33 to 76 MB |
| `<SaveName>.WebP` | The same screenshot that sits next to the `.lsv` | 9 to 190 KB |

## Documents (`.lsf`)

A tree of named nodes carrying typed attributes, format version 7. One version 6 file turned
up, a level cache left over inside the Patch 5 save; it has the same layout.

- Five sections: strings (the name table), nodes, attributes, values, keys. The header lists
  their sizes in the order strings, keys, nodes, attributes, values, but they sit in the file
  as strings, nodes, attributes, values, keys.
- Strings is one LZ4 block. The others are LZ4 frames with linked 64 KB blocks and no
  checksums.
- Nodes are in depth-first order; a parent always comes before its children.
- Attributes are in the order the game wrote them, not node order. A node's attributes can
  come after its children's and be interleaved with them, so the order has to be kept.
- Keys are present only in some level caches. Each names the attribute that keys a node in a
  map; all seen are `MapKey`.
- The engine version in the header is `4.<patch>`: 4.4, 4.5 and 4.8 in the samples.

Value types found: Byte, Short, UShort, Int, UInt, Float, Double, IVec2, Vec2, Vec3, Vec4,
Bool, String, FixedString, LSString, ULongLong, ScratchBuffer, Int8, TranslatedString, UUID and
Int64.

- Text is UTF-8 with one terminating NUL.
- A TranslatedString is a u16 version, an i32 length and the handle text.
- A UUID is stored with its first three groups little-endian and each later pair of bytes
  swapped.
- A Bool is one byte, and not always 0 or 1: `HotbarLocked` in `meta.lsf` has held 30 and 143.

### `meta.lsf`

Everything is under `MetaData/MetaData`:

- `GameVersions/GameVersion`: the game version text.
- `ModuleSettings/Mods/ModuleShortDesc`: one per mod, with `Folder`, `Name`, `UUID`, `MD5`,
  `Version64` and `PublishHandle`. All eleven samples are modded.
- `Difficulty` (a byte), `Rulesets` (UUIDs; present when a preset is in use, the Honour save
  has two) and `CustomRulesetValues` (filled in when the rules are custom).
- `SaveGameType`: 0 for a manual save, 2 for an autosave, 3 for the Honour Mode save.
- `LeaderName`, `GameID`, `GameSessionID`, `SaveTime`, `TimeStamp`, `Seed`,
  `DisabledSingleSave`, `DishonorDifficultySelection`, `Modded`, `HasUnofficialMods`,
  `Crossplay`, `OriginalPlatform`.
- `PartyMetaData/CharacterMetaData`: an icon and a name handle per party member.

### `Globals.lsf`

Top-level nodes include `Characters`, `Items`, `Journal`, `Story`, `Stats`, `GameControl`,
`AnubisFramework` and `NewAge`.

- `Characters/CharacterFactory/Creators/Creator` lists every character: `Entity` (a UUID) and
  `TemplateID`. `Characters/CharacterFactory/Characters/Character` has the same characters in
  the same order. `Items/ItemFactory` is laid out the same way.
- A companion's `TemplateID` is fixed: Shadowheart is `3ed74f06-3c60-42dc-83f6-f034cb47c679`,
  Astarion `c7c13742-bacd-460a-8f65-f864fe41f255`, Karlach
  `2c76687d-93a2-477b-8b18-8a14b549304c`, Gale `ad9af97d-75da-406a-ae13-7071c563f604`.

## Entity data (`NewAge`)

`Globals.lsf` and every level cache have a top-level `NewAge` node whose one attribute is a
buffer of 60 KB to 41 MB. This is where the game keeps component data: the party's level,
experience and ability scores, approval, camp supplies, inventories and stacks are here, not
in the node tree.

The buffer is a memory image with the signature `LSMF`: a header, a data section, and a
directory that names every table in it (339 in a mid-Act 1 `Globals.lsf`). A table is an
array of fixed-size rows; where the game had a pointer, the row holds a 64-bit offset into
the data section.

- The header carries a checksum of everything after it: the first half of MurmurHash3
  x64_128, with both halves of the state seeded with `0xf0015eed5eedf001`. The game checks
  it. It holds for all 45 buffers in the samples, Patch 4 and 5 included. The function and
  seed come from the notes in the [SaveForge](https://github.com/danaketh/SaveForge)
  project's `newage.rs`.
- Unused bytes (the first eight of the data section, padding in table records, the first
  byte of the names) hold whatever was in memory. They count towards the checksum.
- `core.v0.EntityId` starts with the entity list, 16 bytes per entity. These are the `Entity`
  values from the node tree's `Creator` nodes. The rest of that table is identifiers that
  other rows point at, so the same identifier repeats there.
- An owner map says, for each component table, which entity each row belongs to.
- So a party member's data is reached as: template → `Creator.Entity` → index in the entity
  list → row of a table.

Fields located so far. Level and experience match `SaveInfo.json` for every party member in
every Patch 8 sample:

| Table | Row | What |
| --- | --- | --- |
| `game.experience.v0.ExperienceComponent` | 12 bytes | Total experience, twice: u32 at 0 and at 4. Only party members have a row. **Editable**: the game takes the new total, and shows it as experience into the current level |
| `game.stats.v0.LevelComponent` | 4 bytes | Character level |
| `game.stats.v3.StatsComponent` | 36 bytes | Strength to Charisma as six u32 from offset 8. These are totals with every bonus applied (an item that sets a score shows here). **Recalculated on load**: a change here is discarded |
| `game.camp.v0.TotalSuppliesComponent` | 4 bytes | Camp supplies total. One row. **Recalculated on load**: a change here is discarded |
| `game.character_creation.v1.CharacterCreationStatsComponent` | 88 bytes | Two identifiers (probably race and subrace), the character's name (offset and length at 56 and 64), and at 72 a pair of offsets to seven u32: the points put into each ability above the base of 8, with Strength at index 1. Seven characters have a row in the sample, the four party members among them. **Editable**: the game works ability scores out from these on load, and does not hold them to the usual limit of 7 |
| `game.experience.v0.AvailableLevelComponent` | 4 bytes | Equal to the character level for every character in the samples. Probably the level the character is entitled to: experience raised past a threshold on its own does not make the game offer a level-up |
| `game.inventory.v0.NewStackComponent` | 8 bytes | One per stack of items: an offset to a row of `game.inventory.v0.Stack`. That row is two pairs of offsets: the stack's items (offsets to their entity identifiers) and its entries (rows of `StackEntry`: a u16 index into the items, two unused bytes, a u32 amount). Gold is a stack whose item has the template `1c3c9c74-34a1-4685-989e-410dc080be6f`. Each character or container holding gold has its own stack, and the game's top bar shows the party's sum. In the sample all of the party's gold is on one character, with nine more gold stacks on other entities |
| `game.inventory.v0.MemberComponent` | 8 bytes | On an item: an offset to a row of `MemberData`, which is an offset to the entity identifier of the inventory the item is in, then its slot number |
| `game.inventory.v1.IsOwnedComponent` | 8 bytes | On an inventory: an offset to the entity identifier of its owner. An item inside a bag is in the bag's inventory, whose owner is the bag, so finding the character means following this up |
| `game.action_resources.v1.Component` | 16 bytes | A pair of offsets to 64-byte entries: a resource identifier, a level, then the amount and the maximum as 64-bit floats. Action points, movement, spell slots and class resources appear here. The party has its own entity, the one that owns the `game.party.*` rows, and it holds three more: `c38dea32-f5cb-4e38-a341-eb3fa653fb3e` matches the Inspiration count the game shows, and `a24ca5e2-01e1-48fd-a4c8-79b8817f0a18` is 2 of 2, probably short rests |

Many rows hold a pair of offsets like these, begin and end, to an array elsewhere in the data
section. The level-up tables (`game.character_creation.v3.LevelUpComponent` and
`game.progression.v3.LevelUpComponent`) point at one `LevelUpComponentData` row per level
gained, each with identifiers for the class, subclass and feat taken.

Tables that look like the remaining Phase 2 targets, not yet decoded:

- `game.approval.v0.Ratings` (48 bytes, six offsets): approval. Its values are in data the
  row points at.
- `game.inventory.v0.Stack`, `StackEntry`, `NewStackComponent`, `StackMemberComponent`,
  `ContainerComponent`, `ContainerSlotData`, `MemberComponent`: inventories and stacks.
- `game.stats.v0.ClassesComponent` and the selector tables under `game.character_creation.*`:
  class levels and the choices made at each level-up, which a level change may have to agree
  with.
- `game.party.v0.MemberComponent`, `game.camp.v1.ChestComponent`, `game.v0.PlayerComponent`.

## Tried in the game

All on build `4.1.1.7631656`, with copies of one manual save.

| Change | Result |
| --- | --- |
| None, but every document parsed, written and compressed again by this code | Loads |
| A byte-for-byte copy in a folder with another save name, files inside not renamed | Listed without its screenshot; loading hangs at 0% |
| The same copy with the files inside renamed to match the folder | Loads |
| None, exported by this code as a new folder with matching file names | Loads |
| Every party member's experience set to 6000 in the entity data, checksum left as it was | Fails |
| One bit of the entity data's checksum flipped, nothing else | Fails |
| The checksum set to zero, nothing else | Fails |
| One unused byte at the start of the data section changed | Fails |
| One unused byte at the start of the directory changed | Fails |
| One character's experience raised by one, checksum left as it was or set to zero | Fails |
| Every party member's experience set to 6000, checksum recomputed | Loads. Each shows 2750, which is 6000 less the 3250 at which level 5 starts in that game |
| One character's experience set to 7100, another's Strength total 18 → 20, camp supplies total 660 → 700, checksum recomputed | Loads. Experience shows 600 and no level-up is offered; Strength is still 18 and camp supplies still 660 |
| Points allocated to Strength at character creation: 0 → 7 for one character, 7 → 9 for another | Loads. Strength shows 15 (was 8) and 20 (was 18) |

Every failure looks the same: the save loads to 100%, drops to the title screen, and shows
"Server shutting down". Other saves load normally afterwards.

So the game accepts this code's LZ4 and Zstandard output, its package layout and its export
folders, and it refuses any entity data whose checksum does not match. The checksum function
above predicts each of these results. With the checksum right, an edit loads; whether it
sticks depends on whether the game keeps the value or works it out again from something else.

## Not yet known

- **Whether changes to the level a character is entitled to, the party's Inspiration and a
  gold stack's amount stick.** Each has been located by matching the number the game shows;
  a test of all three is out.
- What sets the experience thresholds. The 7100 test shows 600, which fits a level 6
  threshold of 6500 in that modded game; an unmodded game's is 14000.
- Where the bonuses on top of the allocated points are recorded (the +2 and +1 chosen at
  creation, and ability improvements taken at level-up).
- Where camp supplies come from. The total is recalculated, presumably from the food items.
- Whether the game checks the package's MD5 or the per-mod MD5s on load.
- Whether an exported copy should also get its own name inside the package, in
  `SaveInfo.json` and the screenshot entry. It loads without.
- What level, experience and `SaveInfo.json` have to agree on for the game to be content.
- The layout of tables on Patch 4 and 5. The buffer parses, but those saves stay read-only.
