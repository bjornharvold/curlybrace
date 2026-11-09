//! The text a person has to type to confirm (a drop, an overwriting or production copy) is laid
//! out in full as a wrapping label above the box, however long it is; the box's floating label
//! is short and its accessible name still carries the whole prompt.
use bson::Document;
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::protocol::{CopyItem, CopyRequest, CopyScope};
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::app::copy_state::CopyDialog;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme::{self};
use curlybrace_ui::viewmodel::{ConnectionBadge, DropConfirm, IndexesAction};
use curlybrace_ui::views::copy_dialog::show_copy_modal;
use curlybrace_ui::views::index_dialogs::{DialogContext, show_drop_confirm};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use uuid::Uuid;

/// 60 characters, no break opportunity: the worst case for a label that elides at the end.
const LONG: &str = "customer_orders_by_created_at_and_status_and_billing_country";

fn themed(ctx: &egui::Context, installed: &mut bool) {
    if !*installed {
        theme::install(ctx);
        ctx.set_theme(egui::Theme::Light);
        *installed = true;
    }
}

/// The text is a label inside `[left, right]` (the modal), wrapped rather than cut.
fn assert_laid_out_in_full<S>(
    h: &Harness<'_, S>,
    required: &str,
    prompt: &str,
    (left, right): (f32, f32),
) {
    let text = h.get_by_role_and_label(Role::Label, required);
    let rect = text.rect();
    assert!(rect.left() >= left && rect.right() <= right, "{rect:?}");
    // Not one letter per line, and not cut: the whole text is in the node.
    assert!(rect.height() < 80.0, "{rect:?}");
    h.get_by_role_and_label(Role::TextInput, prompt);
}

#[test]
fn a_long_index_name_is_shown_in_full_above_the_drop_box() {
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(
            move |ctx, confirm: &mut DropConfirm| {
                themed(ctx, &mut installed);
                let badge = ConnectionBadge::default();
                let context = DialogContext {
                    connection: "local",
                    badge: &badge,
                    collection: "shop.orders",
                    busy: false,
                };
                if let Some(IndexesAction::DropTyped(text)) =
                    show_drop_confirm(ctx, confirm, &context, &I18n::english())
                {
                    *confirm = confirm.clone().with_typed(text);
                }
            },
            DropConfirm::new(LONG.into()),
        );
    h.run_steps(8);
    let prompt = I18n::english().t_with("indexes.drop.type_name", &[("name", LONG)]);
    assert_laid_out_in_full(&h, LONG, &prompt, (210.0, 690.0));
}

#[test]
fn a_long_target_name_is_shown_in_full_above_the_copy_box() {
    let item = CopyItem {
        source: Namespace::new("shop", LONG),
        target: Namespace::new("crm", LONG),
        filter: Document::new(),
    };
    let request = CopyRequest {
        source: Uuid::from_u128(1),
        target: Uuid::from_u128(2),
        scope: CopyScope::Collections(vec![item]),
        policy: ConflictPolicy::Overwrite,
        with_indexes_and_options: false,
    };
    let app = AppState {
        copy: Some(CopyDialog::opened(request).with_policy(ConflictPolicy::Overwrite)),
        ..AppState::default()
    };
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 1100.0))
        .build_state(
            move |ctx, app: &mut AppState| {
                themed(ctx, &mut installed);
                show_copy_modal(ctx, app, &I18n::english());
            },
            app,
        );
    h.run_steps(8);
    let prompt = I18n::english().t_with("copy.dialog.confirm_prompt", &[("text", LONG)]);
    // The name also sits in the name box as its value; the label is a different node.
    assert_laid_out_in_full(&h, LONG, &prompt, (140.0, 760.0));
}
