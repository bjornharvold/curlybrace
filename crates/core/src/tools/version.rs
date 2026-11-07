//! Tool kinds, version parsing and the "too old for this server" rule.
use serde::{Deserialize, Serialize};

/// The command-line tools the app can run. `mongoexport` and `mongoimport` join later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Mongosh,
    Mongodump,
    Mongorestore,
}

impl ToolKind {
    pub const ALL: [Self; 3] = [Self::Mongosh, Self::Mongodump, Self::Mongorestore];

    /// The executable name looked up on `PATH`. Not translated: it is a file name.
    pub const fn binary_name(self) -> &'static str {
        match self {
            Self::Mongosh => "mongosh",
            Self::Mongodump => "mongodump",
            Self::Mongorestore => "mongorestore",
        }
    }
}

/// A tool's `major.minor.patch`, ordered like a version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ToolVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// The first `x.y.z` found in the output of `--version`, line by line. `mongosh` prints the
    /// bare number, the database tools print `mongodump version: 100.10.0` and more lines.
    pub fn parse(output: &str) -> Option<Self> {
        output
            .lines()
            .flat_map(str::split_whitespace)
            .find_map(Self::parse_token)
    }

    fn parse_token(token: &str) -> Option<Self> {
        let mut parts = token.splitn(3, '.');
        let major = number(parts.next()?)?;
        let minor = number(parts.next()?)?;
        let patch = leading_digits(parts.next()?)?;
        Some(Self::new(major, minor, patch))
    }
}

fn number(part: &str) -> Option<u32> {
    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        .then(|| part.parse().ok())
        .flatten()
}

/// The digits at the start of `part` (`1-rc1` gives 1); `None` when there are none.
fn leading_digits(part: &str) -> Option<u32> {
    let end = part
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(part.len());
    number(&part[..end])
}

/// A server's `major.minor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerVersion {
    pub major: u32,
    pub minor: u32,
}

impl ServerVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// The `major.minor` of a version string such as `8.0.3` or `7.0.14-rc0`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().splitn(3, '.');
        let major = number(parts.next()?)?;
        let minor = leading_digits(parts.next()?)?;
        Some(Self::new(major, minor))
    }
}

/// Whether a tool suits a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    Ok,
    TooOld,
    /// No server to compare with (not connected): never a warning.
    Unknown,
}

/// For each server release: the oldest `mongosh` and database tools that support it.
/// Newest first. Release notes of the tools are the source; correct a row here when they change.
const MINIMUMS: [(ServerVersion, ToolVersion, ToolVersion); 3] = [
    (
        ServerVersion::new(8, 0),
        ToolVersion::new(2, 3, 0),
        ToolVersion::new(100, 10, 0),
    ),
    (
        ServerVersion::new(7, 0),
        ToolVersion::new(2, 0, 0),
        ToolVersion::new(100, 8, 0),
    ),
    (
        ServerVersion::new(6, 0),
        ToolVersion::new(1, 6, 0),
        ToolVersion::new(100, 5, 0),
    ),
];

/// `TooOld` when `tool` is older than the minimum for the newest listed release that is not
/// newer than `server`. A server older than every row has no minimum.
pub fn compatibility(
    kind: ToolKind,
    tool: ToolVersion,
    server: Option<ServerVersion>,
) -> Compatibility {
    let Some(server) = server else {
        return Compatibility::Unknown;
    };
    let Some((_, shell, tools)) = MINIMUMS.iter().find(|(release, ..)| *release <= server) else {
        return Compatibility::Ok;
    };
    let minimum = match kind {
        ToolKind::Mongosh => shell,
        ToolKind::Mongodump | ToolKind::Mongorestore => tools,
    };
    if tool < *minimum {
        Compatibility::TooOld
    } else {
        Compatibility::Ok
    }
}

#[cfg(test)]
#[path = "version_tests.rs"]
mod tests;
