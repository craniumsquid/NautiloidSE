//! The interface: a save browser, and an editor for the save that is open.
//!
//! [`EditorApp::update`] draws one frame. Everything a person does in a frame is collected as
//! [`Action`]s and applied afterwards, so drawing never changes what is being drawn. The
//! window, file dialogs and the system file manager are behind [`Platform`], which lets the
//! tests drive the whole interface without a screen.

pub mod approval;
pub mod browser;
pub mod changes;
pub mod dialogs;
pub mod game;
pub mod info;
pub mod inventory;
pub mod mods;
pub mod party;
pub mod story;
pub mod theme;
pub mod thumbs;
pub mod toast;
pub mod widgets;

use crate::party::Holder;
use crate::saves::{self, Dropped};
use crate::session::{Field, Session};
use browser::BrowserState;
use dialogs::{After, Dialog};
use eframe::egui::{
    self, Align, Align2, Context, CursorIcon, Frame, Key, KeyboardShortcut, Layout, Margin,
    Modifiers, Pos2, Rounding, Sense, Ui, Vec2, ViewportCommand,
};
use std::path::{Path, PathBuf};
use thumbs::Thumbnails;
use toast::{Toast, Toasts, Tone};
use widgets::Kind;

/// Below this width the Changes panel is hidden until asked for.
const NARROW_WINDOW: f32 = 1180.0;

/// What the app needs from the system it runs on.
pub trait Platform {
    /// Asks the person to choose a folder. `None` if they cancel.
    fn pick_folder(&self, title: &str, start: Option<&Path>) -> Option<PathBuf>;
    /// Shows `path` in the system's file manager.
    fn reveal(&self, path: &Path);
}

/// How the app starts.
#[derive(Default)]
pub struct Options {
    /// The saves folder to show. Without one, the game's own is looked for.
    pub saves_dir: Option<PathBuf>,
    /// Whether to look for the game's saves folder when none is given.
    pub detect: bool,
    /// A save to open straight away.
    pub open: Option<PathBuf>,
    /// The game's `Data` folder, for the names players see. Without one, the game is looked
    /// for if `detect` is set.
    pub game_dir: Option<PathBuf>,
    /// The current time in seconds since 1970, fixed. For tests; normally the clock is read.
    pub now: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Party,
    Inventory,
    Approval,
    Story,
    Mods,
    Info,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Party,
        Tab::Inventory,
        Tab::Approval,
        Tab::Story,
        Tab::Mods,
        Tab::Info,
    ];

    /// The tab a field is edited on.
    pub fn of(field: Field) -> Tab {
        match field {
            Field::Approval(..) => Tab::Approval,
            Field::Amount(..) | Field::Add(..) => Tab::Inventory,
            _ => Tab::Party,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Party => "Party",
            Tab::Inventory => "Inventory",
            Tab::Approval => "Approval",
            Tab::Story => "Story",
            Tab::Mods => "Mods",
            Tab::Info => "Save info",
        }
    }
}

/// What a folder is being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderFor {
    /// The folder the saves are in.
    Saves,
    /// The folder the game is installed in.
    Game,
}

/// Whether a folder dropped on the window reaches the app. Not under Wayland: the windowing
/// library doesn't pass file drops on there.
pub fn can_drop() -> bool {
    !(cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some())
}

/// A folder's path as someone typed or pasted it: without the quotes a file manager's "copy
/// path" puts around it or a `file://` in front, and with `~` as their home folder.
pub fn typed_path(text: &str, home: Option<&std::path::Path>) -> PathBuf {
    let mut text = text.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = text.strip_prefix(quote).and_then(|t| t.strip_suffix(quote)) {
            text = inner.trim();
        }
    }
    let text = text.strip_prefix("file://").unwrap_or(text);
    match (text.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(text),
    }
}

/// Something a person did, to be applied once the frame is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open the save in this folder.
    Open(PathBuf),
    /// Go back to the save browser.
    Back,
    Refresh,
    PickFolder,
    SetFolder(PathBuf),
    SelectCampaign(String),
    Tab(Tab),
    Set(Field, i32),
    Revert(Field),
    RevertAll,
    Undo,
    Redo,
    ToggleChanges,
    /// Ask for a name to export under.
    AskExport,
    /// Export under the name in the export dialog.
    Export,
    /// Drop the changes and carry on with what was asked.
    Discard,
    CloseDialog,
    Reveal(PathBuf),
    /// Ask where the game is, for the names players see.
    PickGameFolder,
    /// Ask for a folder's path as text, for when there is no folder picker to use.
    AskFolderPath(FolderFor),
    /// Use the path typed for this.
    UseFolderPath(FolderFor, String),
    /// Choose items from the game's to add for this holder, the search starting with this.
    AskAddItem(Holder, String),
    /// Add one more of the item with these stats for this holder.
    AddItem(Holder, String),
}

pub struct EditorApp {
    platform: Box<dyn Platform>,
    fixed_now: Option<u64>,
    browser: BrowserState,
    session: Option<Session>,
    tab: Tab,
    show_changes: bool,
    /// Whether the window is too narrow to keep the Changes panel beside the content.
    narrow: bool,
    dialog: Option<Dialog>,
    toasts: Toasts,
    thumbs: Thumbnails,
    /// The game's own files, for the names players see.
    game: game::GameFiles,
    /// Set once the person has agreed to close with changes unexported.
    allow_close: bool,
}

impl EditorApp {
    pub fn new(ctx: &Context, platform: Box<dyn Platform>, options: Options) -> EditorApp {
        theme::apply(ctx);
        let (dir, detected) = match options.saves_dir {
            Some(dir) => (Some(dir), false),
            None if options.detect => (saves::detect(), true),
            None => (None, false),
        };
        let detected = detected && dir.is_some();
        let game_dir = match options.game_dir {
            Some(dir) => Some(dir),
            None if options.detect => crate::catalog::detect_game(),
            None => None,
        };
        let game = match game_dir {
            Some(dir) => game::GameFiles::start(dir),
            None => game::GameFiles::Missing,
        };
        let mut app = EditorApp {
            platform,
            fixed_now: options.now,
            browser: BrowserState::new(dir, detected),
            session: None,
            tab: Tab::Party,
            show_changes: true,
            narrow: false,
            dialog: None,
            toasts: Toasts::default(),
            thumbs: Thumbnails::new(ctx),
            game,
            allow_close: false,
        };
        if let Some(path) = options.open {
            app.open_path(&path);
        }
        app
    }

    fn now(&self) -> u64 {
        self.fixed_now.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        })
    }

    /// The save that is open, if one is.
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn browser(&self) -> &BrowserState {
        &self.browser
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// The dialog that is up, if one is.
    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialog.as_ref()
    }

    /// The notices showing, oldest first.
    pub fn notices(&self) -> Vec<&str> {
        self.toasts.texts()
    }

    /// Whether anything is still loading in the background. Tests wait for this.
    pub fn is_settled(&self) -> bool {
        self.thumbs.is_idle() && !self.game.is_loading()
    }

    /// Whether the window may close: nothing is unexported, or the person said to go ahead.
    pub fn may_close(&self) -> bool {
        self.allow_close || !self.is_dirty()
    }

    fn is_dirty(&self) -> bool {
        self.session.as_ref().is_some_and(Session::is_dirty)
    }

    /// Draws one frame and applies what the person did in it.
    pub fn update(&mut self, ctx: &Context) {
        self.thumbs.poll(ctx);
        if let Some(catalog) = self.game.poll() {
            if let Some(session) = self.session.as_mut() {
                session.set_catalog(catalog);
            }
        }
        if self.game.is_loading() {
            // Keep looking until the names arrive.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut actions = Vec::new();
        self.handle_close_request(ctx);
        if self.dialog.is_none() {
            self.handle_dropped_files(ctx, &mut actions);
            self.handle_shortcuts(ctx, &mut actions);
        }
        let modal = self.dialog.is_some();
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        // In a narrow window the Changes panel starts out of the way; the header button
        // still brings it up. Widening the window puts it back.
        let narrow = ctx.screen_rect().width() < NARROW_WINDOW;
        if narrow != self.narrow {
            self.narrow = narrow;
            self.show_changes = !narrow;
        }

        self.header(ctx, modal, &mut actions);
        if self.session.is_some() {
            self.sidebar(ctx, modal, &mut actions);
            if self.show_changes {
                self.changes_panel(ctx, modal, &mut actions);
            }
        }
        self.content(ctx, modal, hovering, &mut actions);
        if let Some(dialog) = &mut self.dialog {
            dialogs::show(ctx, dialog, self.session.as_ref(), &mut actions);
        }
        self.toasts.show(ctx, &mut actions);

        let time = ctx.input(|i| i.time);
        for action in actions {
            self.apply(ctx, action, time);
        }
    }

    fn handle_close_request(&mut self, ctx: &Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.may_close() {
            return;
        }
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        if self.dialog.is_none() {
            self.dialog = Some(Dialog::Unexported { then: After::Close });
        }
    }

    fn handle_dropped_files(&mut self, ctx: &Context, actions: &mut Vec<Action>) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let Some(path) = dropped.into_iter().find_map(|f| f.path) else {
            return;
        };
        match saves::resolve(&path) {
            Some(Dropped::Save(folder)) => actions.push(Action::Open(folder)),
            Some(Dropped::SavesFolder(dir)) => actions.push(Action::SetFolder(dir)),
            None => self.toasts.push(Toast::new(
                "That isn't a save, a save folder or a folder of saves.",
                Tone::Problem,
            )),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &Context, actions: &mut Vec<Action>) {
        if self.session.is_none() {
            return;
        }
        let pressed = |modifiers: Modifiers, key: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        let shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // The longer shortcut first, or Ctrl+Shift+Z would be taken as Ctrl+Z.
        if pressed(shift, Key::Z) || pressed(Modifiers::COMMAND, Key::Y) {
            actions.push(Action::Redo);
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            actions.push(Action::Undo);
        }
        if pressed(Modifiers::COMMAND, Key::E) {
            actions.push(Action::AskExport);
        }
    }

    fn header(&mut self, ctx: &Context, modal: bool, actions: &mut Vec<Action>) {
        egui::TopBottomPanel::top("header")
            .exact_height(theme::HEADER_HEIGHT)
            .frame(
                Frame::none()
                    .fill(theme::BG_HEADER)
                    .inner_margin(Margin::symmetric(20.0, 0.0)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                // The line under the header.
                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.left() - 20.0..=rect.right() + 20.0,
                    rect.bottom(),
                    theme::stroke(1.0, theme::LINE),
                );
                ui.add_enabled_ui(!modal, |ui| {
                    ui.horizontal_centered(|ui| {
                        logo(ui);
                        let Some(session) = &self.session else {
                            return;
                        };
                        ui.add_space(18.0);
                        if widgets::button(ui, "⬅  All saves", Kind::Quiet).clicked() {
                            actions.push(Action::Back);
                        }
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add_space(13.0);
                            ui.set_max_width(360.0);
                            let summary = session.summary();
                            widgets::clipped(ui, &summary.name, theme::semibold(15.0), theme::TEXT);
                            widgets::clipped(
                                ui,
                                &format!(
                                    "{} · {}",
                                    summary.leader,
                                    browser::level_name(&summary.level)
                                ),
                                theme::body(12.5),
                                theme::MUTED,
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            // The way out is in the Changes panel; when that is hidden it
                            // is here.
                            if !self.show_changes
                                && widgets::button(ui, "Export…", Kind::Primary).clicked()
                            {
                                actions.push(Action::AskExport);
                            }
                            let count = session.changes().len();
                            let label = if count == 0 {
                                "Changes".to_owned()
                            } else {
                                format!("Changes  {count}")
                            };
                            let kind = if self.show_changes {
                                Kind::Secondary
                            } else {
                                Kind::Quiet
                            };
                            if widgets::button(ui, &label, kind).clicked() {
                                actions.push(Action::ToggleChanges);
                            }
                            ui.add_enabled_ui(session.can_redo(), |ui| {
                                if widgets::button(ui, "Redo", Kind::Quiet).clicked() {
                                    actions.push(Action::Redo);
                                }
                            });
                            ui.add_enabled_ui(session.can_undo(), |ui| {
                                if widgets::button(ui, "Undo", Kind::Quiet).clicked() {
                                    actions.push(Action::Undo);
                                }
                            });
                        });
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &Context, modal: bool, actions: &mut Vec<Action>) {
        let Some(session) = &self.session else {
            return;
        };
        egui::SidePanel::left("sidebar")
            .exact_width(theme::SIDEBAR_WIDTH)
            .resizable(false)
            .frame(
                Frame::none()
                    .fill(theme::BG_SIDEBAR)
                    .inner_margin(Margin::same(14.0)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().vline(
                    rect.right() + 14.0,
                    rect.top() - 14.0..=rect.bottom() + 14.0,
                    theme::stroke(1.0, theme::LINE),
                );
                ui.add_enabled_ui(!modal, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let changes = session.changes();
                    for tab in Tab::ALL {
                        let count = changes.iter().filter(|c| Tab::of(c.field) == tab).count();
                        if tab_button(ui, tab.label(), self.tab == tab, count).clicked() {
                            actions.push(Action::Tab(tab));
                        }
                    }
                    ui.add_space(14.0);
                    save_card(ui, session, &mut self.thumbs);
                    ui.add_space(10.0);
                    let changes = changes.len();
                    if changes > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "•  {changes} change{} not exported",
                                if changes == 1 { "" } else { "s" }
                            ))
                            .font(theme::semibold(13.0))
                            .color(theme::CHANGED),
                        );
                    }
                    ui.label(
                        egui::RichText::new("The save you opened is never changed.")
                            .font(theme::body(12.5))
                            .color(theme::MUTED),
                    );
                    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                        ui.label(
                            egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                .font(theme::body(12.0))
                                .color(theme::MUTED),
                        );
                    });
                });
            });
    }

    fn changes_panel(&mut self, ctx: &Context, modal: bool, actions: &mut Vec<Action>) {
        let Some(session) = &self.session else {
            return;
        };
        egui::SidePanel::right("changes")
            .exact_width(theme::CHANGES_WIDTH)
            .resizable(false)
            .frame(
                Frame::none()
                    .fill(theme::BG_SIDEBAR)
                    .inner_margin(Margin::same(16.0)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().vline(
                    rect.left() - 16.0,
                    rect.top() - 16.0..=rect.bottom() + 16.0,
                    theme::stroke(1.0, theme::LINE),
                );
                ui.add_enabled_ui(!modal, |ui| changes::show(ui, session, actions));
            });
    }

    fn content(&mut self, ctx: &Context, modal: bool, hovering: bool, actions: &mut Vec<Action>) {
        let now = self.now();
        egui::CentralPanel::default()
            .frame(Frame::none().fill(theme::BG))
            .show(ctx, |ui| {
                ui.add_enabled_ui(!modal, |ui| match &self.session {
                    None => browser::show(
                        ui,
                        &mut self.browser,
                        &mut self.thumbs,
                        now,
                        hovering,
                        actions,
                    ),
                    Some(session) => {
                        // Each tab keeps its own place, so a tab opens where it was left
                        // and not where another was scrolled to.
                        egui::ScrollArea::vertical()
                            .id_salt(self.tab.label())
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                Frame::none()
                                    .inner_margin(Margin {
                                        left: 28.0,
                                        right: 28.0,
                                        top: 22.0,
                                        bottom: 28.0,
                                    })
                                    .show(ui, |ui| match self.tab {
                                        Tab::Party => party::show(ui, session, actions),
                                        Tab::Inventory => {
                                            inventory::show(ui, session, &self.game, actions)
                                        }
                                        Tab::Approval => approval::show(ui, session, actions),
                                        Tab::Story => story::show(ui, session),
                                        Tab::Mods => mods::show(ui, session),
                                        Tab::Info => info::show(ui, session, now),
                                    });
                            });
                    }
                });
            });
    }

    fn open_path(&mut self, path: &Path) {
        match saves::resolve(path) {
            Some(Dropped::Save(folder)) => self.open_save(&folder),
            Some(Dropped::SavesFolder(dir)) => self.browser.set_dir(dir),
            None => {
                self.dialog = Some(Dialog::Problem {
                    title: "This can't be opened".to_owned(),
                    detail: format!(
                        "{} isn't a save, a save folder or a folder of saves.",
                        path.display()
                    ),
                })
            }
        }
    }

    fn open_save(&mut self, folder: &Path) {
        match Session::open(folder) {
            Ok(mut session) => {
                if let Some(catalog) = self.game.catalog() {
                    session.set_catalog(catalog);
                }
                self.session = Some(session);
                self.tab = Tab::Party;
                self.dialog = None;
            }
            Err(error) => {
                self.dialog = Some(Dialog::Problem {
                    title: format!("{} can't be edited", saves::display_name(folder)),
                    detail: format!("{error:#}"),
                });
            }
        }
    }

    /// Does what was put off while the person decided about unexported changes.
    fn carry_on(&mut self, ctx: &Context, then: After) {
        match then {
            After::Back => {
                self.session = None;
                self.browser.scan();
                self.thumbs.clear();
            }
            After::Open(folder) => {
                self.session = None;
                self.open_save(&folder);
            }
            After::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    /// Leaves the open save for `then`, asking first if there are unexported changes.
    fn leave(&mut self, ctx: &Context, then: After) {
        if self.is_dirty() {
            self.dialog = Some(Dialog::Unexported { then });
        } else {
            self.carry_on(ctx, then);
        }
    }

    fn apply(&mut self, ctx: &Context, action: Action, time: f64) {
        match action {
            Action::Open(folder) => {
                if self.session.is_some() {
                    self.leave(ctx, After::Open(folder));
                } else {
                    self.open_save(&folder);
                }
            }
            Action::Back => self.leave(ctx, After::Back),
            Action::Refresh => {
                self.browser.scan();
                self.thumbs.clear();
            }
            Action::PickGameFolder => {
                let picked = self.platform.pick_folder(
                    "Choose the game's folder (Baldurs Gate 3, or its Data folder)",
                    None,
                );
                if let Some(picked) = picked {
                    match game::data_folder(&picked) {
                        Some(data) => self.game = game::GameFiles::start(data),
                        None => self.toasts.push(Toast::new(
                            "That folder doesn't hold the game's files. Choose the Baldurs \
                             Gate 3 folder, or the Data folder inside it.",
                            Tone::Problem,
                        )),
                    }
                }
            }
            Action::AskFolderPath(what) => {
                let path = match (what, &self.browser.dir) {
                    (FolderFor::Saves, Some(dir)) => dir.display().to_string(),
                    _ => String::new(),
                };
                // So that the cursor goes in the path each time the dialog opens.
                ctx.data_mut(|d| d.remove::<bool>(egui::Id::new(dialogs::FOLDER_PATH_FOCUSED)));
                self.dialog = Some(Dialog::FolderPath {
                    what,
                    path,
                    error: None,
                });
            }
            Action::UseFolderPath(what, text) => {
                let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
                let home = var("HOME").or_else(|| var("USERPROFILE"));
                let path = typed_path(&text, home.as_deref());
                let problem = if !path.is_dir() {
                    Some("There is no folder at that path.".to_owned())
                } else {
                    match what {
                        FolderFor::Saves => {
                            self.dialog = None;
                            self.apply(ctx, Action::SetFolder(path), time);
                            None
                        }
                        FolderFor::Game => match game::data_folder(&path) {
                            Some(data) => {
                                self.dialog = None;
                                self.game = game::GameFiles::start(data);
                                None
                            }
                            None => Some(
                                "That folder doesn't hold the game's files. Give the Baldurs \
                                 Gate 3 folder, or the Data folder inside it."
                                    .to_owned(),
                            ),
                        },
                    }
                };
                if let (Some(problem), Some(Dialog::FolderPath { error, .. })) =
                    (problem, &mut self.dialog)
                {
                    *error = Some(problem);
                }
            }
            Action::PickFolder => {
                let start = self.browser.dir.clone();
                if let Some(dir) = self
                    .platform
                    .pick_folder("Choose your saves folder", start.as_deref())
                {
                    self.apply(ctx, Action::SetFolder(dir), time);
                }
            }
            Action::SetFolder(dir) => match saves::resolve(&dir) {
                // Someone who picks a single save's folder most likely wants that save.
                Some(Dropped::Save(folder)) => self.apply(ctx, Action::Open(folder), time),
                _ if self.session.is_some() => {}
                _ => {
                    self.browser.set_dir(dir);
                    self.thumbs.clear();
                }
            },
            Action::SelectCampaign(id) => self.browser.selected = Some(id),
            Action::Tab(tab) => self.tab = tab,
            Action::Set(field, value) => {
                if let Some(session) = &mut self.session {
                    session.set(field, value, time);
                }
            }
            Action::Revert(field) => {
                if let Some(session) = &mut self.session {
                    session.revert(field, time);
                }
            }
            Action::RevertAll => {
                if let Some(session) = &mut self.session {
                    session.revert_all(time);
                }
            }
            Action::Undo => {
                if let Some(session) = &mut self.session {
                    session.undo();
                }
            }
            Action::Redo => {
                if let Some(session) = &mut self.session {
                    session.redo();
                }
            }
            Action::ToggleChanges => self.show_changes = !self.show_changes,
            Action::AskExport => {
                if let Some(session) = &self.session {
                    let then = match self.dialog.take() {
                        Some(Dialog::Unexported { then }) => Some(then),
                        _ => None,
                    };
                    let parent = session.default_export_folder();
                    self.dialog = Some(Dialog::Export {
                        name: session.suggested_name(&parent),
                        error: None,
                        then,
                    });
                }
            }
            Action::Export => self.export(ctx),
            Action::Discard => {
                if let Some(Dialog::Unexported { then }) = self.dialog.take() {
                    self.carry_on(ctx, then);
                }
            }
            Action::CloseDialog => self.dialog = None,
            Action::AskAddItem(holder, query) => {
                self.dialog = Some(Dialog::AddItem { holder, query })
            }
            Action::AddItem(holder, stats) => {
                if let Some(session) = &mut self.session {
                    if let Err(why) = session.add_item(holder, &stats, time) {
                        self.toasts.push(Toast::new(why, Tone::Problem));
                    }
                }
            }
            Action::Reveal(path) => self.platform.reveal(&path),
        }
    }

    fn export(&mut self, ctx: &Context) {
        let (Some(session), Some(Dialog::Export { name, then, .. })) =
            (&self.session, self.dialog.take())
        else {
            return;
        };
        let name = name.trim().to_owned();
        let parent = session.default_export_folder();
        match session.export(&parent, &name) {
            Ok(folder) => {
                self.toasts.push(
                    Toast::new(format!("Exported as \"{name}\""), Tone::Success)
                        .with_action("Show in folder", Action::Reveal(folder.clone())),
                );
                match then {
                    Some(then) => {
                        self.session = None;
                        self.carry_on(ctx, then);
                    }
                    // Carry on from the exported save, so further changes build on it.
                    None => {
                        self.browser.scan();
                        self.open_save(&folder);
                    }
                }
            }
            Err(error) => {
                self.dialog = Some(Dialog::Export {
                    name,
                    error: Some(format!("{error:#}")),
                    then,
                });
            }
        }
    }
}

fn logo(ui: &mut Ui) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("NAUTILOID")
                .font(theme::display(22.0))
                .color(theme::ACCENT)
                .extra_letter_spacing(2.0),
        );
        ui.label(
            egui::RichText::new("SAVE EDITOR")
                .font(theme::bold(10.5))
                .color(theme::MUTED)
                .extra_letter_spacing(2.6),
        );
    });
}

fn tab_button(ui: &mut Ui, label: &str, selected: bool, count: usize) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = ui.is_enabled() && response.hovered();
        if selected {
            ui.painter().rect(
                rect,
                Rounding::same(8.0),
                theme::ACCENT_BG,
                theme::stroke(1.0, theme::ACCENT_LINE),
            );
        } else if hot {
            ui.painter()
                .rect_filled(rect, Rounding::same(8.0), theme::WIDGET);
        }
        let color = if selected {
            theme::ACCENT_HOVER
        } else {
            theme::TEXT_SOFT
        };
        ui.painter().text(
            Pos2::new(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::semibold(15.0),
            color,
        );
        if count > 0 {
            let badge = egui::Rect::from_center_size(
                Pos2::new(rect.right() - 22.0, rect.center().y),
                Vec2::new(24.0, 20.0),
            );
            ui.painter()
                .rect_filled(badge, Rounding::same(10.0), theme::CHANGED);
            ui.painter().text(
                badge.center(),
                Align2::CENTER_CENTER,
                count.to_string(),
                theme::bold(12.0),
                theme::ON_ACCENT,
            );
        }
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The open save, as its card in the browser but smaller.
fn save_card(ui: &mut Ui, session: &Session, thumbs: &mut Thumbnails) {
    let summary = session.summary();
    Frame::none()
        .fill(theme::PANEL)
        .stroke(theme::stroke(1.0, theme::LINE))
        .rounding(Rounding::same(10.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let width = ui.available_width();
            let (picture, _) = ui.allocate_exact_size(Vec2::new(width, 112.0), Sense::hover());
            let top = Rounding {
                nw: 9.0,
                ne: 9.0,
                sw: 0.0,
                se: 0.0,
            };
            let texture = summary
                .screenshot
                .as_ref()
                .and_then(|path| thumbs.get(path))
                .map(|t| (t.id(), t.size_vec2()));
            match texture {
                Some((id, size)) => {
                    let (frame, image) = (picture.aspect_ratio(), size.x / size.y.max(1.0));
                    let keep = (frame / image).min(1.0);
                    let tall = (image / frame).min(1.0);
                    let uv = egui::Rect::from_min_max(
                        Pos2::new((1.0 - keep) / 2.0, (1.0 - tall) / 2.0),
                        Pos2::new((1.0 + keep) / 2.0, (1.0 + tall) / 2.0),
                    );
                    ui.painter().add(widgets::textured(
                        picture.shrink(1.0),
                        top,
                        egui::Color32::WHITE,
                        id,
                        uv,
                    ));
                }
                None => {
                    ui.painter()
                        .rect_filled(picture.shrink(1.0), top, theme::SUNKEN);
                }
            }
            Frame::none()
                .inner_margin(Margin::symmetric(12.0, 10.0))
                .show(ui, |ui| {
                    ui.set_width(width - 24.0);
                    widgets::clipped(ui, &summary.name, theme::semibold(14.0), theme::TEXT);
                    widgets::clipped(
                        ui,
                        browser::level_name(&summary.level),
                        theme::body(12.5),
                        theme::MUTED,
                    );
                    widgets::clipped(
                        ui,
                        &browser::party_line(summary),
                        theme::body(12.5),
                        theme::MUTED,
                    );
                });
        });
}

#[cfg(test)]
mod path_tests {
    use super::typed_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn reads_a_typed_path() {
        let home = Path::new("/home/deck");
        let typed = |text: &str| typed_path(text, Some(home));
        assert_eq!(
            typed("  /mnt/games/Story \n"),
            PathBuf::from("/mnt/games/Story")
        );
        // As file managers copy it: in quotes, or as an address.
        assert_eq!(
            typed("\"/mnt/My Games/Story\""),
            PathBuf::from("/mnt/My Games/Story")
        );
        assert_eq!(typed("'/mnt/games'"), PathBuf::from("/mnt/games"));
        assert_eq!(typed("file:///mnt/games"), PathBuf::from("/mnt/games"));
        // The home folder.
        assert_eq!(
            typed("~/.local/share/Larian Studios"),
            PathBuf::from("/home/deck/.local/share/Larian Studios")
        );
        assert_eq!(typed("~"), PathBuf::from("/home/deck"));
        assert_eq!(typed("~other/x"), PathBuf::from("~other/x"));
        assert_eq!(typed_path("~/x", None), PathBuf::from("~/x"));
        // A Windows path is left as it is.
        assert_eq!(
            typed("C:\\Users\\me\\Saves"),
            PathBuf::from("C:\\Users\\me\\Saves")
        );
    }
}
