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
  `Version64` and `PublishHandle`. All the samples are modded. `Globals.lsf` and every level
  cache carry the same list, under a `ModuleSettings/ModuleSettings/Mods` root. The editor
  only reads the list.
- The mods the game has active are in the profile's `modsettings.lsx`, beside its
  `Savegames` folder: `PlayerProfiles/<profile>/modsettings.lsx`, an XML file of
  `ModuleShortDesc` nodes. The editor reads only each entry's `UUID`, or its `Folder` if it
  has none.
- `Difficulty` (a byte), `Rulesets` (UUIDs; present when a preset is in use, the Honour save
  has two) and `CustomRulesetValues` (filled in when the rules are custom).
- `SaveGameType`: 0 for a manual save, 2 for an autosave, 3 for the Honour Mode save.
- `GameSessionID`: the playthrough. Every save of one campaign has the same one, and the
  editor groups saves by it. `GameID` is not that: three saves of one playthrough in the
  samples share a `GameSessionID` but have two `GameID`s between them, the two made minutes
  apart sharing one.
- `LeaderName`, `SaveTime`, `TimeStamp`, `Seed`, `DisabledSingleSave`,
  `DishonorDifficultySelection`, `Modded`, `HasUnofficialMods`, `Crossplay`,
  `OriginalPlatform`.
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

- `VersionHash/Value` is 32 characters of hexadecimal. In one sample it is 31 characters
  followed by a NUL, before the usual terminating NUL: text can hold a NUL inside.

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
| `game.experience.v0.AvailableLevelComponent` | 4 bytes | The level the character is entitled to. Equal to the character level in every sample. **Editable**: set one above the level, the game offers a level-up. Raising experience past a threshold does not do this on its own |
| `game.inventory.v0.NewStackComponent` | 8 bytes | One per stack of items: an offset to a row of `game.inventory.v0.Stack`. That row is two pairs of offsets: the stack's items (offsets to their entity identifiers) and its entries (rows of `StackEntry`: a u16 index into the items, two unused bytes, a u32 amount). Gold is a stack whose item has the template `1c3c9c74-34a1-4685-989e-410dc080be6f`. Each character or container holding gold has its own stack, and the game's top bar shows the party's sum. In the sample all of the party's gold is on one character, with nine more gold stacks on other entities |
| `game.inventory.v0.MemberComponent` | 8 bytes | On an item: an offset to a row of `MemberData`, which is an offset to the entity identifier of the inventory the item is in, then its slot number |
| `game.inventory.v1.IsOwnedComponent` | 8 bytes | On an inventory: an offset to the entity identifier of its owner. An item inside a bag is in the bag's inventory, whose owner is the bag, so finding the character means following this up |
| `game.action_resources.v1.Component` | 16 bytes | A pair of offsets to 64-byte entries: a resource identifier, a level, then the amount and the maximum as 64-bit floats. Action points, movement, spell slots and class resources appear here. The party has its own entity, the one that owns the `game.party.*` rows, and it holds three more: `c38dea32-f5cb-4e38-a341-eb3fa653fb3e` matches the Inspiration count the game shows, and `a24ca5e2-01e1-48fd-a4c8-79b8817f0a18` is 2 of 2, probably short rests |

### Items

- An item is in an inventory (`MemberComponent`), the inventory has an owner
  (`IsOwnedComponent`), and a bag is an item with an inventory of its own, so the holder is
  found by following owners up. The camp chest is the one entity with a
  `game.camp.v1.ChestComponent` row.
- The game's name for an item (`OBJ_Potion_Healing`, `CONS_FOOD_Fruit_Apple`) is the `Stats`
  attribute of its node under `Items/ItemFactory/Items`, which lists the items in the same
  order as `Items/ItemFactory/Creators`. The names the game shows are in its language
  files, not in the save.
- Only a stack has an amount stored, in its `StackEntry`, the same field as a gold stack's.
  A stack can hold 1. An item with no stack is one of its kind and has no amount to change.
- Food carries `game.camp.v0.SupplyComponent` (4 bytes): the camp supplies one of it is
  worth. Adding up worth times amount over the party, the companions at camp and the camp
  chest gives the stored total exactly in two samples (165 and 660) and misses in two
  others (773 against 782, 217 against 177), so the game's rule is close to this but not
  this.

The editor changes the amount of any stack. Tried in the game for gold, a stack of
potions and a stack of food: the amounts hold, and the camp supplies total follows the food
(see Tried in the game).

### Approval

`game.approval.v0.Ratings` has a 48-byte row on each of the ten possible companions, met or
not. It is three pairs of offsets, begin and end:

1. A list of 8-byte offsets to entity identifiers: the characters approved of. In a game for
   one player there is one, the player's character.
2. An `i32` rating for each of those.
3. A list of 16-byte identifiers that grows by one each time the rating moves: what has
   already moved it, so that nothing counts twice.

On a companion with no rating yet, every field is all ones.

Found from two saves either side of one conversation with Shadowheart: her rating went from
10 to 11 and the third list from one identifier to two. The editor changes the rating and
leaves the third list alone. Tried in the game: a rating set to 50 shows as "High (50)" on
the character sheet and is still 50 in the next save the game writes. The range of −50 to
100 is the game's as generally reported, not something read from a save.

Not every party member has all of this. A character played as an origin has an empty name
in the character creation record (Gale, in the Honour Mode sample), and a game that has only
just begun has no Inspiration resource yet.

### Who is a character

- The party is whoever owns a `game.experience.v0.ExperienceComponent` row. They also own
  `game.party.v0.MemberComponent` and `game.v0.PlayerComponent`.
- A recruited companion who is not in the party keeps a character creation record, a level,
  an entitled level, ability scores, an inventory and `game.recruit.v0.RecruitedByComponent`,
  but has **no experience row**: the game keeps experience only for the party, and awards a
  companion who joins enough to match it (it shows the award on screen). On Patch 8
  they also own `game.camp.v1.PresenceComponent`. The 8-hour sample has three (Wyll, Lae'zel
  and Gale); a Patch 4 sample has one at level 1 entitled to level 12.
- So the editor's characters are the experience owners, then every other owner of a
  character creation record that also has a level, an entitled level and ability scores.
- The player's own character has no `RecruitedByComponent`. Its template is not a mark of
  origin: a Dark Urge Drow and custom Drow characters share one, and a Dark Urge Dragonborn
  has another, so it follows race or body.

Many rows hold a pair of offsets like these, begin and end, to an array elsewhere in the data
section. The level-up tables (`game.character_creation.v3.LevelUpComponent` and
`game.progression.v3.LevelUpComponent`) point at one `LevelUpComponentData` row per level
gained, each with identifiers for the class, subclass and feat taken.

Tables that look like the remaining Phase 2 targets, not yet decoded:

- `game.inventory.v0.StackMemberComponent`, `ContainerComponent`, `ContainerSlotData`: the
  rest of inventories and stacks, which adding an item will need.
- `game.stats.v0.ClassesComponent` and the selector tables under `game.character_creation.*`:
  class levels and the choices made at each level-up, which a level change may have to agree
  with.
- `game.party.v0.MemberComponent`, `game.camp.v1.ChestComponent`, `game.v0.PlayerComponent`.

## Adding an item (research)

What adding an item would have to touch, from an unstacked scroll in a character's pack:

- **Where things sit in the entity data.** The data section is the `core.v0.Level` row,
  the entity table (entity list first, then copies of identifiers that rows point at), the
  tables, the owner lists, the owner map, and then about 750 KB of loose data that rows point
  into: names, arrays, approval lists, inventory arrays. Every owner list is sorted by
  entity index; the entity list is not sorted.
- **The node tree.** A `Creator` under `Items/ItemFactory/Creators` (`Entity`,
  `TemplateType`, `TemplateID`, and for `TemplateType` 0 a `UUID` of its own that appears
  nowhere else) and an `Item` under `Items/ItemFactory/Items` in the same position
  (`Translate`, `Flags`, `Level`, `Global`, `CurrentTemplate`, `Stats` and three empty
  child nodes).
- **Its own rows.** About forty component rows. Most can be shared with another item of the
  same kind: their offsets point at template strings and arrays that are only read. Two
  differ per item: `game.inventory.v0.MemberComponent` (an offset to a `MemberData`: an
  offset to the inventory's identifier, then the slot number) and
  `game.sight.v0.DataComponent` (sixteen bytes that look like an identifier).
- **Its inventory.** `game.inventory.v1.ContainerComponent`: two pairs of offsets, to the
  slot numbers (u16) and to offsets of `ContainerSlotData` (an offset to the item's
  identifier, a u32 position, four bytes left over).
- **Its owner.** The item's `game.v0.OwneeCurrentComponent` points at the owner, and the
  owner's `IsCurrentOwner`, `IsLatestOwner` and `IsOriginalOwner` components each hold a
  list of offsets to what it owns. The scroll was also on two hotbar slots.
- **Not an item.** A character can be carried too: one sample has a corpse in Karlach's
  pack, with a character creator and no `game.item.v0.ItemComponent`.

The editor adds by appending: a new entity list with the new identifier at the end, each
table that gains a row copied whole to the end with its owner list, and new arrays at the
end, pointed at. Nothing already in the data moves, so every existing offset still reads
the same bytes.

The first round of tests (see Tried in the game) showed that the entity list and a whole
table can be moved, but that records rows point at must stay inside their tables: a copy
whose `MemberData`, slot record and identifier were in the loose data hung the load, and so
did moving just the scroll's records there. So the second version keeps them in:

- `MemberData` and `ContainerSlotData` have no spare rows (each row is pointed at exactly
  once: every `MemberData` from the start of a `MemberComponent` row, every slot record from
  an inventory's list). Each table is copied to the end with the new record, and every one
  of those pointers is moved by the same distance.
- The entity table has no spare rows either, but once the entity list has been moved, most
  of the rows where it was are pointed at by nothing (about 2,200 of 2,300 in one sample).
  The copy's identifier goes in one of those.

Checked on the four saves with the biggest inventories (Patch 4, 5 and 8, up to 1,272 items
in one pack): every `MemberData` is pointed at from a `MemberComponent` row and every slot
record from an inventory's list, most of the entity list is pointed at by nothing (2,500 to
5,000 rows), and every owner list is sorted. Slot numbers have gaps in most inventories and
the position counter runs well past the number of items, but no two items share a position,
so the next slot and position after the highest are what the game would use.

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
| One character's experience set to 7100 and entitled level 5 → 6; the party's Inspiration 6 → 3; that character's gold stack 3,385 → 5,000 | Loads. A level-up is offered, Inspiration shows 3, and gold shows 5,000 on that character and in the party total |
| Made in the app on Windows, on a Dark Urge character in another campaign: points allocated to Strength 0 → 2, Intelligence 0 → 2 and Wisdom 2 → 8; gold stack 429 → 1,429 | Loads with the changes in place. The file is byte for byte what this code writes on Linux for the same changes |
| The entity list copied to the end of the data, unchanged, and `core.v0.Level` pointed at the copy | Loads |
| The experience table and its owner list copied to the end, the directory and owner map pointed at the copies, and one character's experience +100 in the copy | Loads, with the new experience |
| A second copy of a scroll in the same pack, its `MemberData`, slot record and identifier in the loose data at the end | Hangs at 75% |
| The scroll's own `MemberData`, slot record and identifier copied to the loose data and pointed at, nothing else | Hangs at 75% |
| The scroll's `Creator` and `Item` nodes copied, with nothing in the entity data | Crashes to the desktop |
| Made in the app: two companions' approval 0 → 50; a companion at camp's Strength points 0 → 18; a stack of healing potions 1 → 21; a stack of fish in the camp chest, worth 4 camp supplies each, 2 → 14 | Loads. The character sheet shows approval as "High (50)". Camp supplies show 335, up from 287 by the 48 the twelve fish are worth. After the companion is swapped into the party and the game saves again, that save has approval at 50, the stacks at 21 and 14, the companion's Strength at 26 with the 18 points kept, and experience awarded to them to match the party |

Every failure looks the same: the save loads to 100%, drops to the title screen, and shows
"Server shutting down". Other saves load normally afterwards.

So the game accepts this code's LZ4 and Zstandard output, its package layout and its export
folders, and it refuses any entity data whose checksum does not match. The checksum function
above predicts each of these results. With the checksum right, an edit loads; whether it
sticks depends on whether the game keeps the value or works it out again from something else.

## Not yet known

- What sets the experience thresholds. The 7100 test shows 600, which fits a level 6
  threshold of 6500 in that modded game; an unmodded game's is 14000.
- Where the bonuses on top of the allocated points are recorded (the +2 and +1 chosen at
  creation, and ability improvements taken at level-up).
- Exactly which food the camp supplies total counts. See Items.
- Whether a companion at camp keeps a change to their entitled level or gold when they join
  the party. Ability points do (see Tried in the game); the others are the same fields as
  a party member's but have not been tried on a companion at camp.
- The names of the approval levels and where each begins. The game calls 50 "High".
- How to give gold to a character who carries none. There is no stack to change, so it means
  creating an item.
- Whether the game checks the package's MD5 or the per-mod MD5s on load.
- Whether an exported copy should also get its own name inside the package, in
  `SaveInfo.json` and the screenshot entry. It loads without.
- What level, experience and `SaveInfo.json` have to agree on for the game to be content.
- The layout of tables on Patch 4 and 5. The buffer parses, but those saves stay read-only.
