//! The game's own files, read in the background for the names players see.

use super::widgets::{self, Kind};
use super::{theme, Action};
use crate::catalog::Catalog;
use eframe::egui::{self, Ui};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

/// Where reading the game's files stands.
pub enum GameFiles {
    /// No game folder known.
    Missing,
    Loading {
        folder: PathBuf,
        result: Receiver<Result<Catalog, String>>,
    },
    Ready {
        catalog: Arc<Catalog>,
    },
    Failed {
        folder: PathBuf,
        error: String,
    },
}

impl GameFiles {
    /// Starts reading the game's files in `folder` (its `Data` folder) on another thread.
    pub fn start(folder: PathBuf) -> GameFiles {
        let (send, result) = channel();
        let from = folder.clone();
        std::thread::spawn(move || {
            let _ = send.send(Catalog::from_game(&from).map_err(|e| format!("{e:#}")));
        });
        GameFiles::Loading { folder, result }
    }

    /// Takes the result if reading has finished, and returns the catalog if it just arrived.
    pub fn poll(&mut self) -> Option<Arc<Catalog>> {
        let GameFiles::Loading { folder, result } = self else {
            return None;
        };
        let outcome = match result.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("reading stopped unexpectedly".to_owned()),
        };
        match outcome {
            Ok(catalog) => {
                let catalog = Arc::new(catalog);
                *self = GameFiles::Ready {
                    catalog: catalog.clone(),
                };
                Some(catalog)
            }
            Err(error) => {
                *self = GameFiles::Failed {
                    folder: folder.clone(),
                    error,
                };
                None
            }
        }
    }

    pub fn catalog(&self) -> Option<Arc<Catalog>> {
        match self {
            GameFiles::Ready { catalog } => Some(catalog.clone()),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, GameFiles::Loading { .. })
    }
}

/// The `Data` folder for a folder someone picked: that folder, or the `Data` folder in it.
pub fn data_folder(picked: &Path) -> Option<PathBuf> {
    [picked.to_path_buf(), picked.join("Data")]
        .into_iter()
        .find(|folder| folder.join("Shared.pak").is_file())
}

/// A line saying where item names come from, with a way to point at the game when they
/// can't come from it. Nothing when they do.
pub fn notice(ui: &mut Ui, game: &GameFiles, actions: &mut Vec<Action>) {
    let text = match game {
        GameFiles::Ready { .. } => return,
        GameFiles::Loading { .. } => "Reading item names from your game…".to_owned(),
        GameFiles::Missing => "Items go by the game's internal names: the game wasn't found. \
                               Point at its folder for the names players see."
            .to_owned(),
        GameFiles::Failed { folder, error } => format!(
            "Items go by the game's internal names: the game's files in {} couldn't be read \
             ({error}).",
            folder.display()
        ),
    };
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(text).color(theme::MUTED));
            if !game.is_loading()
                && widgets::button(ui, "Choose game folder…", Kind::Secondary).clicked()
            {
                actions.push(Action::PickGameFolder);
            }
        });
    });
    ui.add_space(8.0);
}
