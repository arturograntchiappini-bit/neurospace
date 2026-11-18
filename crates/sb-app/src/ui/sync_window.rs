//! The «Sync» window (spec 8.8 and 9.5): a modal that shows the sync engine and drives it.

use eframe::egui::{
    Button, Context, Id, Key, Margin, Modal, RichText, ScrollArea, TextEdit, TextStyle, Ui,
};
use sb_sync::{NetMode, Status};

use crate::state::sync_panel::{Request, SyncPanel, SyncState};
use crate::ui::theme::{self, Palette};

const WIDTH: f32 = 440.0;
const PADDING: i8 = 20;
const MAX_HEIGHT_FRACTION: f32 = 0.8;
const SHORT_ID_LENGTH: usize = 12;
const TICKET_ROWS: usize = 3;
const TICKET_WARNING: &str = "Anyone with this ticket can join your notes. Share it privately.";
const JOIN_NOTE: &str = "Where a note here differs from the group's, both versions are kept.";
const MODE_NOTE: &str = "Internet mode contacts the public discovery and relay servers of the \
    iroh project: they see device ids and network addresses, never your notes. \
    LAN only contacts nothing outside your network.";
const ELSEWHERE_NOTE: &str = "Sync is handled by another process (for example `sb sync run`).";
const JOIN_HINT: &str = "Paste a ticket";

/// What the user did in the window this frame.
pub struct SyncWindowOutcome {
    /// What to ask the sync worker for.
    pub request: Option<Request>,
    /// The window should close (Escape, or a click outside it).
    pub close: bool,
}

/// Shows the window over the application.
pub fn show(
    ctx: &Context,
    p: &Palette,
    panel: &mut SyncPanel,
    reduce_motion: bool,
) -> SyncWindowOutcome {
    let frame = theme::document_frame(p).inner_margin(Margin::same(PADDING));
    let height = ctx.content_rect().height() * MAX_HEIGHT_FRACTION;
    let modal = Modal::new(Id::unique("sync_window"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ScrollArea::vertical()
                .max_height(height)
                .show(ui, |ui| contents(ui, p, panel, reduce_motion))
                .inner
        });
    let close = modal.should_close();
    SyncWindowOutcome {
        request: modal.inner,
        close,
    }
}

fn contents(
    ui: &mut Ui,
    p: &Palette,
    panel: &mut SyncPanel,
    reduce_motion: bool,
) -> Option<Request> {
    ui.heading("Sync");
    ui.label(state_line(&panel.state));
    if let Some(error) = &panel.error {
        ui.label(format!("Error: {error}"));
    }
    if let Some(busy) = panel.busy {
        ui.horizontal(|ui| {
            if !reduce_motion {
                ui.spinner();
            }
            ui.label(busy);
        });
    }
    let read_only = panel.state == SyncState::HandledElsewhere;
    let status = panel.status.clone();
    device_section(ui, p, status.as_ref(), !read_only);
    let request = if read_only {
        None
    } else {
        controls(ui, p, panel)
    };
    if let Some(status) = &status {
        status_sections(ui, p, status);
    }
    request
}

fn state_line(state: &SyncState) -> String {
    match state {
        SyncState::Off => "Sync is off".to_owned(),
        SyncState::Starting => "Sync is starting…".to_owned(),
        SyncState::On => "Sync is on".to_owned(),
        SyncState::HandledElsewhere => ELSEWHERE_NOTE.to_owned(),
        SyncState::Error(error) => format!("Sync is not running: {error}"),
    }
}

fn note(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).color(p.text_secondary.to_color32()));
}

fn device_section(ui: &mut Ui, p: &Palette, status: Option<&Status>, copyable: bool) {
    let Some(status) = status else {
        return;
    };
    theme::section_label(ui, p, "THIS DEVICE");
    ui.horizontal(|ui| {
        ui.label(RichText::new(&status.device_id).monospace());
        if copyable && copy_button(ui, "Copy device id") {
            ui.ctx().copy_text(status.device_id.clone());
        }
    });
}

/// A «Copy» button whose accessible name says what it copies; true when pressed.
fn copy_button(ui: &mut Ui, name: &str) -> bool {
    ui.button("Copy")
        .on_hover_text(name)
        .accessible_name(name)
        .clicked()
}

fn controls(ui: &mut Ui, p: &Palette, panel: &mut SyncPanel) -> Option<Request> {
    if let Some(ticket) = panel.ticket.clone() {
        ticket_field(ui, p, &ticket);
    }
    let enabled = panel.busy.is_none() && panel.state != SyncState::Starting;
    ui.add_enabled_ui(enabled, |ui| {
        let invite = invite_section(ui, p);
        let join = join_section(ui, p, &mut panel.join_text);
        let mode = mode_section(ui, p, panel.mode);
        let off = turn_off_section(ui, &panel.state);
        invite.or(join).or(mode).or(off)
    })
    .inner
}

fn ticket_field(ui: &mut Ui, p: &Palette, ticket: &str) {
    theme::section_label(ui, p, "TICKET");
    let mut shown = ticket;
    let field = TextEdit::multiline(&mut shown)
        .font(TextStyle::Monospace)
        .desired_rows(TICKET_ROWS)
        .desired_width(f32::INFINITY);
    ui.add(field).accessible_name("Invite ticket");
    if copy_button(ui, "Copy ticket") {
        ui.ctx().copy_text(ticket.to_owned());
    }
    note(ui, p, TICKET_WARNING);
}

fn invite_section(ui: &mut Ui, p: &Palette) -> Option<Request> {
    theme::section_label(ui, p, "INVITE");
    ui.button("Create invite")
        .clicked()
        .then_some(Request::Invite)
}

fn join_section(ui: &mut Ui, p: &Palette, text: &mut String) -> Option<Request> {
    theme::section_label(ui, p, "JOIN");
    let field = TextEdit::singleline(text)
        .hint_text(JOIN_HINT)
        .desired_width(f32::INFINITY);
    let response = ui.add(field).accessible_name(JOIN_HINT);
    let entered = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    let ready = !text.trim().is_empty();
    let clicked = ui.add_enabled(ready, Button::new("Join")).clicked();
    note(ui, p, JOIN_NOTE);
    ((clicked || entered) && ready).then(|| Request::Join(text.trim().to_owned()))
}

fn mode_section(ui: &mut Ui, p: &Palette, current: NetMode) -> Option<Request> {
    theme::section_label(ui, p, "NETWORK");
    let mut chosen = None;
    ui.horizontal(|ui| {
        for (mode, name) in [(NetMode::Internet, "Internet"), (NetMode::Lan, "LAN only")] {
            let clicked = ui.selectable_label(current == mode, name).clicked();
            if clicked && current != mode {
                chosen = Some(Request::SetMode(mode));
            }
        }
    });
    note(ui, p, MODE_NOTE);
    chosen
}

fn turn_off_section(ui: &mut Ui, state: &SyncState) -> Option<Request> {
    // An engine that failed to start or ended still leaves a group the user may want to leave.
    if !matches!(state, SyncState::On | SyncState::Error(_)) {
        return None;
    }
    ui.add_space(4.0);
    ui.button("Turn sync off")
        .clicked()
        .then_some(Request::Leave)
}

fn status_sections(ui: &mut Ui, p: &Palette, status: &Status) {
    theme::section_label(ui, p, "DEVICES");
    if status.peers.is_empty() {
        note(ui, p, "No other devices yet");
    }
    for peer in &status.peers {
        let id: String = peer.id.chars().take(SHORT_ID_LENGTH).collect();
        let word = if peer.connected { "online" } else { "offline" };
        ui.horizontal(|ui| {
            ui.label(RichText::new(id).monospace());
            ui.label(word);
        });
    }
    if let Some(relay) = &status.relay {
        ui.label(format!("Relay: {relay}"));
    }
    blocked_section(ui, p, status);
}

fn blocked_section(ui: &mut Ui, p: &Palette, status: &Status) {
    if status.blocked.is_empty() {
        return;
    }
    theme::section_label(ui, p, "NOT SYNCED");
    for note in &status.blocked {
        ui.label(format!("{}: blocked ({})", note.path, note.reason));
    }
}
