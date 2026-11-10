//! The MongoDB command-line tools the app can run for the user: which executable to use, what
//! version it is and whether it suits the server. The app never bundles these tools.
mod probe;
mod resolve;
mod settings;
mod suggest;
mod version;
pub(crate) use probe::{absolute_entries, kill_group};
pub use probe::{probe_version, probe_version_with_path};
pub use resolve::{ToolChoice, ToolError, resolve};
pub use settings::ToolSettings;
pub use suggest::{COMMON_DIRS, default_search_path, search_path, suggestions, suggestions_in};
pub use version::{Compatibility, ServerVersion, ToolKind, ToolVersion, compatibility};
