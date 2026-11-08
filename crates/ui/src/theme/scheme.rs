//! Material 3 color schemes (light and dark).
//!
//! Seed: green (`#2C6B30` primary). The mockup-given roles (primaries, containers,
//! backgrounds, surface low/high, outlines) are verbatim; the remaining roles follow
//! Material 3's tone mapping for a green seed and are hand-written, arbitrated by the
//! AA contrast tests. They can be regenerated later with Material Theme Builder.
use eframe::egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warning,
    Bad,
    Neutral,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToneColors {
    pub fg: Color32,
    pub container: Color32,
    pub on_container: Color32,
}

#[derive(Clone, Debug)]
pub struct Scheme {
    mode: Mode,
    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,
    pub background: Color32,
    pub surface: Color32,
    pub surface_container_low: Color32,
    pub surface_container: Color32,
    pub surface_container_high: Color32,
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,
    pub error: Color32,
    pub on_error: Color32,
    pub error_container: Color32,
    pub on_error_container: Color32,
    pub success: Color32,
    pub on_success: Color32,
    pub success_container: Color32,
    pub on_success_container: Color32,
    pub warning: Color32,
    pub on_warning: Color32,
    pub warning_container: Color32,
    pub on_warning_container: Color32,
    pub shadow: Color32,
}

const fn hex(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

static LIGHT: Scheme = Scheme {
    mode: Mode::Light,
    primary: hex(0x2C6B30),
    on_primary: hex(0xFFFFFF),
    primary_container: hex(0xB0F2AC),
    on_primary_container: hex(0x002106),
    secondary_container: hex(0xD4E8CF),
    on_secondary_container: hex(0x101F10),
    background: hex(0xFBFDF6),
    surface: hex(0xFBFDF6),
    surface_container_low: hex(0xF0F5EA),
    surface_container: hex(0xEAEFE4),
    surface_container_high: hex(0xE4E9DE),
    on_surface: hex(0x191D17),
    on_surface_variant: hex(0x424940),
    outline: hex(0x72796F),
    outline_variant: hex(0xC2C9BD),
    error: hex(0xBA1A1A),
    on_error: hex(0xFFFFFF),
    error_container: hex(0xFFDAD6),
    on_error_container: hex(0x410002),
    success: hex(0x386A20),
    on_success: hex(0xFFFFFF),
    success_container: hex(0xB8F397),
    on_success_container: hex(0x072100),
    warning: hex(0x825500),
    on_warning: hex(0xFFFFFF),
    warning_container: hex(0xFFDEA6),
    on_warning_container: hex(0x281900),
    shadow: hex(0x000000),
};

static DARK: Scheme = Scheme {
    mode: Mode::Dark,
    primary: hex(0x8BD88A),
    on_primary: hex(0x00390F),
    primary_container: hex(0x1E5127),
    on_primary_container: hex(0xB0F2AC),
    secondary_container: hex(0x3A4B39),
    on_secondary_container: hex(0xD4E8CF),
    background: hex(0x10140F),
    surface: hex(0x10140F),
    surface_container_low: hex(0x181D17),
    surface_container: hex(0x1C211B),
    surface_container_high: hex(0x262B24),
    on_surface: hex(0xE0E4DB),
    on_surface_variant: hex(0xC2C9BD),
    outline: hex(0x8B9388),
    outline_variant: hex(0x414940),
    error: hex(0xFFB4AB),
    on_error: hex(0x690005),
    error_container: hex(0x93000A),
    on_error_container: hex(0xFFDAD6),
    success: hex(0xA1D78A),
    on_success: hex(0x0F3800),
    success_container: hex(0x225106),
    on_success_container: hex(0xBDF3A0),
    warning: hex(0xF7BD48),
    on_warning: hex(0x422C00),
    warning_container: hex(0x5F4100),
    on_warning_container: hex(0xFFDEA6),
    shadow: hex(0x000000),
};

/// The color scheme for a mode.
pub fn scheme(mode: Mode) -> &'static Scheme {
    match mode {
        Mode::Light => &LIGHT,
        Mode::Dark => &DARK,
    }
}

impl Scheme {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Semantic status colors: foreground, tonal container and the text on it.
    pub fn tone(&self, tone: Tone) -> ToneColors {
        match tone {
            Tone::Good => ToneColors {
                fg: self.success,
                container: self.success_container,
                on_container: self.on_success_container,
            },
            Tone::Warning => ToneColors {
                fg: self.warning,
                container: self.warning_container,
                on_container: self.on_warning_container,
            },
            Tone::Bad => ToneColors {
                fg: self.error,
                container: self.error_container,
                on_container: self.on_error_container,
            },
            Tone::Neutral => ToneColors {
                fg: self.on_surface_variant,
                container: self.surface_container_high,
                on_container: self.on_surface,
            },
        }
    }
}
