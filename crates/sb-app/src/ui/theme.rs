//! Glass look: colour tokens, fonts and frames (spec 9.6, appendix A.5).
//!
//! The only place in `src/ui/` where colour literals, fonts, radii and shadows are defined.

use eframe::egui::epaint::Shadow;
use eframe::egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, LayerId,
    Margin, Mesh, Painter, Rect, RichText, Stroke, Style, TextStyle, Theme, Ui,
};
use std::sync::Arc;

/// Gap between panels, in points.
pub const GUTTER: f32 = 8.0;
/// Corner radius of panels and the document surface, in points.
pub const RADIUS: u8 = 12;
const CHIP_RADIUS: u8 = 8;
const SECTION_LABEL_SIZE: f32 = 11.0;
const SECTION_LABEL_SPACING: f32 = 1.1;

/// A colour with straight (not premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha, 255 is opaque.
    pub a: u8,
}

/// Builds an [`Rgba`].
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba { r, g, b, a }
}

impl Rgba {
    /// The same colour as an egui (premultiplied) colour.
    pub fn to_color32(self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.r, self.g, self.b, self.a)
    }
}

/// What sits behind the glass panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// The Windows Mica material behind a transparent window.
    Mica,
    /// A neutral gradient painted by the app.
    Simulated,
    /// The same gradient, with every glass panel fully opaque.
    Opaque,
}

/// Every colour of one theme and material (spec 9.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Fill of glass panels.
    pub glass: Rgba,
    /// Fill of the solid document surface.
    pub document: Rgba,
    /// Primary text.
    pub text_primary: Rgba,
    /// Secondary text.
    pub text_secondary: Rgba,
    /// Links and selected items.
    pub accent: Rgba,
    /// 1 px border of glass panels.
    pub hairline: Rgba,
    /// 1 px line along the top edge of glass panels.
    pub highlight: Rgba,
    /// Fill of a hovered widget.
    pub hover: Rgba,
    /// Fill of the selected item.
    pub selection: Rgba,
    /// Links drawn in the graph: at least 3 : 1 against the document surface (WCAG 1.4.11).
    pub edge: Rgba,
    /// Top of the simulated backdrop gradient.
    pub backdrop_top: Rgba,
    /// Bottom of the simulated backdrop gradient.
    pub backdrop_bottom: Rgba,
    /// Shadow of the document surface.
    pub shadow: Shadow,
}

const SELECTION: Rgba = rgba(0x0A, 0x84, 0xFF, 46);

fn shadow(alpha: u8) -> Shadow {
    Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: rgba(0, 0, 0, alpha).to_color32(),
    }
}

/// The colour tokens of spec 9.6 for `theme` and `material`.
pub fn palette(theme: Theme, material: Material) -> Palette {
    match theme {
        Theme::Light => light(match material {
            Material::Mica => 194,
            Material::Simulated => 153,
            Material::Opaque => 255,
        }),
        Theme::Dark => dark(match material {
            Material::Mica => 209,
            Material::Simulated => 153,
            Material::Opaque => 255,
        }),
    }
}

fn light(glass_alpha: u8) -> Palette {
    Palette {
        glass: rgba(0xFF, 0xFF, 0xFF, glass_alpha),
        document: rgba(0xFF, 0xFF, 0xFF, 255),
        text_primary: rgba(0x1C, 0x1C, 0x1E, 255),
        text_secondary: rgba(0x3C, 0x3C, 0x43, 255),
        accent: rgba(0x09, 0x40, 0xA0, 255),
        hairline: rgba(255, 255, 255, 89),
        highlight: rgba(255, 255, 255, 128),
        hover: rgba(0, 0, 0, 15),
        selection: SELECTION,
        edge: rgba(0x8A, 0x8A, 0x90, 255),
        backdrop_top: rgba(0xF2, 0xF2, 0xF4, 255),
        backdrop_bottom: rgba(0xE9, 0xE9, 0xEC, 255),
        shadow: shadow(20),
    }
}

fn dark(glass_alpha: u8) -> Palette {
    Palette {
        glass: rgba(0x2C, 0x2C, 0x30, glass_alpha),
        document: rgba(0x23, 0x23, 0x25, 255),
        text_primary: rgba(0xF5, 0xF5, 0xF7, 255),
        text_secondary: rgba(0xC8, 0xC8, 0xD2, 255),
        accent: rgba(0xB8, 0xD3, 0xFF, 255),
        hairline: rgba(255, 255, 255, 20),
        highlight: rgba(255, 255, 255, 26),
        hover: rgba(255, 255, 255, 15),
        selection: SELECTION,
        edge: rgba(0x77, 0x77, 0x7D, 255),
        backdrop_top: rgba(0x1E, 0x1E, 0x20, 255),
        backdrop_bottom: rgba(0x18, 0x18, 0x1A, 255),
        shadow: shadow(77),
    }
}

/// `top` drawn over `bottom` (source-over); the result is opaque.
pub fn composite(top: Rgba, bottom: Rgba) -> Rgba {
    let alpha = f64::from(top.a) / 255.0;
    let mix = |t: u8, b: u8| (f64::from(t) * alpha + f64::from(b) * (1.0 - alpha)).round() as u8;
    rgba(
        mix(top.r, bottom.r),
        mix(top.g, bottom.g),
        mix(top.b, bottom.b),
        255,
    )
}

fn relative_luminance(c: Rgba) -> f64 {
    let linear = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

/// WCAG 2 contrast ratio between two opaque colours, from 1 to 21.
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Installs the text styles and visuals of `palette` for `theme`.
pub fn apply(ctx: &Context, theme: Theme, palette: &Palette) {
    ctx.style_mut_of(theme, |style| {
        set_text_styles(style);
        set_visuals(style, palette);
    });
}

fn set_text_styles(style: &mut Style) {
    let proportional = |size| FontId::new(size, FontFamily::Proportional);
    style
        .text_styles
        .insert(TextStyle::Body, proportional(15.5));
    style
        .text_styles
        .insert(TextStyle::Button, proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Small, proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(22.0, bold_family()));
}

fn set_visuals(style: &mut Style, p: &Palette) {
    let v = &mut style.visuals;
    v.panel_fill = Color32::TRANSPARENT;
    // Menus, context menus and dialogs draw on this: solid like the document surface.
    v.window_fill = p.document.to_color32();
    v.hyperlink_color = p.accent.to_color32();
    v.selection.bg_fill = p.selection.to_color32();
    // Also the text colour of a selected label and the focus ring of text fields (appendix A.5).
    v.selection.stroke = Stroke::new(2.0, p.accent.to_color32());
    v.widgets.noninteractive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.hovered.weak_bg_fill = p.hover.to_color32();
    v.widgets.hovered.bg_fill = p.hover.to_color32();
    // egui draws keyboard focus with the `active` visuals: the ring is a 2 px accent stroke.
    v.widgets.active.bg_stroke = Stroke::new(2.0, p.accent.to_color32());
    v.widgets.active.weak_bg_fill = p.selection.to_color32();
    v.widgets.open.weak_bg_fill = p.selection.to_color32();
    let text = Stroke::new(1.0, p.text_primary.to_color32());
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.fg_stroke = text;
    }
}

/// The font family that holds Inter SemiBold.
pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Inter Regular first in the proportional family, SemiBold as the family `"bold"`.
pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let regular = FontData::from_static(include_bytes!("../../assets/Inter-Regular.ttf"));
    let semi_bold = FontData::from_static(include_bytes!("../../assets/Inter-SemiBold.ttf"));
    fonts.font_data.insert("Inter".into(), Arc::new(regular));
    fonts
        .font_data
        .insert("Inter-SemiBold".into(), Arc::new(semi_bold));
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter".into());
    let mut bold = vec!["Inter-SemiBold".to_owned()];
    bold.extend(proportional.iter().cloned());
    fonts.families.insert(bold_family(), bold);
    ctx.set_fonts(fonts);
}

/// Frame of a glass panel: translucent fill, hairline, no shadow.
pub fn glass_frame(p: &Palette) -> Frame {
    Frame::NONE
        .fill(p.glass.to_color32())
        .stroke(Stroke::new(1.0, p.hairline.to_color32()))
        .corner_radius(CornerRadius::same(RADIUS))
}

/// Draws a section label such as «FILES»: 11 px, letter-spaced, in the secondary colour.
pub fn section_label(ui: &mut Ui, p: &Palette, text: &str) {
    let label = RichText::new(text)
        .size(SECTION_LABEL_SIZE)
        .extra_letter_spacing(SECTION_LABEL_SPACING)
        .color(p.text_secondary.to_color32());
    ui.label(label);
}

/// Frame of a tag chip: a hover-coloured pill.
pub fn chip_frame(p: &Palette) -> Frame {
    Frame::NONE
        .fill(p.hover.to_color32())
        .corner_radius(CornerRadius::same(CHIP_RADIUS))
        .inner_margin(Margin::symmetric(8, 2))
}

/// Frame of the slim status strip: the glass fill, no border, no radius, no shadow.
pub fn strip_frame(p: &Palette) -> Frame {
    Frame::NONE.fill(p.glass.to_color32())
}

/// Frame of the solid document surface: opaque fill, soft shadow, no border.
pub fn document_frame(p: &Palette) -> Frame {
    Frame::NONE
        .fill(p.document.to_color32())
        .corner_radius(CornerRadius::same(RADIUS))
        .shadow(p.shadow)
}

/// Draws the 1 px highlight along the top edge of a glass panel, inset by the corner radius.
pub fn paint_top_highlight(painter: &Painter, rect: Rect, p: &Palette) {
    let y = rect.top() + 1.5;
    let inset = f32::from(RADIUS);
    let line = [
        (rect.left() + inset, y).into(),
        (rect.right() - inset, y).into(),
    ];
    painter.line_segment(line, Stroke::new(1.0, p.highlight.to_color32()));
}

/// Paints the vertical backdrop gradient over the whole screen, behind every panel.
pub fn paint_backdrop(ctx: &Context, p: &Palette) {
    let screen = ctx.content_rect();
    let (top, bottom) = (p.backdrop_top.to_color32(), p.backdrop_bottom.to_color32());
    let mut mesh = Mesh::default();
    mesh.colored_vertex(screen.left_top(), top);
    mesh.colored_vertex(screen.right_top(), top);
    mesh.colored_vertex(screen.right_bottom(), bottom);
    mesh.colored_vertex(screen.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ctx.layer_painter(LayerId::background()).add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Theme;

    const WHITE: Rgba = rgba(255, 255, 255, 255);
    const BLACK: Rgba = rgba(0, 0, 0, 255);
    const THEMES: [Theme; 2] = [Theme::Light, Theme::Dark];
    const MATERIALS: [Material; 3] = [Material::Mica, Material::Simulated, Material::Opaque];

    #[test]
    fn should_match_spec_tokens_when_building_palettes() {
        let light_simulated = palette(Theme::Light, Material::Simulated);
        assert_eq!(light_simulated.glass, rgba(255, 255, 255, 153));
        assert_eq!(
            palette(Theme::Dark, Material::Mica).glass,
            rgba(0x2C, 0x2C, 0x30, 209)
        );
        assert_eq!(palette(Theme::Light, Material::Opaque).glass.a, 255);
        assert_eq!(
            palette(Theme::Dark, Material::Simulated).document,
            rgba(0x23, 0x23, 0x25, 255)
        );
        assert_eq!(light_simulated.accent, rgba(0x09, 0x40, 0xA0, 255));
        assert_eq!(
            palette(Theme::Dark, Material::Opaque).backdrop_bottom,
            rgba(0x18, 0x18, 0x1A, 255)
        );
    }

    #[test]
    fn should_compute_known_contrast_ratios() {
        assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 1e-9);
        let grey = rgba(0x77, 0x77, 0x77, 255);
        assert!((contrast_ratio(grey, WHITE) - 4.48).abs() < 0.01);
    }

    /// Opaque surfaces a glass panel can show text on, with a name for failure messages.
    fn glass_surfaces(p: &Palette, material: Material) -> Vec<(&'static str, Rgba)> {
        let backgrounds = match material {
            Material::Mica => [("white", WHITE), ("black", BLACK)],
            _ => [
                ("backdrop top", p.backdrop_top),
                ("backdrop bottom", p.backdrop_bottom),
            ],
        };
        backgrounds
            .iter()
            .map(|&(name, below)| (name, composite(p.glass, below)))
            .collect()
    }

    /// Every (description, text, surface) combination spec 9.6 requires to reach 4.5 : 1.
    fn text_pairs(theme: Theme, material: Material) -> Vec<(String, Rgba, Rgba)> {
        let p = palette(theme, material);
        let glass = glass_surfaces(&p, material);
        let texts = [
            ("primary", p.text_primary),
            ("secondary", p.text_secondary),
            ("accent", p.accent),
        ];
        let mut pairs = Vec::new();
        for (text_name, text) in texts {
            pairs.push((format!("{text_name} on document"), text, p.document));
            for &(name, surface) in &glass {
                pairs.push((format!("{text_name} on glass over {name}"), text, surface));
            }
        }
        for &(name, surface) in &glass {
            let description = format!("accent on selection over glass over {name}");
            pairs.push((description, p.accent, composite(p.selection, surface)));
            let description = format!("primary on selection over glass over {name}");
            pairs.push((description, p.text_primary, composite(p.selection, surface)));
        }
        let on_document = composite(p.selection, p.document);
        let description = "primary on selection over document".to_owned();
        pairs.push((description, p.text_primary, on_document));
        pairs
    }

    #[test]
    fn should_draw_every_widget_state_text_in_primary_colour() {
        let ctx = Context::default();
        for theme in THEMES {
            for material in MATERIALS {
                let p = palette(theme, material);
                apply(&ctx, theme, &p);
                let style = ctx.style_of(theme);
                let widgets = &style.visuals.widgets;
                for (state, widget) in [
                    ("noninteractive", &widgets.noninteractive),
                    ("inactive", &widgets.inactive),
                    ("hovered", &widgets.hovered),
                    ("active", &widgets.active),
                    ("open", &widgets.open),
                ] {
                    assert_eq!(
                        widget.fg_stroke.color,
                        p.text_primary.to_color32(),
                        "{theme:?} {material:?}: {state} text is outside the palette"
                    );
                }
            }
        }
    }

    #[test]
    fn should_draw_focus_ring_and_pressed_fills_from_the_palette() {
        let ctx = Context::default();
        for theme in THEMES {
            let p = palette(theme, Material::Simulated);
            apply(&ctx, theme, &p);
            let style = ctx.style_of(theme);
            let widgets = &style.visuals.widgets;
            let ring = Stroke::new(2.0, p.accent.to_color32());
            assert_eq!(widgets.active.bg_stroke, ring, "{theme:?}: focus ring");
            let fill = p.selection.to_color32();
            assert_eq!(widgets.active.weak_bg_fill, fill, "{theme:?}: focused fill");
            assert_eq!(widgets.open.weak_bg_fill, fill, "{theme:?}: open menu fill");
        }
    }

    #[test]
    fn should_fill_popups_and_dialogs_with_the_opaque_document_colour() {
        let ctx = Context::default();
        for theme in THEMES {
            let p = palette(theme, Material::Mica);
            apply(&ctx, theme, &p);
            let fill = ctx.style_of(theme).visuals.window_fill;
            assert_eq!(fill, p.document.to_color32(), "{theme:?}");
        }
    }

    #[test]
    fn should_draw_graph_edges_with_non_text_contrast_of_three_on_the_document() {
        for theme in THEMES {
            for material in MATERIALS {
                let p = palette(theme, material);
                assert_eq!(p.edge.a, 255, "{theme:?} {material:?}: edges are opaque");
                let ratio = contrast_ratio(p.edge, p.document);
                assert!(
                    ratio >= 3.0,
                    "{theme:?} {material:?}: edge contrast {ratio:.2}"
                );
            }
        }
    }

    #[test]
    fn should_pass_wcag_aa_for_every_text_pair() {
        for theme in THEMES {
            for material in MATERIALS {
                for (description, text, surface) in text_pairs(theme, material) {
                    let ratio = contrast_ratio(text, surface);
                    assert!(
                        ratio >= 4.5,
                        "{theme:?} {material:?}: {description} has contrast {ratio:.2}, needs 4.5"
                    );
                }
            }
        }
    }
}
