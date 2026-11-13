//! Reading and writing Baldur's Gate 3 saves.
//!
//! * [`lspk`]: the `.lsv` package that holds a save's files.
//! * [`lsf`]: the `.lsf` documents inside it.
//! * [`value`]: the typed values those documents carry.
//! * [`newage`]: the entity data blob inside `Globals.lsf` and the level caches.
//! * [`save`]: a save as a whole.
//! * [`party`]: the party's numbers in the entity data, and the edits to them.
//! * [`saves`]: finding saves on disk and describing them for the save browser.
//! * [`session`]: a save open for editing: its changes, undo and redo, and export.
//! * [`diff`]: what differs between two documents.

pub mod diff;
pub mod lsf;
pub mod lspk;
pub mod newage;
pub mod party;
pub mod save;
pub mod saves;
pub mod session;
pub mod value;
