//! Installs the Material 3 theme into egui and exposes the active scheme.
use super::shape::{self, radius};
use super::{Mode, Scheme, font_definitions, scheme, typography};
use eframe::egui::{
    Color32, Context, Shadow, Stroke, Theme, ThemePreference, Ui, Vec2, Visuals,
    style::{Selection, WidgetVisuals},
};

/// Vertical gap between rows (horizontal gaps stay on the 8 pt grid).
const ROW_GAP: f32 = 6.0;

/// Interaction overlays from Material's state-layer spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateLayer {
    Hover,
    Focus,
    Pressed,
}

impl StateLayer {
    /// Overlay opacity in 1/100: hover 8%, focus 10%, pressed 10%.
    pub(crate) fn percent(self) -> u32 {
        match self {
            Self::Hover => 8,
            Self::Focus | Self::Pressed => 10,
        }
    }
}

/// `base` with `on` laid over it at the state's opacity (opaque result).
pub fn state_layer(base: Color32, on: Color32, state: StateLayer) -> Color32 {
    let a = state.percent();
    let mix = |b: u8, o: u8| ((u32::from(b) * (100 - a) + u32::from(o) * a + 50) / 100) as u8;
    Color32::from_rgb(
        mix(base.r(), on.r()),
        mix(base.g(), on.g()),
        mix(base.b(), on.b()),
    )
}

/// Follows the system appearance and installs both modes' visuals and the shared style.
/// Call again at any time; egui picks the visuals for the current system theme each frame.
pub fn install(ctx: &Context) {
    ctx.set_fonts(font_definitions());
    ctx.options_mut(|o| o.theme_preference = ThemePreference::System);
    for (theme, mode) in [(Theme::Light, Mode::Light), (Theme::Dark, Mode::Dark)] {
        ctx.set_visuals_of(theme, visuals_for(scheme(mode)));
        ctx.style_mut_of(theme, |style| {
            style.text_styles = typography::text_styles();
            style.spacing.item_spacing = Vec2::new(shape::SPACE_2, ROW_GAP);
            style.spacing.button_padding = Vec2::new(shape::SPACE_3, shape::SPACE_1);
        });
    }
}

/// The scheme of the egui theme currently in effect.
pub fn current(ctx: &Context) -> &'static Scheme {
    scheme(mode_of(ctx.theme()))
}

/// [`current`] from inside a `Ui`.
pub fn current_ui(ui: &Ui) -> &'static Scheme {
    current(ui.ctx())
}

fn mode_of(theme: Theme) -> Mode {
    match theme {
        Theme::Light => Mode::Light,
        Theme::Dark => Mode::Dark,
    }
}

/// egui visuals built from a scheme: surfaces from the container levels, widgets with
/// state-layer fills, selection from the secondary container.
pub fn visuals_for(s: &Scheme) -> Visuals {
    let mut v = match s.mode() {
        Mode::Light => Visuals::light(),
        Mode::Dark => Visuals::dark(),
    };
    v.override_text_color = None;
    v.panel_fill = s.surface_container_low;
    v.window_fill = s.surface_container_high;
    v.faint_bg_color = s.surface_container;
    v.extreme_bg_color = s.surface;
    v.code_bg_color = s.surface_container;
    v.hyperlink_color = s.primary;
    v.warn_fg_color = s.warning;
    v.error_fg_color = s.error;
    v.window_stroke = Stroke::new(1.0, s.outline_variant);
    v.window_corner_radius = radius(shape::EXTRA_LARGE);
    v.menu_corner_radius = radius(shape::SMALL);
    v.window_shadow = light_shadow(s, 12, 24);
    v.popup_shadow = light_shadow(s, 6, 16);
    v.selection = Selection {
        bg_fill: s.secondary_container,
        stroke: Stroke::new(1.0, s.primary),
    };
    let w = &mut v.widgets;
    w.noninteractive = widget(s.surface_container_low, s.outline_variant, s.on_surface);
    w.inactive = widget(s.surface_container, s.outline_variant, s.on_surface);
    w.hovered = layered(s, StateLayer::Hover, s.outline);
    w.active = layered(s, StateLayer::Pressed, s.primary);
    w.open = layered(s, StateLayer::Focus, s.outline);
    v
}

fn widget(fill: Color32, line: Color32, text: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, line),
        // Small, so egui's checkbox box stays square (a circle reads as a radio button).
        corner_radius: radius(shape::EXTRA_SMALL),
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// An interactive widget: the resting fill with the state layer of `on_surface` over it.
fn layered(s: &Scheme, state: StateLayer, line: Color32) -> WidgetVisuals {
    let fill = state_layer(s.surface_container, s.on_surface, state);
    widget(fill, line, s.on_surface)
}

fn light_shadow(s: &Scheme, blur: u8, alpha: u8) -> Shadow {
    Shadow {
        offset: [0, 2],
        blur,
        spread: 0,
        color: Color32::from_rgba_unmultiplied(s.shadow.r(), s.shadow.g(), s.shadow.b(), alpha),
    }
}
