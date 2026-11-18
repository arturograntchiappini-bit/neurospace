//! The application: screens, settings and the glass material.

use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui::{
    CentralPanel, Context, Key, KeyboardShortcut, Modifiers, Rect, Theme, ThemePreference, Ui,
    ViewportCommand, Visuals,
};
use sb_core::Result as CoreResult;

use crate::platform;
use crate::state::sync_panel::SyncControl;
use crate::state::welcome::WelcomeForm;
use crate::state::{
    AUTOSAVE_DELAY, InspectorCache, Job, NoteAction, NoteDialog, Notice, NoticeBoard, Poll,
    QuickOpen, RELOAD_DEBOUNCE, Reloader, Saved, SearchBox, Settings, ThemeChoice, Workspace,
};
use crate::sync_control::EngineControl;
use crate::sync_host::{ControlFactory, SyncHost};
use crate::ui::editor::{self, EditorState};
use crate::ui::graph_view::GraphView;
use crate::ui::menu::{self, MenuOutcome};
use crate::ui::shell::{self, Sidebars, Strip, TopBar, TopBarOutcome};
use crate::ui::theme::{self, Material, Palette};
use crate::ui::welcome;
use crate::ui::{dialogs, inspector, quick_open, sidebar};

/// One-time setup of a context: fonts, the styles of both themes and the zoom-key rule.
///
/// The application calls it in `new`; a test harness whose context is not the one given to
/// `new` calls it through its context setup.
pub fn prepare_context(ctx: &Context) {
    theme::install_fonts(ctx);
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    for theme in [Theme::Light, Theme::Dark] {
        // The glass alpha is not part of a style, so every material gives the same result.
        theme::apply(ctx, theme, &theme::palette(theme, Material::Opaque));
    }
}

/// How the application starts.
#[derive(Clone, Debug)]
pub struct AppOptions {
    /// Where `app.json` lives.
    pub settings_path: PathBuf,
    /// Base directory of sync state: the sync engine keeps each vault's state under it.
    pub state_base: PathBuf,
    /// The vault named on the command line, if any.
    pub vault: Option<PathBuf>,
}

/// What the opening job returns: the workspace and the watcher that follows its folder.
type Opened = CoreResult<(Workspace, CoreResult<Reloader>)>;

enum Screen {
    Welcome,
    Opening { path: PathBuf, job: Job<Opened> },
    Vault(Box<Workspace>),
}

/// What the open vault's panels remember between frames.
#[derive(Default)]
struct VaultView {
    sidebars: Sidebars,
    search: SearchBox,
    inspector: InspectorCache,
    dialog: Option<NoteDialog>,
    editor: EditorState,
    quick_open: Option<QuickOpen>,
    graph: bool,
    graph_view: GraphView,
    reloader: Option<Reloader>,
    window_focused: bool,
}

/// The whole application state behind the window.
pub struct SecondBrainApp {
    ctx: Context,
    settings: Settings,
    settings_path: PathBuf,
    screen: Screen,
    welcome: WelcomeForm,
    notices: NoticeBoard,
    view: VaultView,
    mica_available: bool,
    mica_key: Option<(bool, bool)>,
    close_warning: Option<String>,
    sync: SyncHost,
}

/// The engine control of a vault: built on the opening thread, never on the UI thread.
fn engine_factory(state_base: PathBuf) -> ControlFactory {
    Arc::new(move |root| {
        let control = EngineControl::new(root, &state_base).map_err(|error| error.to_string())?;
        Ok(Box::new(control) as Box<dyn SyncControl>)
    })
}

impl SecondBrainApp {
    /// Loads the settings and starts opening `options.vault` or else the last vault.
    pub fn new(ctx: &Context, options: AppOptions) -> SecondBrainApp {
        let factory = engine_factory(options.state_base.clone());
        Self::with_factory(ctx, options, factory)
    }

    /// Like `new`, but the first vault opened gets `control` instead of the real sync engine.
    /// For tests, which never start the engine or touch the network.
    pub fn with_sync_control(
        ctx: &Context,
        options: AppOptions,
        control: Box<dyn SyncControl>,
    ) -> SecondBrainApp {
        let slot = Mutex::new(Some(control));
        let factory: ControlFactory = Arc::new(move |_| {
            let taken = slot.lock().ok().and_then(|mut slot| slot.take());
            taken.ok_or_else(|| "this test gave sync control to one vault only".to_owned())
        });
        Self::with_factory(ctx, options, factory)
    }

    fn with_factory(ctx: &Context, options: AppOptions, factory: ControlFactory) -> SecondBrainApp {
        prepare_context(ctx);
        let settings = Settings::load(&options.settings_path);
        let start = options.vault.or_else(|| settings.last_vault.clone());
        let mut app = SecondBrainApp {
            ctx: ctx.clone(),
            settings,
            settings_path: options.settings_path,
            screen: Screen::Welcome,
            welcome: WelcomeForm::default(),
            notices: NoticeBoard::default(),
            view: VaultView::default(),
            mica_available: false,
            mica_key: None,
            close_warning: None,
            sync: SyncHost::new(factory),
        };
        app.sync_context(ctx);
        if let Some(path) = start {
            app.open_vault(path);
        }
        app
    }

    /// The material behind the glass panels (spec 9.6).
    pub fn material(&self) -> Material {
        if self.settings.reduce_transparency {
            Material::Opaque
        } else if self.mica_available {
            Material::Mica
        } else {
            Material::Simulated
        }
    }

    /// Records whether the system accepted the Mica backdrop.
    pub fn set_material_available(&mut self, available: bool) {
        self.mica_available = available;
        self.ctx.request_repaint();
    }

    /// Whether «Reduce transparency» is on.
    pub fn reduce_transparency(&self) -> bool {
        self.settings.reduce_transparency
    }

    /// True once per change of the theme or the transparency setting; remembers them.
    pub fn material_update_due(&mut self, dark: bool) -> bool {
        let key = Some((dark, self.settings.reduce_transparency));
        let due = self.mica_key != key;
        self.mica_key = key;
        due
    }

    /// Which screen is showing: `"welcome"`, `"opening"` or `"vault"`. Used by tests.
    pub fn screen_name(&self) -> &'static str {
        match self.screen {
            Screen::Welcome => "welcome",
            Screen::Opening { .. } => "opening",
            Screen::Vault(_) => "vault",
        }
    }

    /// The open vault, if any. Used by tests.
    pub fn workspace(&self) -> Option<&Workspace> {
        match &self.screen {
            Screen::Vault(workspace) => Some(workspace),
            _ => None,
        }
    }

    /// Whether the open note is shown rendered instead of in the editor. Used by tests.
    pub fn is_previewing(&self) -> bool {
        self.view.editor.preview
    }

    /// Whether the graph view replaces the note in the centre pane.
    pub fn graph_shown(&self) -> bool {
        self.view.graph
    }

    /// Whether the graph layout has come to rest. Used by tests.
    pub fn graph_settled(&self) -> bool {
        self.view.graph_view.is_settled()
    }

    /// The area the graph was drawn in last frame. Used by tests.
    pub fn graph_pane_rect(&self) -> Option<Rect> {
        self.view.graph_view.pane_rect()
    }

    /// Where the graph drew node `id` last frame. Used by tests.
    pub fn graph_node_rect(&self, id: &str) -> Option<Rect> {
        self.view.graph_view.node_rect(id)
    }

    /// The error the welcome screen shows, if any. Used by tests.
    pub fn welcome_error(&self) -> Option<&str> {
        self.welcome.error.as_deref()
    }

    /// Draws one frame: everything that needs no `eframe::Frame`.
    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.sync_context(&ctx);
        let palette = theme::palette(ctx.theme(), self.material());
        if self.material() != Material::Mica {
            theme::paint_backdrop(&ctx, &palette);
        }
        if menu::handle_zoom_keys(&ctx, &mut self.settings) {
            self.save_settings();
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            self.handle_close_request(&ctx);
        }
        self.poll_opening();
        self.sync.poll(&ctx);
        if matches!(self.screen, Screen::Vault(_)) {
            self.handle_note_keys(&ctx);
        }
        match &self.screen {
            Screen::Welcome => self.show_welcome(ui, &palette),
            Screen::Opening { .. } => show_opening(ui),
            Screen::Vault(_) => self.show_vault(ui, &palette),
        }
    }

    /// Pushes the theme choice and the text size to `ctx`.
    fn sync_context(&self, ctx: &Context) {
        ctx.set_theme(theme_preference(self.settings.theme));
        if ctx.zoom_factor() != self.settings.zoom {
            ctx.set_zoom_factor(self.settings.zoom);
        }
    }

    fn save_settings(&mut self) {
        if let Err(error) = self.settings.save(&self.settings_path) {
            let notice = Notice::error(format!("Could not save settings: {error}"));
            self.notices.post(notice, Instant::now());
        }
    }

    fn open_vault(&mut self, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let (root, ctx) = (path.clone(), self.ctx.clone());
        let wake = ctx.clone();
        let job = Job::spawn(
            move || {
                let workspace = Workspace::open(&root)?;
                let reloader = Reloader::start(workspace.vault().root(), move || {
                    wake.request_repaint();
                });
                Ok((workspace, reloader))
            },
            move || ctx.request_repaint(),
        );
        self.sync.switch_to(&path, &self.ctx);
        self.screen = Screen::Opening { path, job };
    }

    fn poll_opening(&mut self) {
        let Screen::Opening { path, job } = &self.screen else {
            return;
        };
        let path = path.clone();
        match job.poll() {
            Poll::Running => {}
            Poll::Ready(Ok((workspace, reloader))) => {
                self.finish_opening(path, workspace, reloader);
            }
            Poll::Ready(Err(error)) => self.fail_opening(&error.to_string()),
            Poll::Lost => self.fail_opening("the opening stopped unexpectedly"),
        }
    }

    fn finish_opening(
        &mut self,
        path: PathBuf,
        workspace: Workspace,
        reloader: CoreResult<Reloader>,
    ) {
        self.welcome = WelcomeForm::default();
        self.view = VaultView {
            sidebars: self.view.sidebars,
            reloader: self.keep_reloader(reloader),
            ..VaultView::default()
        };
        self.settings.last_vault = Some(path);
        self.save_settings();
        self.screen = Screen::Vault(Box::new(workspace));
    }

    /// Keeps the folder watcher; without one the vault stays usable.
    fn keep_reloader(&mut self, reloader: CoreResult<Reloader>) -> Option<Reloader> {
        match reloader {
            Ok(reloader) => Some(reloader),
            Err(error) => {
                let notice = Notice::error(format!(
                    "Could not watch the vault folder: {error}. \
                     External changes will not appear until you reopen the vault"
                ));
                self.notices.post(notice, Instant::now());
                None
            }
        }
    }

    fn fail_opening(&mut self, reason: &str) {
        self.welcome.error = Some(format!("Could not open the vault: {reason}"));
        self.sync.stop_for_leaving();
        self.screen = Screen::Welcome;
    }

    fn show_welcome(&mut self, ui: &mut Ui, palette: &Palette) {
        if let Some(path) = welcome::show(ui, palette, &mut self.welcome) {
            self.open_vault(path);
        }
    }

    fn show_vault(&mut self, ui: &mut Ui, palette: &Palette) {
        self.tick_vault(ui.ctx());
        let name = self.vault_name();
        let Screen::Vault(workspace) = &mut self.screen else {
            return;
        };
        let bar = TopBar {
            vault_name: &name,
            sidebars: &mut self.view.sidebars,
            settings: &mut self.settings,
            preview: self.view.editor.preview,
        };
        let bar_outcome = shell::top_bar(ui, palette, bar);
        let strip = Strip {
            workspace,
            sync_summary: &self.sync.panel.summary(),
        };
        let strip_opens_sync = shell::status_strip(ui, palette, &mut self.notices, strip);
        let side = show_panels(ui, palette, workspace, &mut self.view);
        let centre = shell::document_area(ui, palette, |ui| {
            show_centre(ui, palette, workspace, &mut self.view)
        });
        let created = dialogs::show(ui.ctx(), palette, &mut self.view.dialog, workspace);
        self.view.graph &= !created;
        let found = quick_open::show(ui.ctx(), palette, &mut self.view.quick_open, workspace);
        let from_graph = self.view.graph && side.is_none() && centre.is_some();
        self.sync.window_open |= strip_opens_sync;
        self.sync.show_window(ui.ctx(), palette);
        self.run_action(side.or(centre).or(found), from_graph);
        self.schedule_autosave(ui.ctx());
        self.apply_bar(bar_outcome);
    }

    /// Runs what the user asked for and reports a failure. Opening a note ends the graph view;
    /// a click on a graph node also puts the note in edit mode.
    fn run_action(&mut self, action: Option<NoteAction>, from_graph: bool) {
        let (Some(action), Screen::Vault(workspace)) = (action, &mut self.screen) else {
            return;
        };
        let leaves_graph = self.view.graph && action.navigates();
        let notice = action.run(workspace, &mut self.view.dialog);
        if leaves_graph && notice.is_none() {
            self.view.graph = false;
            self.view.editor.preview &= !from_graph;
        }
        if let Some(notice) = notice {
            self.notices.post(notice, Instant::now());
        }
    }

    /// Saves when the autosave delay has passed or the window lost focus; follows the folder;
    /// hands the motion setting to the graph.
    fn tick_vault(&mut self, ctx: &Context) {
        let now = Instant::now();
        let reduce_motion = self.settings.reduce_motion;
        self.view.graph_view.set_reduce_motion(reduce_motion);
        let focused = ctx.input(|input| input.focused);
        let lost_focus = mem::replace(&mut self.view.window_focused, focused) && !focused;
        let due = self.workspace().is_some_and(|w| w.autosave_due(now));
        if lost_focus || due {
            self.save_open_note();
        }
        self.tick_reloader(ctx, now);
    }

    /// Applies a finished rebuild and wakes the UI when the debounce of a pending change ends.
    fn tick_reloader(&mut self, ctx: &Context, now: Instant) {
        let (Screen::Vault(workspace), Some(reloader)) =
            (&mut self.screen, self.view.reloader.as_mut())
        else {
            return;
        };
        for notice in reloader.tick(workspace, now) {
            self.notices.post(notice, now);
        }
        if let Some(wait) = reloader.next_tick_in(now) {
            ctx.request_repaint_after(wait);
        }
    }

    /// Asks for a frame when the delay of a pending autosave has passed.
    fn schedule_autosave(&self, ctx: &Context) {
        let open = self.workspace().and_then(Workspace::open_note);
        if open.is_some_and(|open| open.is_dirty() && !open.autosave_paused) {
            ctx.request_repaint_after(AUTOSAVE_DELAY);
        }
    }

    /// Saves the open note. Nothing is written when the file changed under it: the reloader is
    /// told to look at the folder and the user is told why. True when nothing unsaved remains.
    fn save_open_note(&mut self) -> bool {
        let now = Instant::now();
        let Screen::Vault(workspace) = &mut self.screen else {
            return true;
        };
        let path = workspace.open_note().map(|open| open.path.to_string());
        let path = path.unwrap_or_default();
        match workspace.save() {
            Ok(Saved::Nothing | Saved::Written) => true,
            Ok(Saved::ChangedOnDisk) => {
                let reloading = self.view.reloader.is_some();
                if let Some(reloader) = self.view.reloader.as_mut() {
                    reloader.mark_changed(now);
                    self.ctx.request_repaint_after(RELOAD_DEBOUNCE);
                }
                self.notices
                    .post(Notice::changed_on_disk(&path, reloading), now);
                false
            }
            Err(error) => {
                let text = format!("Could not save {path}: {error}");
                self.notices.post(Notice::error(text), now);
                false
            }
        }
    }

    /// The window is closing: save, or keep what cannot be saved in a conflict note. A failure
    /// cancels the close and says so; asking again while that notice shows quits without saving.
    fn handle_close_request(&mut self, ctx: &Context) {
        let now = Instant::now();
        let warned = self.close_warning.take();
        let shown = self.notices.current(now).map(|notice| &notice.text);
        if warned.as_ref().is_some_and(|text| Some(text) == shown) {
            return;
        }
        if let Err(text) = self.save_for_close() {
            let text = format!("{text}. Close again to quit without saving");
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.notices.post(Notice::error(text.clone()), now);
            self.close_warning = Some(text);
        }
    }

    /// The steps that quitting and opening another vault share: the open note is saved (or kept
    /// in a conflict note), then the settings. Err says why the note could not be kept.
    fn flush(&mut self) -> Result<(), String> {
        let note = self.save_for_close();
        self.save_settings();
        note
    }

    fn save_for_close(&mut self) -> Result<(), String> {
        let Screen::Vault(workspace) = &mut self.screen else {
            return Ok(());
        };
        let path = workspace.open_note().map(|open| open.path.to_string());
        let failed = |error: sb_core::CoreError| {
            let path = path.clone().unwrap_or_default();
            format!("Could not save {path}: {error}")
        };
        match workspace.save() {
            Ok(Saved::Nothing | Saved::Written) => Ok(()),
            Ok(Saved::ChangedOnDisk) => workspace
                .keep_unsaved_as_conflict()
                .map(drop)
                .map_err(failed),
            Err(error) => Err(failed(error)),
        }
    }

    fn apply_bar(&mut self, outcome: TopBarOutcome) {
        if outcome.new_note {
            self.open_new_note_dialog();
        }
        if outcome.quick_open {
            self.open_quick_open();
        }
        self.view.editor.preview ^= outcome.toggle_preview;
        self.view.graph ^= outcome.toggle_graph;
        self.sync.window_open |= outcome.open_sync;
        self.apply_menu(outcome.menu);
    }

    fn open_new_note_dialog(&mut self) {
        if self.view.dialog.is_none() && self.view.quick_open.is_none() {
            self.view.dialog = Some(NoteDialog::new_note(None));
        }
    }

    fn open_quick_open(&mut self) {
        if self.view.dialog.is_none() && self.view.quick_open.is_none() {
            self.view.quick_open = Some(QuickOpen::default());
        }
    }

    /// The shortcuts of spec 9.3, taken before the editor can see them. A popup in front keeps
    /// them to itself.
    fn handle_note_keys(&mut self, ctx: &Context) {
        let popup = self.view.dialog.is_some() || self.view.quick_open.is_some();
        if popup || self.sync.window_open {
            return;
        }
        if pressed(ctx, Key::N) {
            self.open_new_note_dialog();
        }
        if pressed(ctx, Key::O) {
            self.open_quick_open();
        }
        self.view.editor.preview ^= pressed(ctx, Key::E);
        self.view.graph ^= pressed(ctx, Key::G);
        if pressed(ctx, Key::F) {
            self.view.sidebars.left = true;
            ctx.memory_mut(|memory| memory.request_focus(sidebar::search_field_id()));
        }
    }

    fn vault_name(&self) -> String {
        let root = self.workspace().map_or(Path::new(""), |w| w.vault().root());
        let name = root.file_name().unwrap_or(root.as_os_str());
        name.to_string_lossy().into_owned()
    }

    fn apply_menu(&mut self, outcome: MenuOutcome) {
        if outcome.settings_changed {
            self.save_settings();
        }
        if outcome.open_another_vault {
            self.leave_vault();
        }
    }

    /// Closes the vault (after the shared shutdown steps) and returns to the welcome screen.
    /// The engine is asked to stop without waiting: the UI thread never blocks on it.
    fn leave_vault(&mut self) {
        if let Err(text) = self.flush() {
            self.notices.post(Notice::error(text), Instant::now());
            return;
        }
        self.screen = Screen::Welcome;
        self.view.reloader = None;
        self.sync.stop_for_leaving();
    }
}

fn pressed(ctx: &Context, key: Key) -> bool {
    let shortcut = KeyboardShortcut::new(Modifiers::COMMAND, key);
    ctx.input_mut(|input| input.consume_shortcut(&shortcut))
}

/// Draws both side panels; returns what the user asked for in either.
fn show_panels(
    ui: &mut Ui,
    palette: &Palette,
    workspace: &Workspace,
    view: &mut VaultView,
) -> Option<NoteAction> {
    let VaultView {
        sidebars,
        search,
        inspector: links,
        ..
    } = view;
    let left = shell::left_panel(ui, palette, &mut sidebars.left, |ui| {
        sidebar::show(ui, palette, workspace, search)
    });
    let right = shell::right_panel(ui, palette, &mut sidebars.right, |ui| {
        inspector::show(ui, palette, workspace, links)
    });
    left.flatten().or(right.flatten())
}

/// Draws the graph or the note in the document area; returns the note the user asked for.
fn show_centre(
    ui: &mut Ui,
    palette: &Palette,
    workspace: &mut Workspace,
    view: &mut VaultView,
) -> Option<NoteAction> {
    if !view.graph {
        return editor::show(ui, palette, workspace, &mut view.editor);
    }
    let outcome = view.graph_view.show(ui, palette, workspace);
    if outcome.list_view {
        view.quick_open.get_or_insert_with(QuickOpen::default);
    }
    outcome.action
}

fn show_opening(ui: &mut Ui) {
    CentralPanel::no_frame().show(ui, |ui| {
        ui.centered_and_justified(|ui| ui.label("Opening the vault…"));
    });
}

fn theme_preference(choice: ThemeChoice) -> ThemePreference {
    match choice {
        ThemeChoice::System => ThemePreference::System,
        ThemeChoice::Light => ThemePreference::Light,
        ThemeChoice::Dark => ThemePreference::Dark,
    }
}

impl eframe::App for SecondBrainApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        platform::sync_material(&*frame, ui.ctx().theme(), self);
        self.show(ui);
    }

    /// The window is closing: the shared shutdown steps, then the engine is stopped and waited
    /// for. Nothing is drawn any more, so a failure goes to stderr.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(text) = self.flush() {
            eprintln!("secondbrain: {text}");
        }
        self.sync.shut_down();
    }

    fn clear_color(&self, visuals: &Visuals) -> [f32; 4] {
        if self.material() == Material::Mica {
            return [0.0; 4];
        }
        let theme = if visuals.dark_mode {
            Theme::Dark
        } else {
            Theme::Light
        };
        let top = theme::palette(theme, self.material()).backdrop_top;
        top.to_color32().to_normalized_gamma_f32()
    }
}
