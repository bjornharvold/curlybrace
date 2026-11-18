//! The connection edit dialog in the smallest supported window (800x500): with a long test
//! result, a validation error and a banner, the Cancel / Save footer stays on screen and the body
//! shrinks instead (`CHROME_HEIGHT` of the dialog). Measured from accesskit bounds. Fast, offline,
//! no GPU.
use curlybrace_core::error::CoreError;
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::views::connections::{ConnectionsState, TestState, show_connections};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const SMALL: egui::Vec2 = egui::vec2(800.0, 500.0);
/// The dialog's footer keeps this much room under it (points): the modal's frame.
const BOTTOM_ROOM: f32 = 4.0;

fn harness(prepare: impl FnOnce(&mut ConnectionsState)) -> Harness<'static, ConnectionsState> {
    let mut state = ConnectionsState {
        editing: true,
        ..ConnectionsState::default()
    };
    prepare(&mut state);
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(SMALL)
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, state: &mut ConnectionsState| {
                if !installed {
                    theme::install(ui.ctx());
                    ui.ctx().set_theme(egui::Theme::Light);
                    installed = true;
                }
                show_connections(ui, state, &I18n::english());
            },
            state,
        );
    harness.run_steps(6);
    harness
}

fn long_error() -> CoreError {
    CoreError::InvalidJson("x".repeat(400))
}

fn footer_bottoms(harness: &Harness<'static, ConnectionsState>) -> (f32, f32) {
    let i18n = I18n::english();
    let cancel = harness
        .get_by_role_and_label(Role::Button, &i18n.t("common.cancel"))
        .rect();
    let save = harness
        .get_by_role_and_label(Role::Button, &i18n.t("common.save"))
        .rect();
    (cancel.bottom(), save.bottom())
}

#[test]
fn the_footer_is_on_screen_with_nothing_to_report() {
    let harness = harness(|_| {});
    let (cancel, save) = footer_bottoms(&harness);
    assert!(
        cancel <= SMALL.y - BOTTOM_ROOM && save <= SMALL.y - BOTTOM_ROOM,
        "Cancel ends at {cancel}, Save at {save} (window {})",
        SMALL.y
    );
}

#[test]
fn the_footer_stays_on_screen_with_a_long_test_error_a_validation_error_and_a_banner() {
    let harness = harness(|state| {
        state.test = TestState::Done {
            request_id: 1,
            outcome: Err(long_error()),
        };
        state.error = Some(long_error());
        state.dialog_banner = vec![
            "The connections file could not be saved".into(),
            "y".repeat(160),
        ];
    });
    let (cancel, save) = footer_bottoms(&harness);
    assert!(
        cancel <= SMALL.y - BOTTOM_ROOM && save <= SMALL.y - BOTTOM_ROOM,
        "Cancel ends at {cancel}, Save at {save}: the footer left the {} pt window",
        SMALL.y
    );
}
