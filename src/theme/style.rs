//! Translate an [`M3Palette`] into an egui [`Style`].

use super::palette::{blend, M3Palette};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke,
    Style, TextStyle, Visuals,
};
use std::sync::Arc;

pub const INTER: &str = "Inter";
pub const INTER_MEDIUM: &str = "Inter Medium";

/// Font family for M3 "title" and "label" styles.
pub fn medium_family() -> FontFamily {
    FontFamily::Name(INTER_MEDIUM.into())
}

/// Register Inter (regular for body text, medium for titles) plus the
/// Material Symbols fonts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        INTER.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-Regular.otf"
        ))),
    );
    fonts.font_data.insert(
        INTER_MEDIUM.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-Medium.otf"
        ))),
    );
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, INTER.to_owned());
    let mut medium = vec![INTER_MEDIUM.to_owned()];
    medium.extend(fallbacks);
    fonts.families.insert(medium_family(), medium);
    ctx.set_fonts(fonts);
    egui_material_icons::initialize(ctx);
}

pub fn apply(ctx: &egui::Context, p: &M3Palette) {
    let dark = p.is_dark();
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.override_text_color = Some(p.on_surface);
    visuals.weak_text_color = Some(p.on_surface_variant);
    visuals.hyperlink_color = p.primary;
    visuals.panel_fill = p.surface;
    visuals.window_fill = p.surface_container_high;
    visuals.window_stroke = Stroke::NONE;
    visuals.window_corner_radius = CornerRadius::same(28);
    visuals.menu_corner_radius = CornerRadius::same(12);
    visuals.faint_bg_color = p.surface_container_low;
    visuals.extreme_bg_color = p.surface_container_highest;
    visuals.text_edit_bg_color = Some(p.surface_container_highest);
    visuals.code_bg_color = p.surface_container_highest;
    visuals.warn_fg_color = p.tertiary;
    visuals.error_fg_color = p.error;
    visuals.selection.bg_fill = p.primary.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, p.on_primary_container);
    visuals.text_cursor.stroke = Stroke::new(2.0, p.primary);
    let shadow_color = Color32::from_black_alpha(if dark { 110 } else { 40 });
    visuals.window_shadow = Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: shadow_color,
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 0,
        color: shadow_color,
    };
    visuals.slider_trailing_fill = true;
    visuals.striped = false;

    let radius = CornerRadius::same(10);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = p.surface_container;
    w.noninteractive.weak_bg_fill = p.surface_container;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.outline_variant);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.on_surface);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.surface_container_highest;
    w.inactive.weak_bg_fill = p.surface_container_high;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.on_surface);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = blend(p.surface_container_highest, p.on_surface, 0.08);
    w.hovered.weak_bg_fill = blend(p.surface_container_high, p.on_surface, 0.08);
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.5, p.on_surface);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = blend(p.surface_container_highest, p.on_surface, 0.12);
    w.active.weak_bg_fill = blend(p.surface_container_high, p.on_surface, 0.12);
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(2.0, p.on_surface);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.secondary_container;
    w.open.weak_bg_fill = p.secondary_container;
    w.open.bg_stroke = Stroke::NONE;
    w.open.fg_stroke = Stroke::new(1.0, p.on_secondary_container);
    w.open.corner_radius = radius;

    let mut style = Style {
        visuals,
        ..Style::default()
    };
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Heading, FontId::proportional(22.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.menu_margin = Margin::same(8);
    style.spacing.window_margin = Margin::same(24);
    style.spacing.menu_width = 220.0;
    style.spacing.combo_height = 300.0;
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.animation_time = 0.12;

    let theme = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.set_style_of(theme, style);
}
