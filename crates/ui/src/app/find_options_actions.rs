//! The options of a find in the reducer. A find the server refused for an option the person set
//! (a hint naming no index, a refused collation, an unsatisfiable read preference, or a timeout
//! under the person's own time limit) is said in the query bar next to the options, with More and
//! the Query options section open and the last page still shown. Any other failure of a find keeps
//! the banner. Option values are user data: nothing here logs one.
use super::reducer::failure;
use super::state::AppState;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::MaxTimeEffect;
use curlybrace_core::query_options::{FindOperation, QueryOptions};
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;

/// Whether `error` is an answer to the options of the find that was sent. The store only answers
/// with the first three when the matching option was sent; a timeout is the person's own only
/// when a limit below the find's bound was sent.
fn caused_by(options: &QueryOptions, error: &CoreError) -> bool {
    match error {
        CoreError::HintNotFound
        | CoreError::CollationRejected
        | CoreError::ReadPreferenceUnavailable => true,
        CoreError::Database(text) if text == COMMAND_TIMEOUT_MESSAGE => {
            options.find_max_time_effect(FindOperation::Find) == MaxTimeEffect::Lowered
        }
        _ => false,
    }
}

/// `Event::Failed` of a find. Never inlined: `data_failed` is on the deepest path of the reducer.
#[inline(never)]
pub(super) fn failed(state: AppState, error: CoreError) -> AppState {
    if !caused_by(&state.issued_query.options, &error) {
        return failure(
            AppState {
                loading_page: false,
                ..state
            },
            "find",
            error,
        );
    }
    let find = crate::viewmodel::FindBarState {
        last_error: Some(error),
        error_from_options: true,
        more: Some(true),
        options_open: Some(true),
        ..state.find
    };
    AppState {
        find,
        loading_page: false,
        ..state
    }
}

#[cfg(test)]
#[path = "find_options_actions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "find_options_collation_tests.rs"]
mod collation_tests;
