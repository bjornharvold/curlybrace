//! Material 3 custom widgets drawn from the current scheme: the segmented control, chip, outlined
//! field, sidebar pill, card and the three button styles. Each one reports the role, label and
//! state of the egui widget it replaces to the accessibility tree, and the interactive ones draw
//! a 2 dp `primary` focus ring while they hold keyboard focus.
//!
//! Shared here: interaction state (hover / pressed / focus, or a [`Forced`] state for the theme
//! gallery), Material's disabled opacities, truncated one-row galleys and the focus ring.
mod button;
mod card;
mod chip;
mod outlined_field;
mod pill;
mod segmented;
mod tabs;

#[cfg(test)]
mod widgets_tests;

pub use button::{filled_button, text_button, tonal_button};
pub use card::card;
pub use chip::{CHIP_HEIGHT, chip};
pub use outlined_field::{outlined_field, outlined_field_with};
pub use pill::{PILL_HEIGHT, pill, pill_row};
pub use segmented::segmented;
pub use tabs::tabs;

use super::fonts::MEDIUM_FAMILY;
use super::{StateLayer, state_layer};
use eframe::egui::{
    Color32, CornerRadius, FontFamily, FontId, Galley, Id, Painter, Rect, Response, Stroke,
    StrokeKind, Ui,
    text::{LayoutJob, TextWrapping},
};
use std::sync::Arc;

/// Height of buttons, segmented controls and outlined fields (Material's 40 dp target).
pub(crate) const CONTROL_HEIGHT: f32 = 40.0;
/// Width of the keyboard focus ring.
const FOCUS_RING_WIDTH: f32 = 2.0;
/// Gap between a widget and its focus ring, so the ring stays visible on a `primary` fill.
const FOCUS_RING_OFFSET: u8 = 2;
/// Material's disabled opacities: content 38%, container 12% of `on_surface`.
const DISABLED_CONTENT_PERCENT: u32 = 38;
const DISABLED_CONTAINER_PERCENT: u32 = 12;

/// A widget state forced on every widget drawn inside [`with_forced`], for the theme gallery,
/// which has to show hover, pressed and focus without a real pointer or keyboard.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forced {
    Hover,
    Pressed,
    Focused,
}

fn forced_id() -> Id {
    Id::new("curlybrace.theme.widgets.forced")
}

/// Draws `add` with `forced` applied to every theme widget inside it. Interaction still works;
/// only the look is forced. Meant for previews such as the theme gallery, not for views.
#[doc(hidden)]
pub fn with_forced<R>(ui: &mut Ui, forced: Forced, add: impl FnOnce(&mut Ui) -> R) -> R {
    let id = forced_id();
    let previous = ui.ctx().data(|d| d.get_temp::<Forced>(id));
    ui.ctx().data_mut(|d| d.insert_temp(id, forced));
    let result = add(ui);
    ui.ctx().data_mut(|d| match previous {
        Some(p) => d.insert_temp(id, p),
        None => d.remove::<Forced>(id),
    });
    result
}

fn forced(ui: &Ui) -> Option<Forced> {
    ui.ctx().data(|d| d.get_temp::<Forced>(forced_id()))
}

/// How a widget looks this frame: its state layer (if any) and whether it shows focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub layer: Option<StateLayer>,
    pub focused: bool,
    pub enabled: bool,
}

impl Look {
    /// From the response (pressed over focus over hover), or the [`Forced`] state.
    pub fn of(ui: &Ui, response: &Response) -> Self {
        let enabled = response.enabled();
        let forced = if enabled { forced(ui) } else { None };
        let focused = response.has_focus() || forced == Some(Forced::Focused);
        let pressed = response.is_pointer_button_down_on() || forced == Some(Forced::Pressed);
        let hovered = response.hovered() || forced == Some(Forced::Hover);
        let layer = if !enabled {
            None
        } else if pressed {
            Some(StateLayer::Pressed)
        } else if focused {
            Some(StateLayer::Focus)
        } else if hovered {
            Some(StateLayer::Hover)
        } else {
            None
        };
        Self {
            layer,
            focused: focused && enabled,
            enabled,
        }
    }

    /// `base` with this look's state layer of `on` over it (`base` when there is none).
    pub fn fill(self, base: Color32, on: Color32) -> Color32 {
        match self.layer {
            Some(layer) => state_layer(base, on, layer),
            None => base,
        }
    }

    /// The translucent state layer of `on` for a widget with no container of its own.
    pub fn overlay(self, on: Color32) -> Color32 {
        match self.layer {
            Some(layer) => with_alpha(on, layer.percent()),
            None => Color32::TRANSPARENT,
        }
    }
}

/// `color` at `percent` opacity.
pub(crate) fn with_alpha(color: Color32, percent: u32) -> Color32 {
    let alpha = (percent.min(100) * 255 + 50) / 100;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha as u8)
}

/// Material's disabled content color (38% of `on_surface`).
pub(crate) fn disabled_content(on_surface: Color32) -> Color32 {
    with_alpha(on_surface, DISABLED_CONTENT_PERCENT)
}

/// Material's disabled container color (12% of `on_surface`).
pub(crate) fn disabled_container(on_surface: Color32) -> Color32 {
    with_alpha(on_surface, DISABLED_CONTAINER_PERCENT)
}

/// The painter for a widget. egui fades a disabled `Ui`'s painter on top of whatever is drawn;
/// these widgets apply Material's own disabled opacities instead, so the fade is undone.
pub(crate) fn painter(ui: &Ui) -> Painter {
    let mut painter = ui.painter().clone();
    if !ui.is_enabled() {
        painter.set_opacity(1.0);
    }
    painter
}

/// Where the focus ring goes relative to the widget's rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ring {
    /// 2 dp outside the rect, so it shows on a `primary` fill (widgets sized to their content).
    Outside,
    /// On the rect's inner edge, for widgets that take the full available width: anything
    /// outside them would fall outside the parent's clip rect (a side panel, a scroll area).
    Inside,
}

/// The 2 dp `primary` ring drawn while a widget holds focus, placed as `ring` says.
pub(crate) fn focus_ring(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    look: Look,
    primary: Color32,
    ring: Ring,
) {
    if !look.focused {
        return;
    }
    let stroke = Stroke::new(FOCUS_RING_WIDTH, primary);
    match ring {
        Ring::Inside => {
            painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
        }
        Ring::Outside => {
            let offset = FOCUS_RING_OFFSET;
            let ring_radius = CornerRadius {
                nw: radius.nw.saturating_add(offset),
                ne: radius.ne.saturating_add(offset),
                sw: radius.sw.saturating_add(offset),
                se: radius.se.saturating_add(offset),
            };
            let outer = rect.expand(f32::from(offset));
            painter.rect_stroke(outer, ring_radius, stroke, StrokeKind::Outside);
        }
    }
}

/// `ui.available_width()`, or `fallback` when it is unbounded (a horizontal scroll area), so a
/// width-filling widget never allocates an infinite rect.
pub(crate) fn finite_width(ui: &Ui, fallback: f32) -> f32 {
    let width = ui.available_width();
    if width.is_finite() { width } else { fallback }
}

/// Roboto Medium at `size` when the theme's fonts are installed, else the proportional font
/// (a bare test context without [`super::install`] has no Medium family).
pub(crate) fn medium_font(ui: &Ui, size: f32) -> FontId {
    let family = FontFamily::Name(MEDIUM_FAMILY.into());
    let installed = ui.fonts(|f| f.lock().fonts.definitions().families.contains_key(&family));
    if installed {
        FontId::new(size, family)
    } else {
        FontId::proportional(size)
    }
}

/// `text` on one row no wider than `max_width`, elided with an ellipsis when it does not fit.
/// The color is a placeholder: pass the real one to `Painter::galley`. egui caches the layout.
pub(crate) fn one_row(ui: &Ui, text: &str, font: FontId, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, Color32::PLACEHOLDER);
    job.wrap = TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.painter().layout_job(job)
}
