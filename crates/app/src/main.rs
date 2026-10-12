//! Curlybrace binary: owns the Tokio runtime and starts the eframe window.
mod logging;

use curlybrace_core::secrets::{KeyringSecretStore, SecretStore};
use curlybrace_ui::{I18n, app::CurlybraceApp};
use std::sync::Arc;

const DEFAULT_SIZE: [f32; 2] = [1280.0, 800.0];
const MIN_SIZE: [f32; 2] = [800.0, 500.0];

fn main() -> eframe::Result {
    logging::init();
    // The runtime must outlive the window; the core runtime spawns onto `Handle::current()`.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("could not start the async runtime: {e}");
            std::process::exit(1);
        }
    };
    let _enter = runtime.enter();
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new());
    let title = I18n::english().t("app.title");
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(title.clone())
        .with_inner_size(DEFAULT_SIZE)
        .with_min_inner_size(MIN_SIZE);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    // The repaint closure needs the egui context, which only exists inside this callback.
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            Ok(Box::new(CurlybraceApp::launch(
                secrets,
                cc.egui_ctx.clone(),
            )))
        }),
    )
}

/// The window and Dock icon, embedded at build time (`scripts/make-icon.sh` regenerates the PNG).
/// A broken icon must never stop the app from starting, so failure only logs.
fn window_icon() -> Option<eframe::egui::IconData> {
    let png = include_bytes!("../../../assets/icon/app-icon-256.png");
    match eframe::icon_data::from_png_bytes(png) {
        Ok(icon) => Some(icon),
        Err(e) => {
            log::warn!("could not load the window icon: {e}");
            None
        }
    }
}

#[cfg(test)]
mod icon_tests {
    use super::window_icon;

    #[test]
    fn the_embedded_window_icon_decodes_as_a_256_pixel_square() {
        let icon = window_icon().expect("the embedded icon PNG must decode");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert!(
            icon.rgba.chunks(4).any(|px| px[3] == 255),
            "the icon must contain opaque pixels"
        );
    }
}
