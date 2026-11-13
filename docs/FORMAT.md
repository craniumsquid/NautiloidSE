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
| `game.experience.v0.ExperienceComponent` | 12 bytes | Total experience, twice: u32 at 0 and at 4. Only party members have a row |
| `game.stats.v0.LevelComponent` | 4 bytes | Character level |
| `game.stats.v3.StatsComponent` | 36 bytes | Strength to Charisma as six u32 from offset 8. Whether these are base scores or totals with bonuses is not known |
| `game.camp.v0.TotalSuppliesComponent` | 4 bytes | Camp supplies total. One row |

Tables that look like the remaining Phase 2 targets, not yet decoded:

- `game.approval.v0.Ratings` (48 bytes, six offsets): approval. Its values are in data the
  row points at.
- `game.inventory.v0.Stack`, `StackEntry`, `NewStackComponent`, `StackMemberComponent`,
  `ContainerComponent`, `ContainerSlotData`, `MemberComponent`: inventories and stacks.
- `game.action_resources.v1.Component`: action resources, which may include Inspiration.
- `game.stats.v0.ClassesComponent`, `game.progression.v3.LevelUpComponent`,
  `game.character_creation.*`: class levels and level-up choices, which a level change may
  have to agree with.
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

Every failure looks the same: the save loads to 100%, drops to the title screen, and shows
"Server shutting down". Other saves load normally afterwards.

So the game accepts this code's LZ4 and Zstandard output, its package layout and its export
folders, and it refuses any entity data whose checksum does not match. The checksum function
above predicts each of these results.

## Not yet known

- **Whether an edit with a correct checksum loads and takes effect.** Nothing else is known
  to stand in the way, but it has not been tried.
- Whether the game checks the package's MD5 or the per-mod MD5s on load.
- Whether an exported copy should also get its own name inside the package, in
  `SaveInfo.json` and the screenshot entry. It loads without.
- What level, experience and `SaveInfo.json` have to agree on for the game to be content.
- The layout of tables on Patch 4 and 5. The buffer parses, but those saves stay read-only.
