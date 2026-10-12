//! Finding rows in the virtualised sidebar. Rows outside the viewport are not in the accessibility
//! tree, and a developer's server may list any number of databases, so helpers scroll with the
//! mouse wheel until the wanted row exists (bounded, with a clear failure).
use super::screen::{H, dump};
use super::wait::settle;
use eframe::egui::{Event, Modifiers, MouseWheelUnit, pos2, vec2};
use egui_kittest::kittest::Queryable;

/// A point inside the sidebar panel (the left side panel starts at x = 0).
const OVER_SIDEBAR: (f32, f32) = (100.0, 300.0);
/// Scroll step in points; smaller than the viewport so no row is skipped.
const STEP: f32 = 300.0;
/// Upper bound on scroll steps (STEP * MAX_STEPS points, i.e. thousands of rows).
const MAX_STEPS: usize = 400;

/// Scrolls the sidebar down until a label equal to `text` exists; returns after at most
/// [`MAX_STEPS`] steps and PANICS with the visible labels if it never appears.
#[track_caller]
pub fn scroll_sidebar_until_shown(harness: &mut H<'_>, text: &str) {
    for _ in 0..MAX_STEPS {
        if harness.query_all_by_label(text).next().is_some() {
            return;
        }
        let input = harness.input_mut();
        input
            .events
            .push(Event::PointerMoved(pos2(OVER_SIDEBAR.0, OVER_SIDEBAR.1)));
        input.events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -STEP),
            modifiers: Modifiers::NONE,
        });
        settle(harness);
    }
    if harness.query_all_by_label(text).next().is_some() {
        return;
    }
    panic!(
        "{text:?} never appeared in the sidebar after {MAX_STEPS} scroll steps:\n{}",
        dump(harness)
    );
}

/// Frames to wait for the (smooth) scrolling to come to rest before clicking.
const SETTLE_STEPS: usize = 60;
/// A row whose centre is inside this vertical band is fully inside the viewport and safe to click
/// (rows at the very edge exist in the tree but may be clipped, which swallows the click).
const SAFE_BAND: std::ops::RangeInclusive<f32> = 120.0..=560.0;
/// Rounds of "scroll the row towards the middle".
const CENTER_ROUNDS: usize = 10;

fn rest(harness: &mut H<'_>, text: &str) -> eframe::egui::Rect {
    let mut last = harness.get_by_label(text).rect();
    for _ in 0..SETTLE_STEPS {
        harness.step();
        let now = harness.get_by_label(text).rect();
        if now == last {
            break;
        }
        last = now;
    }
    last
}

fn wheel(harness: &mut H<'_>, dy: f32) {
    let input = harness.input_mut();
    input
        .events
        .push(Event::PointerMoved(pos2(OVER_SIDEBAR.0, OVER_SIDEBAR.1)));
    input.events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        modifiers: Modifiers::NONE,
    });
    settle(harness);
}

/// [`scroll_sidebar_until_shown`], then moves the row into the safe band, waits until it stops
/// moving (egui scrolls smoothly, so a click at the last reported position could land on a
/// neighbour) and clicks it.
#[track_caller]
pub fn click_sidebar_row(harness: &mut H<'_>, text: &str) {
    scroll_sidebar_until_shown(harness, text);
    let mut rect = rest(harness, text);
    for _ in 0..CENTER_ROUNDS {
        if SAFE_BAND.contains(&rect.center().y) {
            break;
        }
        // Content moves with the wheel delta: pull the row towards y = 300.
        wheel(harness, 300.0 - rect.center().y);
        let moved = rest(harness, text);
        if moved == rect {
            break; // cannot scroll further (end of the list)
        }
        rect = moved;
    }
    harness.get_by_label(text).click();
    settle(harness);
}
