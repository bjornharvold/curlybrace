//! Draws a connection's colour glyph and production badge. The glyph carries the colour's
//! translated name as its accessible label, and production is text, so colour is never alone.
use crate::I18n;
use crate::theme::{Tone, chip};
use crate::viewmodel::connection_tag::{PRODUCTION_BADGE_KEY, READ_ONLY_BADGE_KEY};
use crate::viewmodel::{ColorTag, ConnectionBadge};
use eframe::egui;
use eframe::egui::{WidgetInfo, WidgetType};

/// Shows nothing for a connection with neither a colour nor the production flag.
pub fn show_badge(ui: &mut egui::Ui, badge: &ConnectionBadge, i18n: &I18n) {
    if let Some(tag) = &badge.color {
        show_glyph(ui, tag, i18n);
    }
    if badge.production {
        chip(ui, Tone::Bad, &i18n.t(PRODUCTION_BADGE_KEY));
    }
    if badge.read_only {
        chip(ui, Tone::Warning, &i18n.t(READ_ONLY_BADGE_KEY));
    }
}

fn show_glyph(ui: &mut egui::Ui, tag: &ColorTag, i18n: &I18n) {
    let [r, g, b] = tag.rgb;
    let glyph = egui::RichText::new(tag.glyph).color(egui::Color32::from_rgb(r, g, b));
    let name = i18n.t(tag.name_key);
    let response = ui.label(glyph);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &name));
}
