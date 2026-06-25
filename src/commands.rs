//! CLI argument parsing and command execution.

use std::{env, path::Path};

use clap::{Parser, Subcommand};
use snafu::ensure;
use uuid::Uuid;

use crate::{
    error::{Result, SelfDependencySnafu},
    model::{Comment, Issue, NewIssue, now_rfc3339},
    readiness::{issue_is_ready, issue_map},
    store::{DEFAULT_ISSUES_DIR, Store, normalise_labels, normalise_optional},
    views::{
        ExportView,
        InitView,
        IssueListView,
        emit_issue,
        print_issue_detail,
        print_issue_summary,
        print_json,
    },
};

#[derive(Debug, Parser)]
#[command(version, about = "A small Git-native issue tracker")]
pub struct Cli {
    #[arg(long, global = true, help = "Emit deterministic JSON where supported")]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Init(InitArgs),
    Create(CreateArgs),
    List(ListArgs),
    Ready(ReadyArgs),
    Show(ShowArgs),
    Update(UpdateArgs),
    Ref(RefArgs),
    Claim(ClaimArgs),
    Block(BlockArgs),
    Link(LinkArgs),
    Close(CloseArgs),
    Reopen(ReopenArgs),
    #[command(alias = "note")]
    Comment(CommentArgs),
    Export(ExportArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct InitArgs {
    #[arg(
        long,
        help = "Override the ref prefix derived from the Git repository name"
    )]
    prefix: Option<String>,

    #[arg(
        long,
        default_value = DEFAULT_ISSUES_DIR,
        help = "Issue directory relative to the Git root"
    )]
    issue_dir: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct CreateArgs {
    title: String,

    #[arg(long, default_value = "", help = "Issue body or description")]
    body: String,

    #[arg(long = "type", help = "Issue type; defaults to config.default_type")]
    issue_type: Option<String>,

    #[arg(long, help = "Issue priority; lower numbers sort first")]
    priority: Option<u8>,

    #[arg(
        long = "label",
        help = "Label to add; may be repeated or comma-separated"
    )]
    labels: Vec<String>,

    #[arg(long, help = "Assignee to set at creation time")]
    assignee: Option<String>,

    #[arg(long = "ref", help = "Explicit user-visible ref, for example parent.1")]
    reference: Option<String>,

    #[arg(long = "blocked-by", help = "Issue ref or UUID that blocks this issue")]
    blocked_by: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ListArgs {
    #[arg(long, help = "Include resolved issues")]
    all: bool,

    #[arg(long, help = "Filter by exact status")]
    status: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ReadyArgs {}

#[derive(Debug, clap::Args)]
pub(crate) struct ShowArgs {
    issue: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct UpdateArgs {
    issue: String,

    #[arg(long)]
    title: Option<String>,

    #[arg(long)]
    body: Option<String>,

    #[arg(long)]
    status: Option<String>,

    #[arg(long = "type")]
    issue_type: Option<String>,

    #[arg(long)]
    priority: Option<u8>,

    #[arg(
        long = "label",
        help = "Replace labels; may be repeated or comma-separated"
    )]
    labels: Vec<String>,

    #[arg(long, help = "Clear all labels before applying any --label values")]
    clear_labels: bool,

    #[arg(long)]
    assignee: Option<String>,

    #[arg(long)]
    clear_assignee: bool,

    #[arg(long = "ref", help = "Rename the user-visible ref")]
    reference: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RefArgs {
    issue: String,

    #[arg(help = "Explicit new ref; omit to generate a fresh automatic ref")]
    reference: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ClaimArgs {
    issue: String,

    #[arg(long, help = "Assignee; defaults to GITRACK_ACTOR or USER")]
    assignee: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct BlockArgs {
    issue: String,

    #[arg(
        long = "by",
        required = true,
        help = "Issue ref or UUID that blocks this issue"
    )]
    blockers: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct LinkArgs {
    issue: String,
    blocker: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct CloseArgs {
    issue: String,

    #[arg(long, help = "Optional close reason recorded as a comment")]
    reason: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ReopenArgs {
    issue: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct CommentArgs {
    issue: String,
    body: String,

    #[arg(long, help = "Comment author; defaults to GITRACK_ACTOR or USER")]
    author: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ExportArgs {
    #[command(subcommand)]
    format: ExportFormat,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ExportFormat {
    Json(JsonExportArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct JsonExportArgs {
    #[arg(long, help = "Pretty-print JSON output")]
    pretty: bool,
}

/// Execute one parsed CLI command.
///
/// # Errors
///
/// Returns an error when the issue store cannot be read or written, command
/// arguments reference unknown or ambiguous issues, or output serialisation
/// fails.
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init(args) => init(args, cli.json),
        Command::Create(args) => create(args, cli.json),
        Command::List(args) => list(&args, cli.json),
        Command::Ready(args) => ready(args, cli.json),
        Command::Show(args) => show(&args, cli.json),
        Command::Update(args) => update(args, cli.json),
        Command::Ref(args) => rename_ref(args, cli.json),
        Command::Claim(args) => claim(args, cli.json),
        Command::Block(args) => block(args, cli.json),
        Command::Link(args) => link(args, cli.json),
        Command::Close(args) => close(args, cli.json),
        Command::Reopen(args) => reopen(&args, cli.json),
        Command::Comment(args) => comment(args, cli.json),
        Command::Export(args) => export(args),
    }
}

fn init(args: InitArgs, json: bool) -> Result<()> {
    let store = Store::init(Path::new("."), args.prefix, args.issue_dir)?;
    if json {
        print_json(&InitView::from_store(&store), true)?;
    } else {
        println!(
            "Initialised issue store at {} with config {} and ref prefix `{}`",
            store.issues_dir.display(),
            store.config_path.display(),
            store.config.ref_prefix
        );
    }
    Ok(())
}

fn create(args: CreateArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let issues = store.load_issues()?;
    let id = Uuid::now_v7();
    let reference = match args.reference {
        Some(reference) => {
            Store::ensure_ref_available(&issues, &reference, None)?;
            reference
        }
        None => store.generated_ref(&issues)?,
    };
    let blocked_by = resolve_many(&issues, args.blocked_by)?;
    let now = now_rfc3339()?;
    let issue_type = args
        .issue_type
        .unwrap_or_else(|| store.config.default_issue_type.clone());
    let priority = args.priority.unwrap_or(store.config.default_priority);
    let issue = Issue::new(NewIssue {
        id,
        reference,
        title: args.title,
        body: args.body,
        status: store.config.default_status.clone(),
        kind: issue_type,
        priority,
        labels: normalise_labels(args.labels),
        assignee: normalise_optional(args.assignee),
        blocked_by,
        now,
    });

    store.save_issue(&issue)?;
    let mut all_issues = issues;
    all_issues.push(issue.clone());
    emit_issue(&store.config, &all_issues, &issue, json)
}

fn list(args: &ListArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let issues = store.load_issues()?;
    let filtered = issues
        .iter()
        .filter(|issue| {
            if let Some(status) = &args.status {
                &issue.status == status
            } else {
                args.all || !store.config.status_is_resolved(&issue.status)
            }
        })
        .collect::<Vec<_>>();

    if json {
        let view = IssueListView::new(&store.config, &issues, filtered)?;
        print_json(&view, true)
    } else {
        for issue in filtered {
            print_issue_summary(issue);
        }
        Ok(())
    }
}

fn ready(_args: ReadyArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let issues = store.load_issues()?;
    let by_id = issue_map(&issues);
    let mut ready = Vec::new();
    for issue in &issues {
        if issue_is_ready(&store.config, issue, &by_id)? {
            ready.push(issue);
        }
    }

    if json {
        let view = IssueListView::new(&store.config, &issues, ready)?;
        print_json(&view, true)
    } else {
        for issue in ready {
            print_issue_summary(issue);
        }
        Ok(())
    }
}

fn show(args: &ShowArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let issues = store.load_issues()?;
    let issue = Store::resolve_issue(&issues, &args.issue)?;
    if json {
        emit_issue(&store.config, &issues, issue, true)
    } else {
        print_issue_detail(&store.config, &issues, issue)?;
        Ok(())
    }
}

fn update(args: UpdateArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;

    if let Some(reference) = &args.reference {
        let issue = Store::resolve_issue(&issues, &args.issue)?;
        Store::ensure_ref_available(&issues, reference, Some(issue.id))?;
    }

    let issue = Store::resolve_issue_mut(&mut issues, &args.issue)?;
    if let Some(title) = args.title {
        issue.title = title;
    }
    if let Some(body) = args.body {
        issue.body = body;
    }
    if let Some(status) = args.status {
        if store.config.status_is_resolved(&status) && issue.closed_at.is_none() {
            let now = now_rfc3339()?;
            issue.closed_at = Some(now);
        } else if !store.config.status_is_resolved(&status) {
            issue.closed_at = None;
        }
        issue.status = status;
    }
    if let Some(issue_type) = args.issue_type {
        issue.kind = issue_type;
    }
    if let Some(priority) = args.priority {
        issue.priority = priority;
    }
    if args.clear_labels || !args.labels.is_empty() {
        issue.labels = normalise_labels(args.labels);
    }
    if args.clear_assignee {
        issue.assignee = None;
    }
    if args.assignee.is_some() {
        issue.assignee = normalise_optional(args.assignee);
    }
    if let Some(reference) = args.reference {
        issue.reference = reference;
    }

    let now = now_rfc3339()?;
    issue.touch(now);
    let updated = issue.clone();
    store.save_issue(&updated)?;
    emit_issue(&store.config, &issues, &updated, json)
}

fn rename_ref(args: RefArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let issue = Store::resolve_issue(&issues, &args.issue)?;
    let reference = if let Some(reference) = args.reference {
        Store::ensure_ref_available(&issues, &reference, Some(issue.id))?;
        reference
    } else {
        let reference = store.generated_ref(&issues)?;
        Store::ensure_ref_available(&issues, &reference, Some(issue.id))?;
        reference
    };

    let issue = Store::resolve_issue_mut(&mut issues, &args.issue)?;
    issue.reference = reference;
    let now = now_rfc3339()?;
    issue.touch(now);
    let updated = issue.clone();
    store.save_issue(&updated)?;
    emit_issue(&store.config, &issues, &updated, json)
}

fn claim(args: ClaimArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let issue = Store::resolve_issue_mut(&mut issues, &args.issue)?;
    issue.assignee = Some(args.assignee.unwrap_or_else(default_actor));
    let now = now_rfc3339()?;
    issue.touch(now);
    let updated = issue.clone();
    store.save_issue(&updated)?;
    emit_issue(&store.config, &issues, &updated, json)
}

fn block(args: BlockArgs, json: bool) -> Result<()> {
    add_blockers(&args.issue, args.blockers, json)
}

fn link(args: LinkArgs, json: bool) -> Result<()> {
    add_blockers(&args.issue, vec![args.blocker], json)
}

fn close(args: CloseArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let issue = Store::resolve_issue_mut(&mut issues, &args.issue)?;
    let now = now_rfc3339()?;
    issue.status.clone_from(&store.config.closed_status);
    issue.closed_at = Some(now.clone());
    issue.touch(now.clone());

    if let Some(reason) = normalise_optional(args.reason) {
        issue
            .comments
            .push(Comment::new(default_actor(), reason, now));
    }

    let updated = issue.clone();
    store.save_issue(&updated)?;
    emit_issue(&store.config, &issues, &updated, json)
}

fn reopen(args: &ReopenArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let issue = Store::resolve_issue_mut(&mut issues, &args.issue)?;
    issue.status.clone_from(&store.config.default_status);
    issue.closed_at = None;
    let now = now_rfc3339()?;
    issue.touch(now);
    let updated = issue.clone();
    store.save_issue(&updated)?;
    emit_issue(&store.config, &issues, &updated, json)
}

fn comment(args: CommentArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let issue = Store::resolve_issue_mut(&mut issues, &args.issue)?;
    let author = args.author.unwrap_or_else(default_actor);
    let now = now_rfc3339()?;
    issue
        .comments
        .push(Comment::new(author, args.body, now.clone()));
    issue.touch(now);
    let updated = issue.clone();
    store.save_issue(&updated)?;
    emit_issue(&store.config, &issues, &updated, json)
}

fn export(args: ExportArgs) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let issues = store.load_issues()?;
    let ExportFormat::Json(json_args) = args.format;
    let view = ExportView::new(&store.config, &issues)?;
    print_json(&view, json_args.pretty)
}

fn add_blockers(issue_identifier: &str, blockers: Vec<String>, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let blocker_ids = resolve_many(&issues, blockers)?;
    let issue = Store::resolve_issue_mut(&mut issues, issue_identifier)?;

    for blocker_id in blocker_ids {
        ensure!(
            blocker_id != issue.id,
            SelfDependencySnafu {
                issue: issue.reference.clone()
            }
        );
        if !issue.blocked_by.contains(&blocker_id) {
            issue.blocked_by.push(blocker_id);
        }
    }

    issue.blocked_by.sort();
    let now = now_rfc3339()?;
    issue.touch(now);
    let updated = issue.clone();
    store.save_issue(&updated)?;
    emit_issue(&store.config, &issues, &updated, json)
}

fn resolve_many(issues: &[Issue], identifiers: Vec<String>) -> Result<Vec<Uuid>> {
    let mut ids = Vec::new();
    for identifier in identifiers {
        let issue = Store::resolve_issue(issues, &identifier)?;
        if !ids.contains(&issue.id) {
            ids.push(issue.id);
        }
    }
    ids.sort();
    Ok(ids)
}

fn default_actor() -> String {
    env::var("GITRACK_ACTOR")
        .or_else(|_| env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::Error, model::Config};

    #[test]
    fn open_unclaimed_issue_with_closed_blocker_is_ready() {
        let config = Config::new("gitrack".to_string(), "issues".to_string());
        let mut blocker = test_issue("gitrack-blocker", "closed");
        blocker.closed_at = Some("2026-06-25T10:00:00Z".to_string());
        let mut issue = test_issue("gitrack-work", "open");
        issue.blocked_by.push(blocker.id);
        let issues = vec![blocker, issue.clone()];
        let by_id = issue_map(&issues);

        assert!(issue_is_ready(&config, &issue, &by_id).expect("readiness"));
    }

    #[test]
    fn open_issue_with_open_blocker_is_not_ready() {
        let config = Config::new("gitrack".to_string(), "issues".to_string());
        let blocker = test_issue("gitrack-blocker", "open");
        let mut issue = test_issue("gitrack-work", "open");
        issue.blocked_by.push(blocker.id);
        let issues = vec![blocker, issue.clone()];
        let by_id = issue_map(&issues);

        assert!(!issue_is_ready(&config, &issue, &by_id).expect("readiness"));
    }

    #[test]
    fn claimed_issue_is_not_ready() {
        let config = Config::new("gitrack".to_string(), "issues".to_string());
        let mut issue = test_issue("gitrack-work", "open");
        issue.assignee = Some("agent".to_string());
        let issues = vec![issue.clone()];
        let by_id = issue_map(&issues);

        assert!(!issue_is_ready(&config, &issue, &by_id).expect("readiness"));
    }

    #[test]
    fn issue_with_missing_blocker_returns_structured_error() {
        let config = Config::new("gitrack".to_string(), "issues".to_string());
        let mut issue = test_issue("gitrack-work", "open");
        issue.blocked_by.push(Uuid::now_v7());
        let issues = vec![issue.clone()];
        let by_id = issue_map(&issues);

        let error = issue_is_ready(&config, &issue, &by_id).expect_err("missing blocker");

        assert!(matches!(error, Error::MissingDependency { .. }));
    }

    fn test_issue(reference: &str, status: &str) -> Issue {
        Issue::new(NewIssue {
            id: Uuid::now_v7(),
            reference: reference.to_string(),
            title: format!("Issue {reference}"),
            body: String::new(),
            status: status.to_string(),
            kind: "task".to_string(),
            priority: 3,
            labels: Vec::new(),
            assignee: None,
            blocked_by: Vec::new(),
            now: "2026-06-25T10:00:00Z".to_string(),
        })
    }
}
