// No console window behind the app on Windows.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui;
use nautiloid_save_editor::ui::{EditorApp, Options, Platform};
use std::path::{Path, PathBuf};

/// The system's own folder picker and file manager.
struct NativePlatform;

impl Platform for NativePlatform {
    fn pick_folder(&self, title: &str, start: Option<&Path>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        dialog.pick_folder()
    }

    fn reveal(&self, path: &Path) {
        // Opens the folder in Explorer, Finder or the desktop's file manager. Nothing else
        // is run.
        #[cfg(target_os = "windows")]
        let program = "explorer";
        #[cfg(target_os = "macos")]
        let program = "open";
        #[cfg(all(unix, not(target_os = "macos")))]
        let program = "xdg-open";
        let _ = std::process::Command::new(program).arg(path).spawn();
    }
}

struct App {
    editor: EditorApp,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.editor.update(ctx);
    }
}

fn main() -> eframe::Result {
    // A save, a save folder or a folder of saves can be given on the command line.
    let open = std::env::args_os().nth(1).map(PathBuf::from);
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Nautiloid Save Editor")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([980.0, 640.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Nautiloid Save Editor",
        native,
        Box::new(move |creation| {
            let options = Options {
                detect: true,
                open,
                ..Options::default()
            };
            Ok(Box::new(App {
                editor: EditorApp::new(&creation.egui_ctx, Box::new(NativePlatform), options),
            }))
        }),
    )
}
