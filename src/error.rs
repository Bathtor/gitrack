//! Error types for the Git-native issue tracker.

use std::path::PathBuf;

use snafu::Snafu;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("not inside a Git working tree from {}", start.display()))]
    NotGitRepository { start: PathBuf },

    #[snafu(display("issue store is already initialised at {}", path.display()))]
    AlreadyInitialised { path: PathBuf },

    #[snafu(display("issue store is not initialised; run `gitrack init` first in {}", root.display()))]
    MissingStore { root: PathBuf },

    #[snafu(display("failed to create directory {}: {source}", path.display()))]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to determine current directory: {source}"))]
    CurrentDir { source: std::io::Error },

    #[snafu(display("failed to canonicalise {}: {source}", path.display()))]
    CanonicalisePath {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to read directory {}: {source}", path.display()))]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to read {}: {source}", path.display()))]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to write {}: {source}", path.display()))]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to parse TOML from {}: {source}", path.display()))]
    ParseToml {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[snafu(display("failed to serialise TOML: {source}"))]
    SerialiseToml { source: toml::ser::Error },

    #[snafu(display("failed to serialise JSON: {source}"))]
    SerialiseJson { source: serde_json::Error },

    #[snafu(display("failed to write to stdout: {source}"))]
    WriteStdout { source: std::io::Error },

    #[snafu(display("failed to format timestamp: {source}"))]
    FormatTime { source: time::error::Format },

    #[snafu(display("invalid ref `{reference}`: {reason}"))]
    InvalidRef {
        reference: String,
        reason: &'static str,
    },

    #[snafu(display("invalid issue directory `{path}`: {reason}"))]
    InvalidIssueDir { path: String, reason: &'static str },

    #[snafu(display("invalid issue file name {}: {reason}", path.display()))]
    InvalidIssueFileName { path: PathBuf, reason: &'static str },

    #[snafu(display(
        "issue file {} is named for UUID {file_id}, but contains issue UUID {issue_id}",
        path.display()
    ))]
    IssueFileNameMismatch {
        path: PathBuf,
        file_id: uuid::Uuid,
        issue_id: uuid::Uuid,
    },

    #[snafu(display(
        "duplicate issue UUID {id} in {} and {}",
        first_path.display(),
        duplicate_path.display()
    ))]
    DuplicateIssueId {
        id: uuid::Uuid,
        first_path: PathBuf,
        duplicate_path: PathBuf,
    },

    #[snafu(display(
        "duplicate issue ref `{reference}` for issue UUIDs {first_id} and {duplicate_id}"
    ))]
    DuplicateIssueRef {
        reference: String,
        first_id: uuid::Uuid,
        duplicate_id: uuid::Uuid,
    },

    #[snafu(display("ref `{reference}` is already used by another issue"))]
    RefExists { reference: String },

    #[snafu(display("issue `{reference}` was not found"))]
    IssueNotFound { reference: String },

    #[snafu(display("issue ref `{reference}` is ambiguous; matching UUIDs: {matches}"))]
    AmbiguousIssue { reference: String, matches: String },

    #[snafu(display("issue `{issue}` cannot depend on itself"))]
    SelfDependency { issue: String },

    #[snafu(display("issue `{issue}` depends on missing blocker UUID {blocker}"))]
    MissingDependency { issue: String, blocker: uuid::Uuid },
}
