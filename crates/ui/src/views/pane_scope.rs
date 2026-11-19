//! The id scope of the pane being drawn (#159, slice 3).
//!
//! A few widgets have a fixed egui id on purpose (a field keeps its focus and its cursor while the
//! controls around it come and go). With two panes on one frame such an id must still be the
//! pane's own, or two panes on the same sub-tab would share it. [`scoped_id`] is that id: the
//! pane's, while a pane of the split view is being drawn, and the plain one otherwise, so an
//! unsplit page keeps exactly the ids it always had. Panes are drawn one after the other, so the
//! scope is one value kept in the context for the length of a pane's draw.
use eframe::egui::{Context, Id};
use std::hash::Hash;

const SCOPE: &str = "curlybrace.split.pane_scope";

/// `pane` is the pane being drawn until [`left`].
pub fn entered(ctx: &Context, pane: Id) {
    ctx.data_mut(|d| d.insert_temp(Id::new(SCOPE), pane));
}

/// The pane's draw is over.
pub fn left(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<Id>(Id::new(SCOPE)));
}

/// A fixed id for `salt`: under the pane being drawn, or the plain one outside a split view.
pub fn scoped_id(ctx: &Context, salt: impl Hash) -> Id {
    match ctx.data(|d| d.get_temp::<Id>(Id::new(SCOPE))) {
        Some(pane) => pane.with(salt),
        None => Id::new(salt),
    }
}

#[cfg(debug_assertions)]
const MODALS: &str = "curlybrace.split.modals_drawn";
#[cfg(debug_assertions)]
const TWICE: &str = "curlybrace.split.modals_twice";

/// The id of a per-tab modal: the pane's own (or the plain one outside a split view), recorded.
pub fn scoped_modal(ctx: &Context, salt: impl Hash) -> Id {
    note_modal(ctx, scoped_id(ctx, salt))
}

/// Returns the modal `id`; in a debug build (and in tests) it also records that the modal is drawn
/// now. A modal drawn twice in one pass is a dialog drawn once per pane (egui does not flag two
/// areas of the same rectangle): see [`modals_drawn_twice`]. A release build records nothing.
pub fn note_modal(ctx: &Context, id: Id) -> Id {
    #[cfg(debug_assertions)]
    record(ctx, id);
    #[cfg(not(debug_assertions))]
    let _ = ctx;
    id
}

#[cfg(debug_assertions)]
fn record(ctx: &Context, id: Id) {
    let pass = ctx.cumulative_pass_nr();
    let (seen_pass, mut seen): (u64, Vec<Id>) = ctx
        .data(|d| d.get_temp(Id::new(MODALS)))
        .unwrap_or((pass, Vec::new()));
    if seen_pass != pass {
        seen.clear();
        // The list is of one pass: a new pass starts it again.
        ctx.data_mut(|d| d.remove::<Vec<Id>>(Id::new(TWICE)));
    }
    if seen.contains(&id) {
        let mut twice: Vec<Id> = ctx.data(|d| d.get_temp(Id::new(TWICE))).unwrap_or_default();
        twice.push(id);
        ctx.data_mut(|d| d.insert_temp(Id::new(TWICE), twice));
    }
    seen.push(id);
    ctx.data_mut(|d| d.insert_temp(Id::new(MODALS), (pass, seen)));
}

/// The modal ids that were drawn twice in the pass so far (a test hook: a correct frame has none;
/// a release build records nothing and returns none).
#[doc(hidden)]
pub fn modals_drawn_twice(ctx: &Context) -> Vec<Id> {
    #[cfg(debug_assertions)]
    {
        ctx.data(|d| d.get_temp(Id::new(TWICE))).unwrap_or_default()
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = ctx;
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_a_pane_the_id_is_the_plain_one() {
        let ctx = Context::default();
        assert_eq!(scoped_id(&ctx, ("a", 1)), Id::new(("a", 1)));
    }

    #[test]
    fn inside_a_pane_the_id_is_the_panes_and_differs_between_panes() {
        let ctx = Context::default();
        entered(&ctx, Id::new("pane one"));
        let one = scoped_id(&ctx, ("a", 1));
        left(&ctx);
        entered(&ctx, Id::new("pane two"));
        let two = scoped_id(&ctx, ("a", 1));
        left(&ctx);
        assert_ne!(one, two);
        assert_ne!(one, Id::new(("a", 1)));
        assert_eq!(
            scoped_id(&ctx, ("a", 1)),
            Id::new(("a", 1)),
            "left means left"
        );
    }
}
