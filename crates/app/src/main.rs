//! MongoStudio binary: owns the Tokio runtime and starts the eframe window.
use mongostudio_core::secrets::{KeyringSecretStore, SecretStore};
use mongostudio_ui::{I18n, app::MongoStudioApp};
use std::sync::Arc;

const DEFAULT_SIZE: [f32; 2] = [1280.0, 800.0];
const MIN_SIZE: [f32; 2] = [800.0, 500.0];

fn main() -> eframe::Result {
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
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size(DEFAULT_SIZE)
            .with_min_inner_size(MIN_SIZE),
        ..Default::default()
    };
    // The repaint closure needs the egui context, which only exists inside this callback.
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            Ok(Box::new(MongoStudioApp::launch(
                secrets,
                cc.egui_ctx.clone(),
            )))
        }),
    )
}
