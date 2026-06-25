//! Persisted issue and configuration model.

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::error::{FormatTimeSnafu, Result};
use snafu::ResultExt;

pub(crate) const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Config {
    pub(crate) version: u32,
    pub(crate) ref_prefix: String,
    pub(crate) issue_dir: String,
    pub(crate) default_status: String,
    #[serde(rename = "default_type")]
    pub(crate) default_issue_type: String,
    pub(crate) default_priority: u8,
    pub(crate) closed_status: String,
    pub(crate) resolved_statuses: Vec<String>,
}

impl Config {
    pub(crate) fn new(ref_prefix: String, issue_dir: String) -> Self {
        Self {
            version: STORE_VERSION,
            ref_prefix,
            issue_dir,
            default_status: "open".to_string(),
            default_issue_type: "task".to_string(),
            default_priority: 3,
            closed_status: "closed".to_string(),
            resolved_statuses: vec!["closed".to_string(), "resolved".to_string()],
        }
    }

    pub(crate) fn status_is_resolved(&self, status: &str) -> bool {
        self.resolved_statuses
            .iter()
            .any(|resolved| resolved == status)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Issue {
    pub(crate) id: Uuid,
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) body: String,
    pub(crate) status: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) priority: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) blocked_by: Vec<Uuid>,
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
            kind,
            priority,
            labels,
            assignee,
            blocked_by,
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
    pub(crate) reference: String,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) status: String,
    pub(crate) kind: String,
    pub(crate) priority: u8,
    pub(crate) labels: Vec<String>,
    pub(crate) assignee: Option<String>,
    pub(crate) blocked_by: Vec<Uuid>,
    pub(crate) now: String,
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
