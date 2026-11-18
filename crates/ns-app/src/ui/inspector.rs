//! The right panel: backlinks, outgoing links and tags of the open note (spec 9.2).

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{RichText, ScrollArea, TextStyle, Ui};
use ns_core::{Backlink, OutLink};

use crate::state::{InspectorCache, NoteAction, Workspace};
use crate::ui::sidebar::row_button;
use crate::ui::theme::{self, Palette};

/// Draws the three lists; returns what the user clicked.
pub fn show(
    ui: &mut Ui,
    p: &Palette,
    workspace: &Workspace,
    cache: &mut InspectorCache,
) -> Option<NoteAction> {
    let data = cache.data(workspace);
    ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            theme::section_label(ui, p, "BACKLINKS");
            let opened = backlinks(ui, p, &data.backlinks);
            theme::section_label(ui, p, "OUTGOING LINKS");
            let followed = outgoing(ui, p, &data.outgoing);
            theme::section_label(ui, p, "TAGS");
            tags(ui, p, &data.tags);
            opened.or(followed)
        })
        .inner
}

fn none_label(ui: &mut Ui, p: &Palette) {
    ui.label(RichText::new("None").color(p.text_secondary.to_color32()));
}

fn backlinks(ui: &mut Ui, p: &Palette, backlinks: &[Backlink]) -> Option<NoteAction> {
    if backlinks.is_empty() {
        none_label(ui, p);
    }
    let mut action = None;
    for backlink in backlinks {
        let parts = [
            (backlink.from.stem(), p.accent),
            ("\n", p.text_secondary),
            (backlink.context.as_str(), p.text_secondary),
        ];
        if row_button(ui, &parts).clicked() {
            action = Some(NoteAction::Open(backlink.from.clone()));
        }
    }
    action
}

fn outgoing(ui: &mut Ui, p: &Palette, links: &[OutLink]) -> Option<NoteAction> {
    if links.is_empty() {
        none_label(ui, p);
    }
    let mut action = None;
    for out in links {
        let target = out.link.target.as_str();
        if out.link.is_attachment() {
            attachment_row(ui, p, target);
        } else if out.resolved.is_none() {
            let parts = [(target, p.accent), ("  not created yet", p.text_secondary)];
            action = follow_if_clicked(ui, &parts, target).or(action);
        } else {
            action = follow_if_clicked(ui, &[(target, p.accent)], target).or(action);
        }
    }
    action
}

fn follow_if_clicked(
    ui: &mut Ui,
    parts: &[(&str, theme::Rgba)],
    target: &str,
) -> Option<NoteAction> {
    row_button(ui, parts)
        .clicked()
        .then(|| NoteAction::Follow(target.to_owned()))
}

/// An attachment is shown but cannot be followed: it is a file, not a note.
fn attachment_row(ui: &mut Ui, p: &Palette, target: &str) {
    let font = TextStyle::Body.resolve(ui.style());
    let mut job = LayoutJob::default();
    let primary = TextFormat::simple(font.clone(), p.text_primary.to_color32());
    let secondary = TextFormat::simple(font, p.text_secondary.to_color32());
    job.append(target, 0.0, primary);
    job.append("  attachment", 0.0, secondary);
    ui.label(job);
}

fn tags(ui: &mut Ui, p: &Palette, tags: &[String]) {
    if tags.is_empty() {
        none_label(ui, p);
    }
    ui.horizontal_wrapped(|ui| {
        for tag in tags {
            theme::chip_frame(p).show(ui, |ui| ui.label(format!("#{tag}")));
        }
    });
}
