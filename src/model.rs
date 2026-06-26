//! Persisted issue and configuration model.

use std::{
    fmt,
    path::{Component, Path},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use snafu::{ResultExt, ensure};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::error::{
    FormatTimeSnafu, InvalidIssueDirSnafu, InvalidIssueKindSnafu, InvalidRefSnafu, Result,
};

pub(crate) const STORE_VERSION: u32 = 1;
pub(crate) const DEFAULT_ISSUE_TYPE: &str = "task";
pub(crate) const DEFAULT_ISSUE_PRIORITY: u8 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Config {
    pub(crate) version: u32,
    pub(crate) ref_prefix: IssueRef,
    pub(crate) issue_dir: IssueDir,
    #[serde(rename = "default_type")]
    pub(crate) default_issue_type: IssueKind,
    pub(crate) default_priority: u8,
}

impl Config {
    pub(crate) fn new(
        ref_prefix: IssueRef,
        issue_dir: IssueDir,
        default_issue_type: IssueKind,
        default_priority: u8,
    ) -> Self {
        Self {
            version: STORE_VERSION,
            ref_prefix,
            issue_dir,
            default_issue_type,
            default_priority,
        }
    }
}

/// Validated user-visible issue reference, serialised as a plain string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct IssueRef(String);

impl IssueRef {
    pub(crate) fn parse(reference: impl Into<String>) -> Result<Self> {
        let reference = reference.into();
        validate_issue_ref(&reference)?;
        Ok(Self(reference))
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IssueRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for IssueRef {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IssueRef {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

/// Validated issue directory configured relative to the Git root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct IssueDir(String);

impl IssueDir {
    pub(crate) fn parse(path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        validate_issue_dir(&path)?;
        Ok(Self(path))
    }

    #[must_use]
    pub(crate) fn as_path(&self) -> &Path {
        Path::new(self.as_str())
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IssueDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for IssueDir {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IssueDir {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

/// Validated issue kind, serialised in the `type` field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct IssueKind(String);

impl IssueKind {
    pub(crate) fn parse(kind: impl Into<String>) -> Result<Self> {
        let kind = kind.into();
        validate_issue_kind(&kind)?;
        Ok(Self(kind))
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for IssueKind {
    fn default() -> Self {
        Self(DEFAULT_ISSUE_TYPE.to_string())
    }
}

impl fmt::Display for IssueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for IssueKind {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IssueKind {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum IssueStatus {
    /// Work is available unless blocked or claimed.
    Open,
    /// Work has been claimed and is actively being handled.
    InProgress,
    /// Work is resolved and excluded from ready work.
    Closed,
}

impl IssueStatus {
    pub(crate) fn from_name(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "in-progress" => Some(Self::InProgress),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in-progress",
            Self::Closed => "closed",
        }
    }

    pub(crate) fn is_resolved(self) -> bool {
        self == Self::Closed
    }
}

impl fmt::Display for IssueStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Issue {
    pub(crate) id: Uuid,
    #[serde(rename = "ref")]
    pub(crate) reference: IssueRef,
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) body: String,
    pub(crate) status: IssueStatus,
    /// Free-form explanation for the fixed status, such as `completed` or `won't do`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status_reason: Option<String>,
    #[serde(rename = "type")]
    pub(crate) kind: IssueKind,
    pub(crate) priority: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) assignee: Option<String>,
    /// Issue UUIDs that must be closed before this issue is ready.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) blocked_by: Vec<Uuid>,
    /// Issue UUIDs blocked by this issue; mirror of their `blocked_by` entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) blocks: Vec<Uuid>,
    /// Structural parent issue UUID; hierarchy does not affect readiness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent: Option<Uuid>,
    /// Structural child issue UUIDs; mirror of their `parent` entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) children: Vec<Uuid>,
    /// One-way informational links to related issues.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) links: Vec<IssueLink>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) closed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) comments: Vec<Comment>,
}

impl Issue {
    pub(crate) fn new(input: NewIssue) -> Self {
        let NewIssue {
            id,
            reference,
            title,
            body,
            status,
            kind,
            priority,
            labels,
            assignee,
            blocked_by,
            now,
        } = input;

        Self {
            id,
            reference,
            title,
            body,
            status,
            status_reason: None,
            kind,
            priority,
            labels,
            assignee,
            blocked_by,
            blocks: Vec::new(),
            parent: None,
            children: Vec::new(),
            links: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            closed_at: None,
            comments: Vec::new(),
        }
    }

    pub(crate) fn touch(&mut self, now: String) {
        self.updated_at = now;
    }

    pub(crate) fn is_claimed(&self) -> bool {
        self.assignee
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }
}

pub(crate) struct NewIssue {
    pub(crate) id: Uuid,
    pub(crate) reference: IssueRef,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) status: IssueStatus,
    pub(crate) kind: IssueKind,
    pub(crate) priority: u8,
    pub(crate) labels: Vec<String>,
    pub(crate) assignee: Option<String>,
    pub(crate) blocked_by: Vec<Uuid>,
    pub(crate) now: String,
}

/// One-way informational relationship to another issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IssueLink {
    /// Target issue UUID.
    pub(crate) target: Uuid,
    /// Human-readable relationship label, trimmed and non-empty.
    pub(crate) label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Comment {
    pub(crate) id: Uuid,
    pub(crate) author: String,
    pub(crate) body: String,
    pub(crate) created_at: String,
}

impl Comment {
    pub(crate) fn new(author: String, body: String, created_at: String) -> Self {
        Self {
            id: Uuid::now_v7(),
            author,
            body,
            created_at,
        }
    }
}

pub(crate) fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context(FormatTimeSnafu)
}

fn validate_issue_ref(reference: &str) -> Result<()> {
    ensure!(
        !reference.trim().is_empty(),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must not be empty"
        }
    );
    ensure!(
        reference == reference.trim(),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must not have leading or trailing whitespace"
        }
    );
    ensure!(
        reference.chars().all(is_ref_char),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "only ASCII letters, digits, dots, underscores, and dashes are supported"
        }
    );
    ensure!(
        reference != "." && reference != "..",
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must be usable as a file name"
        }
    );
    ensure!(
        reference != "issues-by-id",
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "`issues-by-id` is reserved for canonical issue files"
        }
    );
    ensure!(
        !reference.to_ascii_lowercase().ends_with(".toml"),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must not end in .toml; alias paths add that extension"
        }
    );
    Ok(())
}

fn validate_issue_dir(path: &str) -> Result<()> {
    ensure!(
        !path.trim().is_empty(),
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must not be empty"
        }
    );
    ensure!(
        path == path.trim(),
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must not have leading or trailing whitespace"
        }
    );

    let path_value = Path::new(path);
    ensure!(
        !path_value.is_absolute(),
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must be relative to the Git root"
        }
    );

    let mut has_normal_component = false;
    for component in path_value.components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir => {}
            Component::ParentDir => {
                InvalidIssueDirSnafu {
                    path: path.to_string(),
                    reason: "must not contain parent-directory traversal",
                }
                .fail()?;
            }
            Component::Prefix(_) | Component::RootDir => {
                InvalidIssueDirSnafu {
                    path: path.to_string(),
                    reason: "must be relative to the Git root",
                }
                .fail()?;
            }
        }
    }

    ensure!(
        has_normal_component,
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must name a directory"
        }
    );
    Ok(())
}

fn validate_issue_kind(kind: &str) -> Result<()> {
    ensure!(
        !kind.trim().is_empty(),
        InvalidIssueKindSnafu {
            kind: kind.to_string(),
            reason: "must not be empty"
        }
    );
    ensure!(
        kind == kind.trim(),
        InvalidIssueKindSnafu {
            kind: kind.to_string(),
            reason: "must not have leading or trailing whitespace"
        }
    );
    Ok(())
}

fn is_ref_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '.' || character == '_' || character == '-'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn issue_refs_validate_path_safe_names() {
        assert!(IssueRef::parse("gitrack-a1b2c3d4.1").is_ok());
        assert!(matches!(
            IssueRef::parse("issues-by-id").expect_err("reserved ref"),
            Error::InvalidRef { .. }
        ));
        assert!(matches!(
            IssueRef::parse("project:a1b2c3d4").expect_err("invalid char"),
            Error::InvalidRef { .. }
        ));
    }

    #[test]
    fn issue_dirs_must_be_relative_inside_the_git_root() {
        assert!(matches!(
            IssueDir::parse("/tmp/issues").expect_err("absolute path"),
            Error::InvalidIssueDir { .. }
        ));
        assert!(matches!(
            IssueDir::parse("../issues").expect_err("parent traversal"),
            Error::InvalidIssueDir { .. }
        ));
    }

    #[test]
    fn issue_kinds_reject_empty_or_padded_values() {
        assert!(IssueKind::parse("task").is_ok());
        assert!(matches!(
            IssueKind::parse(" task").expect_err("padded kind"),
            Error::InvalidIssueKind { .. }
        ));
        assert!(matches!(
            IssueKind::parse("").expect_err("empty kind"),
            Error::InvalidIssueKind { .. }
        ));
    }

    #[test]
    fn domain_types_serialize_as_plain_strings() {
        let reference = IssueRef::parse("gitrack-abc").expect("valid ref");
        let kind = IssueKind::parse("task").expect("valid kind");
        let issue_dir = IssueDir::parse("issues").expect("valid issue dir");

        assert_eq!(
            serde_json::to_string(&reference).expect("serialize ref"),
            "\"gitrack-abc\""
        );
        assert_eq!(
            serde_json::to_string(&kind).expect("serialize kind"),
            "\"task\""
        );
        assert_eq!(
            serde_json::to_string(&issue_dir).expect("serialize issue dir"),
            "\"issues\""
        );
    }

    #[test]
    fn domain_types_validate_when_deserializing() {
        let invalid_ref = serde_json::from_str::<IssueRef>("\"bad ref\"");
        let invalid_kind = serde_json::from_str::<IssueKind>("\" task\"");
        let invalid_issue_dir = serde_json::from_str::<IssueDir>("\"../issues\"");

        assert!(invalid_ref.is_err());
        assert!(invalid_kind.is_err());
        assert!(invalid_issue_dir.is_err());
    }
}
