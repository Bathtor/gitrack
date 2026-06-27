//! CLI argument parsing and command execution.

#[cfg(feature = "beads-import")]
use std::path::PathBuf;
use std::{env, path::Path};

use clap::{Parser, Subcommand};
use snafu::ensure;
use uuid::Uuid;

use crate::{
    agents::update_agents_file,
    error::{
        Error, InvalidRelationshipCommandSnafu, InvalidStatusSnafu, ResolvedIssueSnafu, Result,
    },
    model::{
        Comment, Config, DEFAULT_ISSUE_PRIORITY, DEFAULT_ISSUE_TYPE, Issue, IssueKind, IssueLink,
        IssueRef, IssueStatus, NewIssue, now_rfc3339,
    },
    readiness::{issue_is_ready, issue_map},
    store::{DEFAULT_ISSUES_DIR, Store, normalise_labels, normalise_optional},
    views::{
        ExportView, HumanPalette, InitView, IssueListStats, IssueListView, emit_issue,
        print_issue_detail, print_issue_summaries, print_json, sort_issue_refs,
    },
};

const DEFAULT_LINK_LABEL: &str = "relates to";

#[derive(Debug, Parser)]
#[command(
    version,
    about = "A small Git-native issue tracker",
    disable_help_subcommand = true,
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
        after_help = "Creates .gitrack/config.toml and the configured issue directory. The default issue directory is ./issues. New issues default to type `task` and priority 3 unless configured during init; those defaults can later be changed by editing .gitrack/config.toml directly."
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
    #[command(
        about = "Create a relationship between two issues",
        after_help = "Defaults to a free-form `relates to` link. Use --child for hierarchy, --blocked-by for ordering constraints, --label for one-way context, and --bidirectional for two free-form links."
    )]
    Link(LinkArgs),
    #[command(
        about = "Remove a relationship between two issues",
        after_help = "Uses the same selector flags as link. Defaults to removing a free-form `relates to` link."
    )]
    Unlink(UnlinkArgs),
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
    #[command(about = "Manage AGENTS.md gitrack instructions")]
    Agents(AgentsArgs),
    #[command(about = "Export issue data")]
    Export(ExportArgs),
    #[cfg(feature = "beads-import")]
    #[command(name = "import-beads", hide = true)]
    ImportBeads(BeadsImportArgs),
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

    #[arg(
        long,
        default_value = DEFAULT_ISSUE_TYPE,
        help = "Default issue type for create when --type is omitted"
    )]
    default_type: Option<String>,

    #[arg(
        long,
        default_value_t = DEFAULT_ISSUE_PRIORITY,
        help = "Default issue priority for create when --priority is omitted"
    )]
    default_priority: u8,

    #[arg(long, help = "Skip creating or updating AGENTS.md instructions")]
    no_agents: bool,
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

    #[arg(
        short = 'n',
        long = "limit",
        value_name = "COUNT",
        help = "Maximum number of matching issues to show"
    )]
    limit: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ReadyArgs {
    #[arg(
        short = 'n',
        long = "limit",
        value_name = "COUNT",
        help = "Maximum number of ready issues to show"
    )]
    limit: Option<usize>,
}

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
pub(crate) struct LinkArgs {
    #[arg(help = "Source issue ref or UUID")]
    source: String,

    #[arg(help = "Target issue ref or UUID")]
    target: String,

    #[arg(long, conflicts_with_all = ["blocked_by", "label"], help = "Make TARGET a child of SOURCE")]
    child: bool,

    #[arg(long = "blocked-by", conflicts_with_all = ["child", "label"], help = "Make SOURCE blocked by TARGET")]
    blocked_by: bool,

    #[arg(long, value_name = "LABEL", conflicts_with_all = ["child", "blocked_by"], help = "Free-form relationship label; defaults to `relates to`")]
    label: Option<String>,

    #[arg(long, conflicts_with_all = ["child", "blocked_by"], help = "For free-form links, also add TARGET -> SOURCE")]
    bidirectional: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct UnlinkArgs {
    #[arg(help = "Source issue ref or UUID")]
    source: String,

    #[arg(help = "Target issue ref or UUID")]
    target: String,

    #[arg(long, conflicts_with_all = ["blocked_by", "label"], help = "Remove TARGET as a child of SOURCE")]
    child: bool,

    #[arg(long = "blocked-by", conflicts_with_all = ["child", "label"], help = "Remove TARGET as a blocker of SOURCE")]
    blocked_by: bool,

    #[arg(long, value_name = "LABEL", conflicts_with_all = ["child", "blocked_by"], help = "Free-form relationship label; defaults to `relates to`")]
    label: Option<String>,

    #[arg(long, conflicts_with_all = ["child", "blocked_by"], help = "For free-form links, also remove TARGET -> SOURCE")]
    bidirectional: bool,
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
pub(crate) struct AgentsArgs {
    #[command(subcommand)]
    command: AgentsCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AgentsCommand {
    #[command(
        about = "Create or update gitrack instructions in AGENTS.md",
        after_help = "The managed gitrack block is updated in place. --with-workflow appends an editable suggested workflow section."
    )]
    Update(AgentsUpdateArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct AgentsUpdateArgs {
    #[arg(
        long,
        help = "Append an editable suggested workflow section after updating the managed block"
    )]
    with_workflow: bool,
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

#[cfg(feature = "beads-import")]
#[derive(Debug, clap::Args)]
pub(crate) struct BeadsImportArgs {
    #[arg(help = "Path to a Beads JSONL issue export")]
    path: PathBuf,
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
        Command::Ready(args) => ready(&args, cli.json),
        Command::Show(args) => show(&args, cli.json),
        Command::Update(args) => update(args, cli.json),
        Command::Ref(args) => rename_ref(args, cli.json),
        Command::Claim(args) => claim(args, cli.json),
        Command::Link(args) => link(args, cli.json),
        Command::Unlink(args) => unlink(args, cli.json),
        Command::Close(args) => close(args, cli.json),
        Command::Reopen(args) => reopen(&args, cli.json),
        Command::Comment(args) => comment(args, cli.json),
        Command::Agents(args) => agents(args, cli.json),
        Command::Export(args) => export(args),
        #[cfg(feature = "beads-import")]
        Command::ImportBeads(args) => crate::beads_import::import_beads(&args.path, cli.json),
    }
}

fn init(args: InitArgs, json: bool) -> Result<()> {
    let store = Store::init(
        Path::new("."),
        args.prefix,
        args.issue_dir,
        args.default_type,
        Some(args.default_priority),
    )?;
    let agents = if args.no_agents {
        None
    } else {
        let update = update_agents_file(&store.root, false)?;
        Some(update)
    };

    if json {
        print_json(&InitView::from_store(&store, agents), true)?;
    } else {
        println!(
            "Initialised issue store at {} with config {} and ref prefix `{}`",
            store.issues_dir.display(),
            store.config_path.display(),
            store.config.ref_prefix
        );
        if let Some(agents) = agents {
            agents.print_human();
        }
    }
    Ok(())
}

fn create(args: CreateArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let id = Uuid::now_v7();
    let reference = match args.reference {
        Some(reference) => {
            let reference = IssueRef::parse(reference)?;
            Store::ensure_ref_available(&issues, &reference, None)?;
            reference
        }
        None => store.generated_ref(&issues)?,
    };
    let blocked_by = resolve_many(&issues, args.blocked_by)?;
    let now = now_rfc3339()?;
    let issue_type = match args.issue_type {
        Some(issue_type) => IssueKind::parse(issue_type)?,
        None => store.config.default_issue_type.clone(),
    };
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
        blocked_by: blocked_by.clone(),
        now: now.clone(),
    });
    let modified_blockers = add_blocked_issue_to_blockers(&mut issues, &blocked_by, issue.id, &now);

    store.save_issue(&issue)?;
    save_issues_by_id(&store, &issues, &modified_blockers)?;
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
    let mut filtered = issues
        .iter()
        .filter(|issue| {
            if let Some(status) = status_filter {
                issue.status == status
            } else {
                args.all || !issue.status.is_resolved()
            }
        })
        .collect::<Vec<_>>();
    sort_issue_refs(&mut filtered);
    let limit = args.limit.unwrap_or(store.config.default_list_limit);
    let selection = LimitedIssueSelection::new(filtered, limit);

    emit_issue_list(&store.config, &issues, selection, json)
}

fn ready(args: &ReadyArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let issues = store.load_issues()?;
    let by_id = issue_map(&issues);
    let mut ready = Vec::new();
    for issue in &issues {
        if issue_is_ready(issue, &by_id)? {
            ready.push(issue);
        }
    }
    sort_issue_refs(&mut ready);
    let limit = args.limit.unwrap_or(store.config.default_list_limit);
    let selection = LimitedIssueSelection::new(ready, limit);

    emit_issue_list(&store.config, &issues, selection, json)
}

fn emit_issue_list(
    config: &Config,
    all_issues: &[Issue],
    selection: LimitedIssueSelection<'_>,
    json: bool,
) -> Result<()> {
    if json {
        let view = IssueListView::new(config, all_issues, selection.issues, selection.stats)?;
        print_json(&view, true)
    } else {
        let palette = HumanPalette::stdout();
        print_issue_summaries(&palette, all_issues, &selection.issues)?;
        print_limit_footer(&selection.stats);
        Ok(())
    }
}

fn print_limit_footer(stats: &IssueListStats) {
    if stats.skipped() > 0 {
        println!(
            "Showing {} of {} tasks; {} hidden by limit. Use -n <COUNT> to change the limit.",
            stats.shown(),
            stats.total(),
            stats.skipped()
        );
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

    let reference = if let Some(reference) = args.reference {
        let reference = IssueRef::parse(reference)?;
        let issue = Store::resolve_issue(&issues, &args.issue)?;
        Store::ensure_ref_available(&issues, &reference, Some(issue.id))?;
        Some(reference)
    } else {
        None
    };

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
        issue.kind = IssueKind::parse(issue_type)?;
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
    if let Some(reference) = reference {
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
    let RefCommandIssues {
        mut issues,
        repair_ref_aliases_after_save,
    } = load_issues_for_ref_command(&store, &args.issue)?;
    let issue = Store::resolve_issue(&issues, &args.issue)?;
    let reference = if let Some(reference) = args.reference {
        let reference = IssueRef::parse(reference)?;
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
    if repair_ref_aliases_after_save {
        store.reconcile_ref_aliases(&issues)?;
    }
    emit_issue(&store.config, &issues, &updated, json)
}

/// Load issues for `gitrack ref`, allowing UUID-targeted ref repair.
///
/// Normal loading rejects duplicate refs and invalid aliases before command
/// dispatch can resolve a target. During a merge ref clash, identifying the
/// issue by UUID is still unambiguous, so this falls back to the store's repair
/// loader only for that narrow case.
fn load_issues_for_ref_command(store: &Store, identifier: &str) -> Result<RefCommandIssues> {
    match store.load_issues() {
        Ok(issues) => Ok(RefCommandIssues {
            issues,
            repair_ref_aliases_after_save: false,
        }),
        Err(error)
            if Uuid::parse_str(identifier).is_ok()
                && matches!(
                    error,
                    Error::DuplicateIssueRef { .. }
                        | Error::MissingRefAlias { .. }
                        | Error::InvalidRefAlias { .. }
                        | Error::RefAliasTargetMismatch { .. }
                ) =>
        {
            let issues = store.load_issues_for_ref_repair()?;
            Ok(RefCommandIssues {
                issues,
                repair_ref_aliases_after_save: true,
            })
        }
        Err(error) => Err(error),
    }
}

fn claim(args: ClaimArgs, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let issue = Store::resolve_issue_mut(&mut issues, &args.issue)?;
    ensure!(
        !issue.status.is_resolved(),
        ResolvedIssueSnafu {
            reference: issue.reference.to_string(),
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

fn link(args: LinkArgs, json: bool) -> Result<()> {
    let kind = relationship_kind_from_flags(args.child, args.blocked_by, args.label)?;
    mutate_relationship(
        RelationshipMutation::Add,
        &args.source,
        &args.target,
        &kind,
        args.bidirectional,
        json,
    )
}

fn unlink(args: UnlinkArgs, json: bool) -> Result<()> {
    let kind = relationship_kind_from_flags(args.child, args.blocked_by, args.label)?;
    mutate_relationship(
        RelationshipMutation::Remove,
        &args.source,
        &args.target,
        &kind,
        args.bidirectional,
        json,
    )
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

fn agents(args: AgentsArgs, json: bool) -> Result<()> {
    match args.command {
        AgentsCommand::Update(args) => agents_update(&args, json),
    }
}

fn agents_update(args: &AgentsUpdateArgs, json: bool) -> Result<()> {
    let root = Store::root_for_worktree(Path::new("."))?;
    let result = update_agents_file(&root, args.with_workflow)?;
    if json {
        print_json(&result, true)
    } else {
        result.print_human();
        Ok(())
    }
}

fn export(args: ExportArgs) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let issues = store.load_issues()?;
    let ExportFormat::Json(json_args) = args.format;
    let view = ExportView::new(&store.config, &issues)?;
    print_json(&view, json_args.pretty)
}

fn mutate_relationship(
    mutation: RelationshipMutation,
    source_identifier: &str,
    target_identifier: &str,
    kind: &RelationshipKind,
    bidirectional: bool,
    json: bool,
) -> Result<()> {
    ensure!(
        !bidirectional || matches!(kind, RelationshipKind::Label(_)),
        InvalidRelationshipCommandSnafu {
            reason: "--bidirectional only applies to free-form links".to_string()
        }
    );

    let store = Store::open(Path::new("."))?;
    let mut issues = store.load_issues()?;
    let source = Store::resolve_issue(&issues, source_identifier)?;
    let source_id = source.id;
    let source_reference = source.reference.to_string();
    let target = Store::resolve_issue(&issues, target_identifier)?;
    let target_id = target.id;
    let target_reference = target.reference.to_string();
    let endpoints = RelationshipEndpoints {
        source_id,
        source_reference: &source_reference,
        target_id,
        target_reference: &target_reference,
    };
    ensure!(
        source_id != target_id,
        InvalidRelationshipCommandSnafu {
            reason: format!(
                "source issue `{source_reference}` and target issue `{target_reference}` must be different"
            )
        }
    );

    let now = now_rfc3339()?;
    let modified_ids = match mutation {
        RelationshipMutation::Add => {
            add_relationship(&mut issues, &endpoints, kind, bidirectional, &now)?
        }
        RelationshipMutation::Remove => {
            remove_relationship(&mut issues, &endpoints, kind, bidirectional, &now)
        }
    };

    let updated = issues
        .iter()
        .find(|issue| issue.id == endpoints.source_id)
        .expect("resolved issue id must remain present")
        .clone();
    save_issues_by_id(&store, &issues, &modified_ids)?;
    emit_issue(&store.config, &issues, &updated, json)
}

/// Parse relationship selector flags into one concrete relationship kind.
fn relationship_kind_from_flags(
    child: bool,
    blocked_by: bool,
    label: Option<String>,
) -> Result<RelationshipKind> {
    let selector_count =
        usize::from(child) + usize::from(blocked_by) + usize::from(label.is_some());
    ensure!(
        selector_count <= 1,
        InvalidRelationshipCommandSnafu {
            reason: "--child, --blocked-by, and --label are mutually exclusive".to_string()
        }
    );

    if child {
        Ok(RelationshipKind::Child)
    } else if blocked_by {
        Ok(RelationshipKind::BlockedBy)
    } else {
        let label = label.unwrap_or_else(|| DEFAULT_LINK_LABEL.to_string());
        let label = label.trim();
        ensure!(
            !label.is_empty(),
            InvalidRelationshipCommandSnafu {
                reason: "link label must not be empty".to_string()
            }
        );
        Ok(RelationshipKind::Label(label.to_string()))
    }
}

/// Add one relationship and return the IDs of issues changed by the operation.
fn add_relationship(
    issues: &mut [Issue],
    endpoints: &RelationshipEndpoints<'_>,
    kind: &RelationshipKind,
    bidirectional: bool,
    now: &str,
) -> Result<Vec<Uuid>> {
    let mut modified_ids = Vec::new();
    match kind {
        RelationshipKind::Child => {
            ensure_child_can_be_linked(issues, endpoints)?;
            if push_unique_sorted(
                &mut issue_mut_by_id(issues, endpoints.source_id).children,
                endpoints.target_id,
            ) {
                touch_issue_by_id(issues, endpoints.source_id, now, &mut modified_ids);
            }
            let child = issue_mut_by_id(issues, endpoints.target_id);
            if child.parent != Some(endpoints.source_id) {
                child.parent = Some(endpoints.source_id);
                touch_issue_by_id(issues, endpoints.target_id, now, &mut modified_ids);
            }
        }
        RelationshipKind::BlockedBy => {
            if push_unique_sorted(
                &mut issue_mut_by_id(issues, endpoints.source_id).blocked_by,
                endpoints.target_id,
            ) {
                touch_issue_by_id(issues, endpoints.source_id, now, &mut modified_ids);
            }
            if push_unique_sorted(
                &mut issue_mut_by_id(issues, endpoints.target_id).blocks,
                endpoints.source_id,
            ) {
                touch_issue_by_id(issues, endpoints.target_id, now, &mut modified_ids);
            }
        }
        RelationshipKind::Label(label) => {
            add_labelled_link(
                issues,
                endpoints.source_id,
                endpoints.target_id,
                label,
                now,
                &mut modified_ids,
            );
            if bidirectional {
                add_labelled_link(
                    issues,
                    endpoints.target_id,
                    endpoints.source_id,
                    label,
                    now,
                    &mut modified_ids,
                );
            }
        }
    }

    Ok(modified_ids)
}

/// Remove one relationship and return the IDs of issues changed by the operation.
fn remove_relationship(
    issues: &mut [Issue],
    endpoints: &RelationshipEndpoints<'_>,
    kind: &RelationshipKind,
    bidirectional: bool,
    now: &str,
) -> Vec<Uuid> {
    let mut modified_ids = Vec::new();
    match kind {
        RelationshipKind::Child => {
            if remove_uuid(
                &mut issue_mut_by_id(issues, endpoints.source_id).children,
                endpoints.target_id,
            ) {
                touch_issue_by_id(issues, endpoints.source_id, now, &mut modified_ids);
            }
            let child = issue_mut_by_id(issues, endpoints.target_id);
            if child.parent == Some(endpoints.source_id) {
                child.parent = None;
                touch_issue_by_id(issues, endpoints.target_id, now, &mut modified_ids);
            }
        }
        RelationshipKind::BlockedBy => {
            if remove_uuid(
                &mut issue_mut_by_id(issues, endpoints.source_id).blocked_by,
                endpoints.target_id,
            ) {
                touch_issue_by_id(issues, endpoints.source_id, now, &mut modified_ids);
            }
            if remove_uuid(
                &mut issue_mut_by_id(issues, endpoints.target_id).blocks,
                endpoints.source_id,
            ) {
                touch_issue_by_id(issues, endpoints.target_id, now, &mut modified_ids);
            }
        }
        RelationshipKind::Label(label) => {
            remove_labelled_link(
                issues,
                endpoints.source_id,
                endpoints.target_id,
                label,
                now,
                &mut modified_ids,
            );
            if bidirectional {
                remove_labelled_link(
                    issues,
                    endpoints.target_id,
                    endpoints.source_id,
                    label,
                    now,
                    &mut modified_ids,
                );
            }
        }
    }
    modified_ids
}

/// Reject assigning a child that already belongs to a different parent.
fn ensure_child_can_be_linked(
    issues: &[Issue],
    endpoints: &RelationshipEndpoints<'_>,
) -> Result<()> {
    let child = issues
        .iter()
        .find(|issue| issue.id == endpoints.target_id)
        .expect("resolved target issue id must remain present");
    ensure!(
        child.parent.is_none() || child.parent == Some(endpoints.source_id),
        InvalidRelationshipCommandSnafu {
            reason: format!(
                "target issue `{}` already has a different parent",
                endpoints.target_reference
            )
        }
    );
    ensure!(
        !is_ancestor(issues, endpoints.source_id, endpoints.target_id),
        InvalidRelationshipCommandSnafu {
            reason: format!(
                "target issue `{}` is already an ancestor of source issue `{}`",
                endpoints.target_reference, endpoints.source_reference
            )
        }
    );
    Ok(())
}

/// Return true when `ancestor_id` is in the issue's parent chain.
fn is_ancestor(issues: &[Issue], issue_id: Uuid, ancestor_id: Uuid) -> bool {
    let mut current_id = issue_id;
    while let Some(current) = issues.iter().find(|issue| issue.id == current_id) {
        let Some(parent_id) = current.parent else {
            return false;
        };
        if parent_id == ancestor_id {
            return true;
        }
        current_id = parent_id;
    }
    false
}

/// Add one labelled link if missing and record the changed source issue.
fn add_labelled_link(
    issues: &mut [Issue],
    source_id: Uuid,
    target_id: Uuid,
    label: &str,
    now: &str,
    modified_ids: &mut Vec<Uuid>,
) {
    if push_unique_link(
        &mut issue_mut_by_id(issues, source_id).links,
        target_id,
        label,
    ) {
        touch_issue_by_id(issues, source_id, now, modified_ids);
    }
}

/// Remove one labelled link if present and record the changed source issue.
fn remove_labelled_link(
    issues: &mut [Issue],
    source_id: Uuid,
    target_id: Uuid,
    label: &str,
    now: &str,
    modified_ids: &mut Vec<Uuid>,
) {
    if remove_link(
        &mut issue_mut_by_id(issues, source_id).links,
        target_id,
        label,
    ) {
        touch_issue_by_id(issues, source_id, now, modified_ids);
    }
}

/// Resolve a mutable issue by UUID after command arguments have been validated.
fn issue_mut_by_id(issues: &mut [Issue], id: Uuid) -> &mut Issue {
    issues
        .iter_mut()
        .find(|issue| issue.id == id)
        .expect("resolved issue id must remain present")
}

/// Touch one issue and record it for persistence.
fn touch_issue_by_id(issues: &mut [Issue], id: Uuid, now: &str, modified_ids: &mut Vec<Uuid>) {
    let issue = issue_mut_by_id(issues, id);
    issue.touch(now.to_string());
    modified_ids.push(id);
}

/// Remove one UUID from a relationship vector.
fn remove_uuid(values: &mut Vec<Uuid>, value: Uuid) -> bool {
    let before_len = values.len();
    values.retain(|candidate| *candidate != value);
    before_len != values.len()
}

/// Insert a labelled link unless an identical target and label already exists.
fn push_unique_link(links: &mut Vec<IssueLink>, target: Uuid, label: &str) -> bool {
    if links
        .iter()
        .any(|link| link.target == target && link.label == label)
    {
        false
    } else {
        links.push(IssueLink {
            target,
            label: label.to_string(),
        });
        links.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.target.cmp(&right.target))
        });
        true
    }
}

/// Remove labelled links matching both target and label.
fn remove_link(links: &mut Vec<IssueLink>, target: Uuid, label: &str) -> bool {
    let before_len = links.len();
    links.retain(|link| link.target != target || link.label != label);
    before_len != links.len()
}

/// Add the blocked issue UUID to each blocker and return changed blocker IDs.
fn add_blocked_issue_to_blockers(
    issues: &mut [Issue],
    blocker_ids: &[Uuid],
    blocked_id: Uuid,
    now: &str,
) -> Vec<Uuid> {
    let mut modified_ids = Vec::new();
    for blocker_id in blocker_ids {
        let blocker = issues
            .iter_mut()
            .find(|issue| issue.id == *blocker_id)
            .expect("resolved blocker id must remain present");
        if push_unique_sorted(&mut blocker.blocks, blocked_id) {
            blocker.touch(now.to_string());
            modified_ids.push(blocker.id);
        }
    }
    modified_ids
}

/// Persist each changed issue once, preserving the caller's loaded issue set.
fn save_issues_by_id(store: &Store, issues: &[Issue], ids: &[Uuid]) -> Result<()> {
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    for id in ids {
        let issue = issues
            .iter()
            .find(|issue| issue.id == id)
            .expect("modified issue id must remain present");
        store.save_issue(issue)?;
    }
    Ok(())
}

/// Insert a UUID into a sorted vector unless it is already present.
fn push_unique_sorted(values: &mut Vec<Uuid>, value: Uuid) -> bool {
    if values.contains(&value) {
        false
    } else {
        values.push(value);
        values.sort();
        true
    }
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

/// Selected issues after applying the list limit, plus stats about truncation.
struct LimitedIssueSelection<'issues> {
    /// Issues to render after the selected task set has been truncated.
    issues: Vec<&'issues Issue>,
    /// Counts computed before and after truncating the selected task set.
    stats: IssueListStats,
}

impl<'issues> LimitedIssueSelection<'issues> {
    fn new(mut issues: Vec<&'issues Issue>, limit: usize) -> Self {
        let total = issues.len();
        issues.truncate(limit);
        let shown = issues.len();
        let stats = IssueListStats::new(limit, total, shown);
        Self { issues, stats }
    }
}

/// Issues loaded for `gitrack ref`, plus whether aliases need full repair.
struct RefCommandIssues {
    issues: Vec<Issue>,
    /// True when issues were loaded from a ref-invalid worktree and alias state
    /// must be reconciled after saving the renamed issue.
    repair_ref_aliases_after_save: bool,
}

/// Resolved source and target endpoints for one relationship command.
struct RelationshipEndpoints<'references> {
    /// UUID of the source issue named first on the command line.
    source_id: Uuid,
    /// Current user-visible ref of the source issue for diagnostics.
    source_reference: &'references str,
    /// UUID of the target issue named second on the command line.
    target_id: Uuid,
    /// Current user-visible ref of the target issue for diagnostics.
    target_reference: &'references str,
}

/// Relationship kind selected by the `link` and `unlink` selector flags.
enum RelationshipKind {
    /// Parent/child hierarchy: source becomes parent of target.
    Child,
    /// Blocking dependency: source is blocked by target.
    BlockedBy,
    /// One-way free-form relationship labelled by the contained text.
    Label(String),
}

/// Add or remove operation selected by the top-level command.
#[derive(Clone, Copy)]
enum RelationshipMutation {
    /// Add the selected relationship.
    Add,
    /// Remove the selected relationship.
    Remove,
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
            reference: IssueRef::parse(reference).expect("valid ref"),
            title: format!("Issue {reference}"),
            body: String::new(),
            status,
            kind: IssueKind::parse("task").expect("valid kind"),
            priority: 3,
            labels: Vec::new(),
            assignee: None,
            blocked_by: Vec::new(),
            now: "2026-06-25T10:00:00Z".to_string(),
        })
    }
}
