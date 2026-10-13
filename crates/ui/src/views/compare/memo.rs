//! Per-frame caching of derived data in egui memory: a value is rebuilt only when the state it
//! was built from changed, so a frame stays O(visible rows) with hundreds of pairs.
use eframe::egui;
use std::sync::Arc;

/// The value cached under `id`, rebuilt by `build` (which also returns the inputs it was built
/// from) unless `current` says the stored inputs still match the state.
pub fn memo<K, V>(
    ui: &egui::Ui,
    id: &'static str,
    current: impl Fn(&K) -> bool,
    build: impl FnOnce() -> (K, V),
) -> Arc<V>
where
    K: Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    let slot = egui::Id::new(("compare_memo", id));
    let stored: Option<Arc<(K, Arc<V>)>> = ui.data(|d| d.get_temp(slot));
    if let Some(hit) = stored.filter(|entry| current(&entry.0)) {
        return hit.1.clone();
    }
    let (inputs, value) = build();
    let value = Arc::new(value);
    let entry = Arc::new((inputs, value.clone()));
    ui.data_mut(|d| d.insert_temp(slot, entry));
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn frame(ctx: &egui::Context, input: u32, builds: &Cell<u32>) -> u32 {
        let mut out = 0;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let value = memo(
                    ui,
                    "test",
                    |stored: &u32| *stored == input,
                    || {
                        builds.set(builds.get() + 1);
                        (input, input * 10)
                    },
                );
                out = *value;
            });
        });
        out
    }

    #[test]
    fn a_value_is_rebuilt_only_when_its_inputs_change() {
        let ctx = egui::Context::default();
        let builds = Cell::new(0);
        assert_eq!(frame(&ctx, 1, &builds), 10);
        assert_eq!(frame(&ctx, 1, &builds), 10);
        assert_eq!(frame(&ctx, 1, &builds), 10);
        assert_eq!(builds.get(), 1, "same inputs: built once");
        assert_eq!(frame(&ctx, 2, &builds), 20);
        assert_eq!(builds.get(), 2, "changed inputs: rebuilt");
        assert_eq!(frame(&ctx, 2, &builds), 20);
        assert_eq!(builds.get(), 2);
    }
}
