# Scope

Nautiloid is a save editor for Baldur's Gate 3, written from scratch in Rust with egui: a save
browser, a sidebar of tabs, a Changes panel where every change has its own Revert, undo and
redo, and an export that always writes a new save. No code comes from other save editors.
Nautiloid is GPL-3.0-only.

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

### Phase 1: the app, its save browser and the Save info tab

Built (see Status).

- Save browser grouped by campaign, with thumbnails, game version, party and a count of mods.
- Application shell: header, sidebar, notices, dialogs, Changes panel, undo and redo, and the
  export of a new save.
- Save info tab. The read-only view of inventories planned for this phase is left to the
  Inventory step below, where their tables have to be worked out anyway.

### Phase 2: edits, in this order

1. **Party.** Built. Per character: experience, level-ups, ability scores and gold.
   Party-wide: Inspiration. Companions waiting at camp are listed too; they have no
   experience to change, since the game awards it when they join.
   - A level-up is granted, not applied: the editor entitles the character to the next level
     and the game takes the player through the choices.
   - Ability scores are changed through the points allocated to each ability; the game adds
     the character's other bonuses on top.
   - Gold is per character. A character who carries none can't be given any: gold is a
     stack, and stacks can't be added yet.
2. **Inventory.** Built. Stack quantities, for each character and the camp chest. Camp
   supplies belong here: the game works the total out from the food items. Items have the
   names the game shows, read from the player's own install, or else their internal names,
   tidied.
3. **Approval.** Built. Each companion's approval of the player's character.
4. **Mods.** Built, read only. The save's list of required mods, compared with the mods the
   game has active. Removing a mod from a save was built and then dropped: the game's own
   mod manager handles that well enough.

### Phase 3: adding items

Built. The Inventory tab's **Add items…** searches the game's items and adds them to a
character or the camp chest, up to 20 of each per export.

- The list of items is read from the player's own install of the game when the app starts:
  the root templates, the item stats and the English language file. Nothing from the game is
  shipped. The app finds a Steam install by itself, or asks for the game's folder.
- A new item is a copy of the closest item anywhere in the save, made into the chosen item by
  its template and stats: the same item if anyone has one, then an item of the same kind and
  slot, of the same kind, or any item; the holder's own before the party's, and the party's
  before anyone else's. An equipped, stacked or story item can be copied: the copy is none of
  those, and belongs to the holder (to nobody in the camp chest). Containers are not copied.
- A scroll keeps its spell in records of its own. A scroll is made from another scroll if
  the save has one, copied without those records, and the game fills the new spell in from
  the template. Giving the copy a spell record of its own was tried and doesn't work: no
  description, and it can't be cast.
- An item that stacks (by the template's `maxStackAmount`) goes in as one stack of as many as
  asked for, up to what a stack holds. A single item's count can't be changed, so the
  Inventory tab puts a + beside it to add more.
- A new item is not marked as wares (a bit in its node's `Flags`), whatever it was copied
  from.
- Items the game marks as story items are offered with a warning that a quest may count on
  there being only one.
- An exported quicksave or autosave is written as a manual save. The game lists its own
  quicksaves and autosaves as "Autosave 4 - Wilderness" and the like, whatever the file is
  called, so eleven test saves made from one autosave all looked alike in its load menu; as
  manual saves they are listed by their names.
- Before any table of records is moved to add to it, the editor checks the save for pointers
  into that table it doesn't know about, and refuses rather than leave them behind.
- Not done: items from mods, whose names are in the mods' own files.

### Stretch goal

- **Difficulty.** Converting an Honour Mode save to Custom or Balanced, and rescuing a failed
  Honour run. One sample is an Honour Mode save, at the start of a game. Until this is built,
  an Honour Mode save opens and exports like any other. The export, a second save of the same
  playthrough, loads from the game's main menu with its changes; the game won't load it while
  a game is running, as Honour Mode allows one save per playthrough. Which of the two the game
  saves over next hasn't been checked. The export dialog says to load from the main menu.

## Out of scope

- Global and story flags, and anything else in the story-script state (`StorySave.bin`). The
  file is carried through unchanged.
- Appearance. The game's Magic Mirror covers it.
- Class, subclass and multiclass changes. Withers' respec covers it.
- Forcing story outcomes or reviving story characters.
- Console saves.

## Status

Phase 1 and the Party tab are built. The app lists saves by campaign, opens a Patch 8 save,
changes the party's values with every change listed and revertible, and exports a new save
beside the original.

- The app's export, given the same changes, writes a save byte for byte the same as one made
  by the command-line tool that loaded in the game with every change in place (experience, a
  level-up, gold and Inspiration together).
- The interface is tested without a window: tests click through the app, check what it does
  and draw each screen. The app has also been run on a virtual Linux display against the
  sample saves. It has not yet been run on Windows or macOS, or by anyone but its tests.
- Builds for Windows and Linux are made by CI for `main`; macOS for version tags and on
  request.

Since then: companions at camp, the Inventory tab and the Approval tab. What has and has
not been tried in the game:

- Tried, on two campaigns and from the Windows app: experience, level-ups, ability points,
  gold and Inspiration for a party member; approval; the amount of a stack of potions and of
  a stack of food, with the camp supplies total following the food; and ability points for
  a companion at camp, which were kept when that companion joined the party.
- Not yet tried: a level-up or a change of gold for a companion at camp. Both are the same
  fields as a party member's.

With the Mods tab the four Phase 2 tabs are done, and Phase 3, adding items, is built.
Items added by the command-line tool, which adds them the same way the app does, loaded in
the game at the end of the inventory: plain copies of a scroll and other items, and items
made from others (a longsword +1 and a dagger +1 from daggers, the Cap of Wrath from a hat,
the Adamantine Shield from a shield, potions from potions). Items added in the app loaded
too: potions and a greatsword +2 made from what a character carried, and a mask in the camp
chest. Round 10, made by the command-line tool in the same ways the app uses, all loaded
and worked: copies from NPCs (an equipped sword made into Phalar Aluve, worn scale mail into
Adamantine Splint Armour, a stack of Potions of Speed), items made from another kind (a Spear
+1 from an oil, the Haste Helm from a quarterstaff), a new stack of ten healing potions, a
Scroll of Lightning Bolt made from Burning Hands with its spell left for the game to fill
in, an NPC's Scroll of Fireball, and copies from the camp chest to a character and back. A
scroll given a spell record of its own did not work. Round 11 tries the app's own export
for these, a scroll made from an oil, and exports from an autosave written as manual saves;
all worked. Items copied from ones marked as wares came out marked too, so new items now have
that mark cleared; round 12 confirmed it (a copy of a wares item, and an item made from one,
unmarked, the originals still marked).

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
