//! CLI argument parsing and command execution.

use std::{env, path::Path};

use clap::{Parser, Subcommand};
use snafu::ensure;
use uuid::Uuid;

use crate::{
    error::{InvalidStatusSnafu, ResolvedIssueSnafu, Result, SelfDependencySnafu},
    model::{Comment, Issue, IssueStatus, NewIssue, now_rfc3339},
    readiness::{issue_is_ready, issue_map},
    store::{DEFAULT_ISSUES_DIR, Store, normalise_labels, normalise_optional},
    views::{
        ExportView, InitView, IssueListView, emit_issue, print_issue_detail, print_issue_summary,
        print_json,
    },
};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "A small Git-native issue tracker",
    long_about = "gitrack stores issue state as ordinary tracked files in the current Git working tree. Use --json for deterministic output suitable for coding agents.",
    after_help = "Examples:\n  gitrack init\n  gitrack --json create \"Fix parser\" --blocked-by gitrack-abc\n  gitrack ready\n  gitrack claim gitrack-abc --assignee agent\n  gitrack close gitrack-abc --reason completed\n  gitrack export json --pretty"
)]
pub struct Cli {
    #[arg(long, global = true, help = "Emit deterministic JSON where supported")]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    #[command(
        about = "Initialise issue tracking files in this Git working tree",
        after_help = "Creates .gitrack/config.toml and the configured issue directory. The default issue directory is ./issues."
    )]
    Init(InitArgs),
    #[command(
        about = "Create a new issue",
        after_help = "Refs are generated automatically unless --ref is provided. Use explicit refs mainly for child issues such as parent.1."
    )]
    Create(CreateArgs),
    #[command(
        about = "List issues",
        after_help = "By default resolved issues are hidden. Use --all to include closed issues."
    )]
    List(ListArgs),
    #[command(
        about = "List work that is open, unclaimed, and unblocked",
        after_help = "Ready work excludes claimed work and work blocked by issues that are not closed."
    )]
    Ready(ReadyArgs),
    #[command(about = "Show one issue by ref or UUID")]
    Show(ShowArgs),
    #[command(
        about = "Update editable issue fields",
        after_help = "Status values are fixed: open, in-progress, closed. Use close/reopen/claim for common workflow transitions."
    )]
    Update(UpdateArgs),
    #[command(
        about = "Rename or regenerate an issue ref",
        after_help = "Omit NEW_REF to generate a fresh automatic ref. Dependencies use UUIDs internally, so ref renames do not rewrite other issue files."
    )]
    Ref(RefArgs),
    #[command(
        about = "Assign an issue and move it to in-progress",
        after_help = "Claiming a closed issue is rejected. Reopen it first if the work should continue."
    )]
    Claim(ClaimArgs),
    #[command(about = "Add one or more blocking dependencies to an issue")]
    Block(BlockArgs),
    #[command(about = "Add one blocking dependency to an issue")]
    Link(LinkArgs),
    #[command(
        about = "Close an issue",
        after_help = "The optional reason is stored as status_reason and also recorded as a comment."
    )]
    Close(CloseArgs),
    #[command(about = "Reopen a closed issue")]
    Reopen(ReopenArgs),
    #[command(alias = "note")]
    #[command(about = "Append a comment to an issue")]
    Comment(CommentArgs),
    #[command(about = "Export issue data")]
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
    #[arg(help = "Issue title")]
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

    #[arg(long, help = "Filter by status: open, in-progress, or closed")]
    status: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ReadyArgs {}

#[derive(Debug, clap::Args)]
pub(crate) struct ShowArgs {
    #[arg(help = "Issue ref or UUID")]
    issue: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct UpdateArgs {
    #[arg(help = "Issue ref or UUID")]
    issue: String,

    #[arg(long, help = "Replace the issue title")]
    title: Option<String>,

    #[arg(long, help = "Replace the issue body")]
    body: Option<String>,

    #[arg(long, help = "Set status: open, in-progress, or closed")]
    status: Option<String>,

    #[arg(long, help = "Set a free-form explanation for the current status")]
    status_reason: Option<String>,

    #[arg(long, help = "Clear any status explanation")]
    clear_status_reason: bool,

    #[arg(long = "type", help = "Replace the issue type")]
    issue_type: Option<String>,

    #[arg(long, help = "Replace the issue priority")]
    priority: Option<u8>,

    #[arg(
        long = "label",
        help = "Replace labels; may be repeated or comma-separated"
    )]
    labels: Vec<String>,

    #[arg(long, help = "Clear all labels before applying any --label values")]
    clear_labels: bool,

    #[arg(long, help = "Set the issue assignee")]
    assignee: Option<String>,

    #[arg(long, help = "Clear the issue assignee")]
    clear_assignee: bool,

    #[arg(long = "ref", help = "Rename the user-visible ref")]
    reference: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RefArgs {
    #[arg(help = "Issue ref or UUID")]
    issue: String,

    #[arg(
        value_name = "NEW_REF",
        help = "Explicit new ref; omit to generate a fresh automatic ref"
    )]
    reference: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ClaimArgs {
    #[arg(help = "Issue ref or UUID")]
    issue: String,

    #[arg(long, help = "Assignee; defaults to GITRACK_ACTOR or USER")]
    assignee: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct BlockArgs {
    #[arg(help = "Issue ref or UUID")]
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
    #[arg(help = "Issue ref or UUID")]
    issue: String,
    #[arg(help = "Blocking issue ref or UUID")]
    blocker: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct CloseArgs {
    #[arg(help = "Issue ref or UUID")]
    issue: String,

    #[arg(
        long,
        help = "Optional close reason, for example completed or won't do"
    )]
    reason: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ReopenArgs {
    #[arg(help = "Issue ref or UUID")]
    issue: String,
}

#[derive(Debug, clap::Args)]
pub(crate) struct CommentArgs {
    #[arg(help = "Issue ref or UUID")]
    issue: String,
    #[arg(help = "Comment body")]
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
    #[command(about = "Export all issues as deterministic JSON")]
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
        status: IssueStatus::Open,
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
    let status_filter = if let Some(status) = &args.status {
        let status = parse_status(status)?;
        Some(status)
    } else {
        None
    };
    let filtered = issues
        .iter()
        .filter(|issue| {
            if let Some(status) = status_filter {
                issue.status == status
            } else {
                args.all || !issue.status.is_resolved()
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
        if issue_is_ready(issue, &by_id)? {
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
        let status = parse_status(&status)?;
        if status.is_resolved() && issue.closed_at.is_none() {
            let now = now_rfc3339()?;
            issue.closed_at = Some(now);
        } else if !status.is_resolved() {
            issue.closed_at = None;
            issue.status_reason = None;
        }
        issue.status = status;
    }
    if let Some(status_reason) = args.status_reason {
        issue.status_reason = normalise_optional(Some(status_reason));
    }
    if args.clear_status_reason {
        issue.status_reason = None;
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
    ensure!(
        !issue.status.is_resolved(),
        ResolvedIssueSnafu {
            reference: issue.reference.clone(),
            status: issue.status.to_string()
        }
    );
    issue.assignee = Some(args.assignee.unwrap_or_else(default_actor));
    issue.status = IssueStatus::InProgress;
    issue.status_reason = None;
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
    issue.status = IssueStatus::Closed;
    issue.closed_at = Some(now.clone());
    issue.touch(now.clone());

    let reason = normalise_optional(args.reason);
    issue.status_reason.clone_from(&reason);
    if let Some(reason) = reason {
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
    issue.status = IssueStatus::Open;
    issue.closed_at = None;
    issue.status_reason = None;
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

fn parse_status(value: &str) -> Result<IssueStatus> {
    let Some(status) = IssueStatus::from_name(value) else {
        return InvalidStatusSnafu {
            status: value.to_string(),
        }
        .fail();
    };

    Ok(status)
}

fn default_actor() -> String {
    env::var("GITRACK_ACTOR")
        .or_else(|_| env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn open_unclaimed_issue_with_closed_blocker_is_ready() {
        let mut blocker = test_issue("gitrack-blocker", IssueStatus::Closed);
        blocker.closed_at = Some("2026-06-25T10:00:00Z".to_string());
        let mut issue = test_issue("gitrack-work", IssueStatus::Open);
        issue.blocked_by.push(blocker.id);
        let issues = vec![blocker, issue.clone()];
        let by_id = issue_map(&issues);

        assert!(issue_is_ready(&issue, &by_id).expect("readiness"));
    }

    #[test]
    fn open_issue_with_open_blocker_is_not_ready() {
        let blocker = test_issue("gitrack-blocker", IssueStatus::Open);
        let mut issue = test_issue("gitrack-work", IssueStatus::Open);
        issue.blocked_by.push(blocker.id);
        let issues = vec![blocker, issue.clone()];
        let by_id = issue_map(&issues);

        assert!(!issue_is_ready(&issue, &by_id).expect("readiness"));
    }

    #[test]
    fn claimed_issue_is_not_ready() {
        let mut issue = test_issue("gitrack-work", IssueStatus::Open);
        issue.assignee = Some("agent".to_string());
        let issues = vec![issue.clone()];
        let by_id = issue_map(&issues);

        assert!(!issue_is_ready(&issue, &by_id).expect("readiness"));
    }

    #[test]
    fn issue_with_missing_blocker_returns_structured_error() {
        let mut issue = test_issue("gitrack-work", IssueStatus::Open);
        issue.blocked_by.push(Uuid::now_v7());
        let issues = vec![issue.clone()];
        let by_id = issue_map(&issues);

        let error = issue_is_ready(&issue, &by_id).expect_err("missing blocker");

        assert!(matches!(error, Error::MissingDependency { .. }));
    }

    fn test_issue(reference: &str, status: IssueStatus) -> Issue {
        Issue::new(NewIssue {
            id: Uuid::now_v7(),
            reference: reference.to_string(),
            title: format!("Issue {reference}"),
            body: String::new(),
            status,
            kind: "task".to_string(),
            priority: 3,
            labels: Vec::new(),
            assignee: None,
            blocked_by: Vec::new(),
            now: "2026-06-25T10:00:00Z".to_string(),
        })
    }
}
