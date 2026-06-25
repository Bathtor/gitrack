//! Human and JSON output views for CLI commands.

use std::io::{self, Write};

use serde::Serialize;
use snafu::ResultExt;
use uuid::Uuid;

use crate::{
    error::{Result, SerialiseJsonSnafu, WriteStdoutSnafu},
    model::{Comment, Config, Issue},
    readiness::{issue_is_ready, issue_map},
    store::Store,
};

pub(crate) fn emit_issue(
    config: &Config,
    issues: &[Issue],
    issue: &Issue,
    json: bool,
) -> Result<()> {
    if json {
        print_json(&IssueView::from_issue(config, issues, issue)?, true)
    } else {
        print_issue_summary(issue);
        Ok(())
    }
}

pub(crate) fn print_issue_summary(issue: &Issue) {
    let assignee = issue
        .assignee
        .as_deref()
        .map(|assignee| format!(" @{assignee}"))
        .unwrap_or_default();
    println!(
        "{} [{} p{} {}]{} {}",
        issue.reference, issue.status, issue.priority, issue.kind, assignee, issue.title
    );
}

pub(crate) fn print_issue_detail(config: &Config, issues: &[Issue], issue: &Issue) -> Result<()> {
    println!("{} ({})", issue.reference, issue.id);
    println!("title: {}", issue.title);
    println!("status: {}", issue.status);
    println!("type: {}", issue.kind);
    println!("priority: {}", issue.priority);
    println!(
        "assignee: {}",
        issue.assignee.as_deref().unwrap_or("<unclaimed>")
    );
    if !issue.labels.is_empty() {
        println!("labels: {}", issue.labels.join(", "));
    }
    if !issue.blocked_by.is_empty() {
        let blockers = issue
            .blocked_by
            .iter()
            .map(|id| dependency_label(issues, *id))
            .collect::<Vec<_>>();
        println!("blocked_by: {}", blockers.join(", "));
    }
    println!("created_at: {}", issue.created_at);
    println!("updated_at: {}", issue.updated_at);
    if let Some(closed_at) = &issue.closed_at {
        println!("closed_at: {closed_at}");
    }
    println!(
        "ready: {}",
        issue_is_ready(config, issue, &issue_map(issues))?
    );
    if !issue.body.is_empty() {
        println!();
        println!("{}", issue.body);
    }
    if !issue.comments.is_empty() {
        println!();
        println!("comments:");
        for comment in &issue.comments {
            println!(
                "- {} {} ({}): {}",
                comment.id, comment.author, comment.created_at, comment.body
            );
        }
    }
    Ok(())
}

pub(crate) fn print_json<T>(value: &T, pretty: bool) -> Result<()>
where
    T: Serialize,
{
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if pretty {
        serde_json::to_writer_pretty(&mut handle, value).context(SerialiseJsonSnafu)?;
    } else {
        serde_json::to_writer(&mut handle, value).context(SerialiseJsonSnafu)?;
    }
    writeln!(handle).context(WriteStdoutSnafu)?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub(crate) struct InitView {
    root: String,
    config_dir: String,
    issues_dir: String,
    config_path: String,
    config: Config,
}

impl InitView {
    pub(crate) fn from_store(store: &Store) -> Self {
        Self {
            root: store.root.display().to_string(),
            config_dir: store.dir.display().to_string(),
            issues_dir: store.issues_dir.display().to_string(),
            config_path: store.config_path.display().to_string(),
            config: store.config.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportView {
    version: u32,
    issues: Vec<IssueView>,
}

impl ExportView {
    pub(crate) fn new(config: &Config, issues: &[Issue]) -> Result<Self> {
        let mut issue_views = Vec::with_capacity(issues.len());
        for issue in issues {
            let issue_view = IssueView::from_issue(config, issues, issue)?;
            issue_views.push(issue_view);
        }

        Ok(Self {
            version: config.version,
            issues: issue_views,
        })
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct IssueListView {
    issues: Vec<IssueView>,
}

impl IssueListView {
    pub(crate) fn new(config: &Config, all_issues: &[Issue], issues: Vec<&Issue>) -> Result<Self> {
        let mut issue_views = Vec::with_capacity(issues.len());
        for issue in issues {
            let issue_view = IssueView::from_issue(config, all_issues, issue)?;
            issue_views.push(issue_view);
        }

        Ok(Self {
            issues: issue_views,
        })
    }
}

#[derive(Debug, Serialize)]
struct IssueView {
    id: Uuid,
    #[serde(rename = "ref")]
    reference: String,
    title: String,
    body: String,
    status: String,
    #[serde(rename = "type")]
    kind: String,
    priority: u8,
    labels: Vec<String>,
    assignee: Option<String>,
    blocked_by: Vec<DependencyView>,
    ready: bool,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    comments: Vec<Comment>,
}

impl IssueView {
    fn from_issue(config: &Config, issues: &[Issue], issue: &Issue) -> Result<Self> {
        let by_id = issue_map(issues);
        Ok(Self {
            id: issue.id,
            reference: issue.reference.clone(),
            title: issue.title.clone(),
            body: issue.body.clone(),
            status: issue.status.clone(),
            kind: issue.kind.clone(),
            priority: issue.priority,
            labels: issue.labels.clone(),
            assignee: issue.assignee.clone(),
            blocked_by: issue
                .blocked_by
                .iter()
                .map(|id| DependencyView::from_id(issues, *id))
                .collect(),
            ready: issue_is_ready(config, issue, &by_id)?,
            created_at: issue.created_at.clone(),
            updated_at: issue.updated_at.clone(),
            closed_at: issue.closed_at.clone(),
            comments: issue.comments.clone(),
        })
    }
}

#[derive(Debug, Serialize)]
struct DependencyView {
    id: Uuid,
    #[serde(rename = "ref")]
    reference: Option<String>,
}

impl DependencyView {
    fn from_id(issues: &[Issue], id: Uuid) -> Self {
        Self {
            id,
            reference: issues
                .iter()
                .find(|issue| issue.id == id)
                .map(|issue| issue.reference.clone()),
        }
    }
}

fn dependency_label(issues: &[Issue], id: Uuid) -> String {
    issues.iter().find(|issue| issue.id == id).map_or_else(
        || id.to_string(),
        |issue| format!("{} ({})", issue.reference, issue.id),
    )
}
