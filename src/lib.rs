//! Reading and writing Baldur's Gate 3 saves.
//!
//! * [`lspk`]: the `.lsv` package that holds a save's files.
//! * [`lsf`]: the `.lsf` documents inside it, and [`lsx`], their XML form, which mods carry.
//! * [`xml`]: a small reader for the XML files mods carry.
//! * [`modfiles`]: the player's active mods, read for the items they add.
//! * [`value`]: the typed values those documents carry.
//! * [`newage`]: the entity data blob inside `Globals.lsf` and the level caches.
//! * [`save`]: a save as a whole.
//! * [`party`]: the party's numbers in the entity data, and the edits to them.
//! * [`saves`]: finding saves on disk and describing them for the save browser.
//! * [`session`]: a save open for editing: its changes, undo and redo, and export.
//! * [`ui`]: the interface.
//! * [`diff`]: what differs between two documents.
//! * [`story`]: the story's databases and flags, read only.

pub mod catalog;
pub mod diff;
pub mod items;
pub mod lsf;
pub mod lspk;
pub mod lsx;
pub mod modfiles;
pub mod mods;
pub mod newage;
pub mod party;
pub mod save;
pub mod saves;
pub mod session;
pub mod story;
pub mod ui;
pub mod value;
pub mod xml;
