//! The administration dialogs of the sidebar: drop a collection or database (by typing its
//! name), rename or duplicate a collection, create a view. Pure and egui-free; every updater
//! returns a new value. The name and pipeline rules are the core's own validators, so the dialog
//! says what the core would refuse before anything is sent; the core still checks again.
use super::typed_confirmation::typed_text_matches;
use curlybrace_core::admin::{parse_pipeline, validate_collection_name};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, SessionId};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// What the person wants to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminKind {
    DropCollection,
    DropDatabase,
    Rename,
    Duplicate,
    CreateView,
}

impl AdminKind {
    /// The destructive kinds, which ask for the exact name before they go.
    pub fn is_drop(self) -> bool {
        matches!(self, Self::DropCollection | Self::DropDatabase)
    }
}

/// A menu entry asking to open a dialog. `collection` is the row the menu belongs to; a view
/// created from a database row has none (the source is typed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminRequest {
    pub kind: AdminKind,
    pub db: String,
    pub collection: Option<String>,
}

/// Why a dialog cannot be sent yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminProblem {
    /// A name is missing (the new name, or the view's source).
    Required,
    /// The core refuses the name (`$`, NUL, `system.`).
    InvalidName,
    /// A rename to the name it already has.
    SameName,
    /// The view pipeline is not a JSON array of stage documents.
    Pipeline,
    /// The exact name of a drop was not typed yet.
    ConfirmMismatch,
}

impl AdminProblem {
    /// The i18n key of the line that says what is wrong.
    pub fn key(self) -> &'static str {
        match self {
            Self::Required => "admin.problem.required",
            Self::InvalidName => "admin.problem.invalid_name",
            Self::SameName => "admin.problem.same_name",
            Self::Pipeline => "admin.problem.pipeline",
            Self::ConfirmMismatch => "admin.problem.confirm_mismatch",
        }
    }
}

/// An open dialog and everything typed into it.
#[derive(Clone, Debug, PartialEq)]
pub struct AdminDialog {
    pub kind: AdminKind,
    pub db: String,
    /// The collection the dialog is about (the source of a rename, duplicate or drop).
    pub collection: Option<String>,
    /// The text typed to confirm a drop.
    pub typed: String,
    /// The new name: of the renamed or duplicated collection, or of the view.
    pub name: String,
    /// The collection a view reads from.
    pub source: String,
    /// The view pipeline, as JSON text.
    pub pipeline: String,
    /// A rename may replace an existing collection of the new name.
    pub drop_target: bool,
    /// What the server or core refused on the last send; shown until the next edit.
    pub error: Option<CoreError>,
    /// The session the dialog was opened on; its write goes only there.
    pub session: Uuid,
}

impl AdminDialog {
    pub fn open(request: AdminRequest) -> Self {
        let AdminRequest {
            kind,
            db,
            collection,
        } = request;
        let name = match (kind, collection.as_deref()) {
            (AdminKind::Rename, Some(c)) => c.to_owned(),
            (AdminKind::Duplicate, Some(c)) => format!("{c}_copy"),
            _ => String::new(),
        };
        let source = match kind {
            AdminKind::CreateView => collection.clone().unwrap_or_default(),
            _ => String::new(),
        };
        Self {
            kind,
            db,
            collection,
            typed: String::new(),
            name,
            source,
            pipeline: "[]".into(),
            drop_target: false,
            error: None,
            session: Uuid::nil(),
        }
    }

    pub fn with_session(self, session: Uuid) -> Self {
        Self { session, ..self }
    }

    /// The text a drop needs typed; none for the other kinds.
    pub fn required_text(&self) -> Option<&str> {
        match self.kind {
            AdminKind::DropCollection => self.collection.as_deref(),
            AdminKind::DropDatabase => Some(&self.db),
            _ => None,
        }
    }

    fn edited(self, change: impl FnOnce(&mut Self)) -> Self {
        let mut next = Self {
            error: None,
            ..self
        };
        change(&mut next);
        next
    }

    pub fn with_typed(self, typed: String) -> Self {
        self.edited(|d| d.typed = typed)
    }

    pub fn with_name(self, name: String) -> Self {
        self.edited(|d| d.name = name)
    }

    pub fn with_source(self, source: String) -> Self {
        self.edited(|d| d.source = source)
    }

    pub fn with_pipeline(self, pipeline: String) -> Self {
        self.edited(|d| d.pipeline = pipeline)
    }

    pub fn with_drop_target(self, drop_target: bool) -> Self {
        self.edited(|d| d.drop_target = drop_target)
    }

    pub fn with_error(self, error: Option<CoreError>) -> Self {
        Self { error, ..self }
    }

    /// The first reason the dialog cannot be sent, if any.
    pub fn problem(&self) -> Option<AdminProblem> {
        match self.kind {
            AdminKind::DropCollection | AdminKind::DropDatabase => {
                let required = self.required_text().unwrap_or_default();
                (!typed_text_matches(required, &self.typed))
                    .then_some(AdminProblem::ConfirmMismatch)
            }
            AdminKind::Rename => self.new_name_problem().or_else(|| {
                (self.collection.as_deref() == Some(self.name.as_str()))
                    .then_some(AdminProblem::SameName)
            }),
            AdminKind::Duplicate => self.new_name_problem(),
            AdminKind::CreateView => self
                .new_name_problem()
                .or_else(|| {
                    self.source
                        .trim()
                        .is_empty()
                        .then_some(AdminProblem::Required)
                })
                .or_else(|| {
                    parse_pipeline(&self.pipeline)
                        .err()
                        .map(|_| AdminProblem::Pipeline)
                }),
        }
    }

    fn new_name_problem(&self) -> Option<AdminProblem> {
        if self.name.trim().is_empty() {
            return Some(AdminProblem::Required);
        }
        validate_collection_name(&self.name)
            .err()
            .map(|_| AdminProblem::InvalidName)
    }

    pub fn ready(&self) -> bool {
        self.problem().is_none()
    }

    /// The namespace the write acts on, which the in-flight marker and the answer are matched
    /// by. A database has no collection part.
    pub fn target_ns(&self) -> Namespace {
        let collection = match self.kind {
            AdminKind::CreateView => self.name.clone(),
            AdminKind::DropDatabase => String::new(),
            _ => self.collection.clone().unwrap_or_default(),
        };
        Namespace::new(&self.db, collection)
    }

    /// The command this dialog sends, once it is ready.
    pub fn command(&self, session: SessionId) -> Option<Command> {
        if !self.ready() {
            return None;
        }
        let ns = self.target_ns();
        Some(match self.kind {
            AdminKind::DropCollection => Command::DropCollection { session, ns },
            AdminKind::DropDatabase => Command::DropDatabase {
                session,
                db: self.db.clone(),
            },
            AdminKind::Rename => Command::RenameCollection {
                session,
                ns,
                new_name: self.name.clone(),
                drop_target: self.drop_target,
            },
            AdminKind::Duplicate => Command::DuplicateCollection {
                session,
                ns,
                target: self.name.clone(),
            },
            AdminKind::CreateView => Command::CreateView {
                session,
                ns,
                source: self.source.trim().to_owned(),
                pipeline_json: self.pipeline.clone(),
            },
        })
    }
}

/// What the person did in the open dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum AdminAction {
    Typed(String),
    NameEdited(String),
    SourceEdited(String),
    PipelineEdited(String),
    DropTargetToggled(bool),
    Confirmed,
    Cancelled,
}

#[cfg(test)]
#[path = "admin_dialog_tests.rs"]
mod tests;
