//! The application: screens, settings and the glass material.

use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{
    CentralPanel, Context, Key, KeyboardShortcut, Modifiers, Theme, ThemePreference, Ui, Visuals,
};
use sb_core::Result as CoreResult;

use crate::platform;
use crate::state::welcome::WelcomeForm;
use crate::state::{
    InspectorCache, Job, NoteAction, NoteDialog, Notice, NoticeBoard, Poll, SearchBox, Settings,
    ThemeChoice, Workspace,
};
use crate::ui::menu::{self, MenuOutcome};
use crate::ui::shell::{self, Sidebars, TopBar};
use crate::ui::theme::{self, Material, Palette};
use crate::ui::welcome;
use crate::ui::{dialogs, inspector, sidebar};

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
            ..VaultView::default()
        };
        self.settings.last_vault = Some(path);
        self.save_settings();
        self.screen = Screen::Vault(Box::new(workspace));
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
        let name = self.vault_name();
        let Screen::Vault(workspace) = &mut self.screen else {
            return;
        };
        let bar = TopBar {
            vault_name: &name,
            sidebars: &mut self.view.sidebars,
            settings: &mut self.settings,
        };
        let bar_outcome = shell::top_bar(ui, palette, bar);
        shell::status_strip(ui, palette, &mut self.notices, workspace);
        let action = show_panels(ui, palette, workspace, &mut self.view);
        shell::document_area(ui, palette);
        dialogs::show(ui.ctx(), palette, &mut self.view.dialog, workspace);
        let notice = action.and_then(|action| action.run(workspace, &mut self.view.dialog));
        if let Some(notice) = notice {
            self.notices.post(notice, Instant::now());
        }
        if bar_outcome.new_note {
            self.open_new_note_dialog();
        }
        self.apply_menu(bar_outcome.menu);
    }

    fn open_new_note_dialog(&mut self) {
        if self.view.dialog.is_none() {
            self.view.dialog = Some(NoteDialog::new_note(None));
        }
    }

    /// Ctrl+N opens the «New note» dialog; Ctrl+F focuses the search field.
    fn handle_note_keys(&mut self, ctx: &Context) {
        let pressed = |key| {
            let shortcut = KeyboardShortcut::new(Modifiers::COMMAND, key);
            ctx.input_mut(|input| input.consume_shortcut(&shortcut))
        };
        if pressed(Key::N) {
            self.open_new_note_dialog();
        }
        if pressed(Key::F) {
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
        let Screen::Vault(workspace) = &mut self.screen else {
            return;
        };
        match workspace.close() {
            Ok(()) => self.screen = Screen::Welcome,
            Err(error) => {
                let notice = Notice::error(format!("Could not close the note: {error}"));
                self.notices.post(notice, Instant::now());
            }
        }
    }
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
