//! The application: screens, settings and the glass material.

use std::mem;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{
    CentralPanel, Context, Key, KeyboardShortcut, Modifiers, Rect, Theme, ThemePreference, Ui,
    Visuals,
};
use sb_core::Result as CoreResult;

use crate::platform;
use crate::state::welcome::WelcomeForm;
use crate::state::{
    AUTOSAVE_DELAY, InspectorCache, Job, NoteAction, NoteDialog, Notice, NoticeBoard, Poll,
    QuickOpen, Reloader, Saved, SearchBox, Settings, ThemeChoice, Workspace,
};
use crate::ui::editor::{self, EditorState};
use crate::ui::graph_view::GraphView;
use crate::ui::menu::{self, MenuOutcome};
use crate::ui::shell::{self, Sidebars, TopBar, TopBarOutcome};
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
    /// Base directory of sync state; the sync window (a later task) reads it.
    pub state_base: PathBuf,
    /// The vault named on the command line, if any.
    pub vault: Option<PathBuf>,
}

enum Screen {
    Welcome,
    Opening {
        path: PathBuf,
        job: Job<CoreResult<Workspace>>,
    },
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
}

impl SecondBrainApp {
    /// Loads the settings and starts opening `options.vault` or else the last vault.
    pub fn new(ctx: &Context, options: AppOptions) -> SecondBrainApp {
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
        self.poll_opening();
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
        let job = Job::spawn(
            move || Workspace::open(&root),
            move || ctx.request_repaint(),
        );
        self.screen = Screen::Opening { path, job };
    }

    fn poll_opening(&mut self) {
        let Screen::Opening { path, job } = &self.screen else {
            return;
        };
        let path = path.clone();
        match job.poll() {
            Poll::Running => {}
            Poll::Ready(Ok(workspace)) => self.finish_opening(path, workspace),
            Poll::Ready(Err(error)) => self.fail_opening(&error.to_string()),
            Poll::Lost => self.fail_opening("the opening stopped unexpectedly"),
        }
    }

    fn finish_opening(&mut self, path: PathBuf, workspace: Workspace) {
        self.welcome = WelcomeForm::default();
        self.view = VaultView {
            sidebars: self.view.sidebars,
            reloader: self.start_reloader(workspace.vault().root()),
            ..VaultView::default()
        };
        self.settings.last_vault = Some(path);
        self.save_settings();
        self.screen = Screen::Vault(Box::new(workspace));
    }

    /// Starts following the vault folder; without a watcher the vault stays usable.
    fn start_reloader(&mut self, root: &Path) -> Option<Reloader> {
        let ctx = self.ctx.clone();
        match Reloader::start(root, move || ctx.request_repaint()) {
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
        shell::status_strip(ui, palette, &mut self.notices, workspace);
        let side = show_panels(ui, palette, workspace, &mut self.view);
        let centre = shell::document_area(ui, palette, |ui| {
            show_centre(ui, palette, workspace, &mut self.view)
        });
        dialogs::show(ui.ctx(), palette, &mut self.view.dialog, workspace);
        let found = quick_open::show(ui.ctx(), palette, &mut self.view.quick_open, workspace);
        let from_graph = self.view.graph && side.is_none() && centre.is_some();
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

    /// Saves when the autosave delay has passed or the window lost focus; follows the folder.
    fn tick_vault(&mut self, ctx: &Context) {
        let now = Instant::now();
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
                if let Some(reloader) = self.view.reloader.as_mut() {
                    reloader.mark_changed(now);
                }
                let text = format!("{path} changed on disk; reloading");
                self.notices.post(Notice::info(text), now);
                false
            }
            Err(error) => {
                let text = format!("Could not save {path}: {error}");
                self.notices.post(Notice::error(text), now);
                false
            }
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
        if self.view.dialog.is_some() || self.view.quick_open.is_some() {
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

    /// Closes the vault (saving its open note) and returns to the welcome screen.
    fn leave_vault(&mut self) {
        if !self.save_open_note() {
            return;
        }
        let Screen::Vault(workspace) = &mut self.screen else {
            return;
        };
        match workspace.close() {
            Ok(()) => {
                self.screen = Screen::Welcome;
                self.view.reloader = None;
            }
            Err(error) => {
                let notice = Notice::error(format!("Could not close the note: {error}"));
                self.notices.post(notice, Instant::now());
            }
        }
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

    /// A window closed within the autosave delay must not lose what was typed.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_open_note();
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
