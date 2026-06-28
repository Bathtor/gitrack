//! Human and JSON output views for CLI commands.

use std::{
    collections::{HashMap, HashSet},
    env,
    io::{self, IsTerminal, Write},
};

use lscolors::{FontStyle, Indicator, LsColors, Style};
use serde::Serialize;
use snafu::{OptionExt, ResultExt};
use uuid::Uuid;

use crate::{
    agents::AgentsUpdateResult,
    error::{
        HierarchyCycleSnafu, MissingRelationshipTargetSnafu, Result, SerialiseJsonSnafu,
        WriteStdoutSnafu,
    },
    model::{Comment, Config, Issue, IssueKind, IssueLink, IssueRef, IssueStatus},
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
        let palette = HumanPalette::stdout();
        print_issue_summary(&palette, issues, issue);
        Ok(())
    }
}

pub(crate) fn print_issue_summary(palette: &HumanPalette, issues: &[Issue], issue: &Issue) {
    println!("{}", issue_summary_line(palette, issues, issue));
}

pub(crate) fn print_issue_summaries(
    palette: &HumanPalette,
    issues: &[Issue],
    selected: &[&Issue],
) -> Result<()> {
    let roots = summary_tree_with_ancestors(issues, selected)?;
    for root in &roots {
        print_issue_summary_node(palette, issues, root, 0);
    }
    Ok(())
}

pub(crate) fn sort_issue_refs(issues: &mut [&Issue]) {
    issues.sort_by(|left, right| issue_order(left, right));
}

fn issue_summary_line(palette: &HumanPalette, issues: &[Issue], issue: &Issue) -> String {
    let blocked = is_blocked_by_unresolved_issue(issues, issue);
    let marker = palette.paint(status_role(issue, blocked), status_marker(issue, blocked));
    let reference = palette.paint(HumanRole::IssueRef, issue.reference.to_string());
    let title = palette.paint(HumanRole::Title, &issue.title);
    let badge = summary_badge(palette, issue, blocked);
    let blocker_note = blocking_summary(issues, issue)
        .map(|summary| format!("  {}", palette.paint(HumanRole::Blocked, summary)))
        .unwrap_or_default();

    format!("{marker} {reference}  {title}  [{badge}]{blocker_note}")
}

pub(crate) fn print_issue_detail(_config: &Config, issues: &[Issue], issue: &Issue) -> Result<()> {
    let palette = HumanPalette::stdout();
    let by_id = issue_map(issues);
    let ready = issue_is_ready(issue, &by_id)?;
    let blocked = is_blocked_by_unresolved_issue(issues, issue);
    let marker = palette.paint(status_role(issue, blocked), status_marker(issue, blocked));
    let reference = palette.paint(HumanRole::IssueRef, issue.reference.to_string());
    let kind = palette.paint(HumanRole::IssueKind, issue.kind.as_str().to_uppercase());
    let title = palette.paint(HumanRole::Title, &issue.title);
    let badge = detail_badge(&palette, issue, blocked);

    println!("{marker} {reference} [{kind}] · {title}   [{badge}]");
    println!("{}", metadata_line(&palette, issues, issue, ready, blocked));
    println!("{}", timestamp_line(issue));
    println!("{}", palette.paint(HumanRole::Muted, uuid_line(issue)));

    if !issue.body.is_empty() {
        print_section_heading(&palette, "DESCRIPTION");
        println!("{}", issue.body);
    }

    if !issue.children.is_empty() {
        print_section_heading(&palette, "CHILDREN");
        for child_id in &issue.children {
            println!("  {}", dependency_summary(&palette, issues, *child_id));
        }
    }

    if !issue.blocked_by.is_empty() {
        print_section_heading(&palette, "BLOCKERS");
        for blocker_id in &issue.blocked_by {
            println!("  {}", dependency_summary(&palette, issues, *blocker_id));
        }
    }

    if !issue.blocks.is_empty() {
        print_section_heading(&palette, "BLOCKS");
        for blocked_id in &issue.blocks {
            println!("  {}", dependency_summary(&palette, issues, *blocked_id));
        }
    }

    if !issue.links.is_empty() {
        print_section_heading(&palette, "LINKS");
        for link in &issue.links {
            let label = palette.paint(HumanRole::Label, &link.label);
            let target = dependency_summary(&palette, issues, link.target);
            println!("  {label}: {target}");
        }
    }

    if !issue.comments.is_empty() {
        print_section_heading(&palette, "COMMENTS");
        for comment in &issue.comments {
            println!("{}", palette.paint(HumanRole::Divider, COMMENT_DIVIDER));
            println!(
                "{} · {}",
                palette.paint(HumanRole::IssueRef, &comment.author),
                palette.paint(HumanRole::Metadata, &comment.created_at)
            );
            println!("{}", comment.body);
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

/// Colour adapter that maps gitrack display roles onto `LS_COLORS` indicators.
pub(crate) struct HumanPalette {
    enabled: bool,
    colours: Option<LsColors>,
}

impl HumanPalette {
    pub(crate) fn stdout() -> Self {
        Self {
            enabled: env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal(),
            colours: LsColors::from_env(),
        }
    }

    fn paint(&self, role: HumanRole, text: impl AsRef<str>) -> String {
        let text = text.as_ref();
        if !self.enabled {
            return text.to_string();
        }

        let style = self
            .colours
            .as_ref()
            .and_then(|colours| colours.style_for_indicator(role.indicator()))
            .copied()
            .map(|style| role.adjust_style(style))
            .or_else(|| role.fallback_style());

        style.map_or_else(
            || text.to_string(),
            |style| style.to_nu_ansi_term_style().paint(text).to_string(),
        )
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct InitView {
    root: String,
    config_dir: String,
    issues_dir: String,
    config_path: String,
    config: Config,
    agents: Option<AgentsUpdateResult>,
}

impl InitView {
    pub(crate) fn from_store(store: &Store, agents: Option<AgentsUpdateResult>) -> Self {
        Self {
            root: store.root.display().to_string(),
            config_dir: store.dir.display().to_string(),
            issues_dir: store.issues_dir.display().to_string(),
            config_path: store.config_path.display().to_string(),
            config: store.config.clone(),
            agents,
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
    stats: IssueListStats,
}

impl IssueListView {
    pub(crate) fn new(
        config: &Config,
        all_issues: &[Issue],
        issues: Vec<&Issue>,
        stats: IssueListStats,
    ) -> Result<Self> {
        let mut issue_views = Vec::with_capacity(issues.len());
        for issue in issues {
            let issue_view = IssueView::from_issue(config, all_issues, issue)?;
            issue_views.push(issue_view);
        }

        Ok(Self {
            issues: issue_views,
            stats,
        })
    }
}

/// Metadata describing whether a list-style response was truncated.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct IssueListStats {
    /// Maximum selected issue count requested by config or CLI arguments.
    limit: usize,
    /// Selected issue count before applying `limit`.
    total: usize,
    /// Selected issue count after applying `limit`.
    shown: usize,
    /// Selected issue count hidden by `limit`.
    skipped: usize,
}

impl IssueListStats {
    #[must_use]
    pub(crate) fn new(limit: usize, total: usize, shown: usize) -> Self {
        Self {
            limit,
            total,
            shown,
            skipped: total.saturating_sub(shown),
        }
    }

    #[must_use]
    pub(crate) fn skipped(self) -> usize {
        self.skipped
    }

    #[must_use]
    pub(crate) fn shown(self) -> usize {
        self.shown
    }

    #[must_use]
    pub(crate) fn total(self) -> usize {
        self.total
    }
}

const COMMENT_DIVIDER: &str = "────────────────────────────────────────────────────────────";

/// Issue plus already-selected descendants for recursive human summary output.
#[derive(Debug)]
struct DisplayIssueNode<'issues> {
    issue: &'issues Issue,
    children: Vec<DisplayIssueNode<'issues>>,
}

fn print_section_heading(palette: &HumanPalette, heading: &str) {
    println!();
    println!("{}", palette.paint(HumanRole::SectionHeading, heading));
}

fn summary_tree_with_ancestors<'issues>(
    issues: &'issues [Issue],
    selected: &[&'issues Issue],
) -> Result<Vec<DisplayIssueNode<'issues>>> {
    let by_id = issues
        .iter()
        .map(|issue| (issue.id, issue))
        .collect::<HashMap<_, _>>();
    let mut included = HashSet::new();
    for issue in selected {
        include_ancestor_chain(issue, &by_id, &mut included)?;
    }

    let mut roots = Vec::new();
    let mut children_by_parent = HashMap::<Uuid, Vec<Uuid>>::new();
    for id in &included {
        let issue = by_id
            .get(id)
            .copied()
            .context(MissingRelationshipTargetSnafu {
                issue: id.to_string(),
                field: "display",
                target: *id,
            })?;
        if let Some(parent_id) = issue.parent
            && included.contains(&parent_id)
        {
            children_by_parent.entry(parent_id).or_default().push(*id);
        } else {
            roots.push(*id);
        }
    }

    sort_issue_ids(&mut roots, &by_id)?;
    for child_ids in children_by_parent.values_mut() {
        sort_issue_ids(child_ids, &by_id)?;
    }

    let mut path = Vec::new();
    roots
        .into_iter()
        .map(|id| build_issue_summary_node(id, &by_id, &children_by_parent, &mut path))
        .collect()
}

fn include_ancestor_chain(
    issue: &Issue,
    by_id: &HashMap<Uuid, &Issue>,
    included: &mut HashSet<Uuid>,
) -> Result<()> {
    let mut seen = HashSet::new();
    let mut current = Some(issue.id);
    while let Some(id) = current {
        if !seen.insert(id) {
            return HierarchyCycleSnafu {
                issue: issue.reference.to_string(),
                ancestor: id,
            }
            .fail();
        }

        let current_issue = by_id
            .get(&id)
            .copied()
            .context(MissingRelationshipTargetSnafu {
                issue: issue.reference.to_string(),
                field: "parent",
                target: id,
            })?;
        included.insert(id);
        current = current_issue.parent;
    }
    Ok(())
}

fn sort_issue_ids(ids: &mut [Uuid], by_id: &HashMap<Uuid, &Issue>) -> Result<()> {
    for id in ids.iter().copied() {
        by_id.get(&id).context(MissingRelationshipTargetSnafu {
            issue: id.to_string(),
            field: "display",
            target: id,
        })?;
    }

    ids.sort_by(|left, right| {
        let left_issue = by_id[left];
        let right_issue = by_id[right];
        issue_order(left_issue, right_issue)
    });
    Ok(())
}

fn issue_order(left: &Issue, right: &Issue) -> std::cmp::Ordering {
    left.priority
        .cmp(&right.priority)
        .then_with(|| right.updated_at.cmp(&left.updated_at))
        .then_with(|| left.reference.cmp(&right.reference))
        .then_with(|| left.id.cmp(&right.id))
}

fn build_issue_summary_node<'issues>(
    id: Uuid,
    by_id: &HashMap<Uuid, &'issues Issue>,
    children_by_parent: &HashMap<Uuid, Vec<Uuid>>,
    path: &mut Vec<Uuid>,
) -> Result<DisplayIssueNode<'issues>> {
    if path.contains(&id) {
        return HierarchyCycleSnafu {
            issue: id.to_string(),
            ancestor: id,
        }
        .fail();
    }
    path.push(id);

    let issue = by_id
        .get(&id)
        .copied()
        .context(MissingRelationshipTargetSnafu {
            issue: id.to_string(),
            field: "display",
            target: id,
        })?;
    let mut children = Vec::new();
    if let Some(child_ids) = children_by_parent.get(&id) {
        for child_id in child_ids {
            let child = build_issue_summary_node(*child_id, by_id, children_by_parent, path)?;
            children.push(child);
        }
    }

    path.pop();
    Ok(DisplayIssueNode { issue, children })
}

fn print_issue_summary_node(
    palette: &HumanPalette,
    issues: &[Issue],
    node: &DisplayIssueNode<'_>,
    depth: usize,
) {
    let line = issue_summary_line(palette, issues, node.issue);
    if depth == 0 {
        println!("{line}");
    } else {
        let indent = "  ".repeat(depth);
        println!("{indent}↳ {line}");
    }

    for child in &node.children {
        print_issue_summary_node(palette, issues, child, depth + 1);
    }
}

fn summary_badge(palette: &HumanPalette, issue: &Issue, blocked: bool) -> String {
    let mut parts = vec![
        palette.paint(priority_role(issue), format!("P{}", issue.priority)),
        palette.paint(status_role(issue, blocked), status_label(issue, blocked)),
        palette.paint(HumanRole::IssueKind, issue.kind.to_string()),
    ];
    if let Some(assignee) = &issue.assignee {
        parts.push(palette.paint(HumanRole::Metadata, assignee));
    }
    if !issue.labels.is_empty() {
        parts.push(label_summary(
            palette,
            issue.labels.iter().map(String::as_str),
        ));
    }
    parts.join(" · ")
}

fn detail_badge(palette: &HumanPalette, issue: &Issue, blocked: bool) -> String {
    let priority = palette.paint(priority_role(issue), format!("P{}", issue.priority));
    let status = palette.paint(status_role(issue, blocked), status_label(issue, blocked));
    format!("{priority} · {status}")
}

fn metadata_line(
    palette: &HumanPalette,
    issues: &[Issue],
    issue: &Issue,
    ready: bool,
    blocked: bool,
) -> String {
    let owner = issue.assignee.as_deref().map_or_else(
        || palette.paint(HumanRole::Muted, "<unclaimed>"),
        |assignee| palette.paint(HumanRole::IssueRef, assignee),
    );
    let availability = palette.paint(
        availability_role(issue, ready),
        availability_label(issue, ready),
    );
    let mut parts = vec![
        format!("Owner: {owner}"),
        format!("Availability: {availability}"),
    ];
    if let Some(parent_id) = issue.parent {
        parts.insert(
            1,
            format!("Parent: {}", issue_ref_or_uuid(palette, issues, parent_id)),
        );
    }
    if let Some(status_reason) = &issue.status_reason {
        let status_reason = palette.paint(status_role(issue, blocked), status_reason);
        parts.insert(
            1,
            format!("{}: {status_reason}", status_reason_label(issue)),
        );
    }
    if !issue.labels.is_empty() {
        parts.push(format!(
            "Labels: {}",
            label_summary(palette, issue.labels.iter().map(String::as_str))
        ));
    }
    parts.join(" · ")
}

fn issue_ref_or_uuid(palette: &HumanPalette, issues: &[Issue], id: Uuid) -> String {
    issues.iter().find(|issue| issue.id == id).map_or_else(
        || palette.paint(HumanRole::Blocked, id.to_string()),
        |issue| palette.paint(HumanRole::IssueRef, issue.reference.to_string()),
    )
}

fn label_summary<'labels>(
    palette: &HumanPalette,
    labels: impl Iterator<Item = &'labels str>,
) -> String {
    labels
        .map(|label| palette.paint(HumanRole::Label, label))
        .collect::<Vec<_>>()
        .join(",")
}

fn timestamp_line(issue: &Issue) -> String {
    let mut parts = vec![
        format!("Created: {}", date_part(&issue.created_at)),
        format!("Updated: {}", date_part(&issue.updated_at)),
    ];
    if let Some(closed_at) = &issue.closed_at {
        parts.push(format!("Closed: {}", date_part(closed_at)));
    }
    parts.join(" · ")
}

fn uuid_line(issue: &Issue) -> String {
    format!("UUID: {}", issue.id)
}

fn status_reason_label(issue: &Issue) -> &'static str {
    if issue.status.is_resolved() {
        "Resolution"
    } else {
        "Phase"
    }
}

fn availability_label(issue: &Issue, ready: bool) -> &'static str {
    match issue.status {
        IssueStatus::Closed => "closed",
        IssueStatus::InProgress => "claimed",
        IssueStatus::Open if ready => "ready",
        IssueStatus::Open if issue.assignee.is_some() => "claimed",
        IssueStatus::Open => "blocked",
    }
}

fn availability_role(issue: &Issue, ready: bool) -> HumanRole {
    match issue.status {
        IssueStatus::Closed => HumanRole::ClosedStatus,
        IssueStatus::InProgress => HumanRole::InProgressStatus,
        IssueStatus::Open if ready => HumanRole::OpenStatus,
        IssueStatus::Open if issue.assignee.is_some() => HumanRole::InProgressStatus,
        IssueStatus::Open => HumanRole::Blocked,
    }
}

fn date_part(timestamp: &str) -> &str {
    timestamp
        .split_once('T')
        .map_or(timestamp, |(date, _time)| date)
}

fn status_label(issue: &Issue, blocked: bool) -> String {
    // In-progress work stays visually in-progress even if new blockers are
    // discovered during implementation. Blocked markers are reserved for open
    // work that should not be picked up yet.
    if blocked && issue.status == IssueStatus::Open {
        "BLOCKED".to_string()
    } else {
        issue.status.as_str().to_uppercase()
    }
}

fn status_marker(issue: &Issue, blocked: bool) -> &'static str {
    if blocked && issue.status == IssueStatus::Open {
        "!"
    } else {
        match issue.status {
            IssueStatus::Open => "□",
            IssueStatus::InProgress => "◆",
            IssueStatus::Closed => "✓",
        }
    }
}

fn status_role(issue: &Issue, blocked: bool) -> HumanRole {
    if blocked && issue.status == IssueStatus::Open {
        HumanRole::Blocked
    } else {
        match issue.status {
            IssueStatus::Open => HumanRole::OpenStatus,
            IssueStatus::InProgress => HumanRole::InProgressStatus,
            IssueStatus::Closed => HumanRole::ClosedStatus,
        }
    }
}

fn is_blocked_by_unresolved_issue(issues: &[Issue], issue: &Issue) -> bool {
    issue.blocked_by.iter().any(|id| {
        issues
            .iter()
            .find(|candidate| candidate.id == *id)
            .is_some_and(|blocker| !blocker.status.is_resolved())
    })
}

fn blocking_summary(issues: &[Issue], issue: &Issue) -> Option<String> {
    let blockers = issue
        .blocked_by
        .iter()
        .filter_map(|id| {
            issues
                .iter()
                .find(|candidate| candidate.id == *id)
                .filter(|blocker| !blocker.status.is_resolved())
                .map(|blocker| blocker.reference.to_string())
        })
        .collect::<Vec<_>>();

    if blockers.is_empty() {
        None
    } else {
        Some(format!("blocked by {}", blockers.join(", ")))
    }
}

fn dependency_summary(palette: &HumanPalette, issues: &[Issue], id: Uuid) -> String {
    if let Some(issue) = issues.iter().find(|candidate| candidate.id == id) {
        let blocked = is_blocked_by_unresolved_issue(issues, issue);
        let marker = palette.paint(status_role(issue, blocked), status_marker(issue, blocked));
        let reference = palette.paint(HumanRole::IssueRef, issue.reference.to_string());
        let title = palette.paint(HumanRole::Title, &issue.title);
        let badge = detail_badge(palette, issue, blocked);
        format!("{marker} {reference}: {title} [{badge}]")
    } else {
        let missing = palette.paint(HumanRole::Blocked, "!");
        format!("{missing} {id}: missing issue")
    }
}

fn priority_role(issue: &Issue) -> HumanRole {
    if issue.priority == 0 {
        HumanRole::HighPriority
    } else {
        HumanRole::Metadata
    }
}

/// Semantic role for human output styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HumanRole {
    IssueRef,
    Title,
    SectionHeading,
    Metadata,
    Muted,
    IssueKind,
    Label,
    HighPriority,
    OpenStatus,
    InProgressStatus,
    ClosedStatus,
    Blocked,
    Divider,
}

impl HumanRole {
    fn indicator(self) -> Indicator {
        match self {
            Self::IssueRef => Indicator::SymbolicLink,
            Self::Title | Self::SectionHeading => Indicator::Directory,
            Self::Metadata | Self::Muted | Self::OpenStatus | Self::Divider => {
                Indicator::RegularFile
            }
            Self::IssueKind => Indicator::Socket,
            Self::Label => Indicator::FIFO,
            Self::HighPriority => Indicator::Setuid,
            Self::InProgressStatus => Indicator::ExecutableFile,
            Self::ClosedStatus => Indicator::MissingFile,
            Self::Blocked => Indicator::OrphanedSymbolicLink,
        }
    }

    fn adjust_style(self, mut style: Style) -> Style {
        if self == Self::Muted {
            style.font_style.dimmed = true;
        }
        style
    }

    fn fallback_style(self) -> Option<Style> {
        (self == Self::Muted).then(|| Style {
            font_style: FontStyle::dimmed(),
            ..Style::default()
        })
    }
}

#[derive(Debug, Serialize)]
struct IssueView {
    id: Uuid,
    #[serde(rename = "ref")]
    reference: IssueRef,
    title: String,
    body: String,
    status: IssueStatus,
    status_reason: Option<String>,
    #[serde(rename = "type")]
    kind: IssueKind,
    priority: u8,
    labels: Vec<String>,
    assignee: Option<String>,
    blocked_by: Vec<DependencyView>,
    blocks: Vec<DependencyView>,
    parent: Option<DependencyView>,
    children: Vec<DependencyView>,
    links: Vec<LinkView>,
    ready: bool,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    comments: Vec<Comment>,
}

impl IssueView {
    fn from_issue(_config: &Config, issues: &[Issue], issue: &Issue) -> Result<Self> {
        let by_id = issue_map(issues);
        Ok(Self {
            id: issue.id,
            reference: issue.reference.clone(),
            title: issue.title.clone(),
            body: issue.body.clone(),
            status: issue.status,
            status_reason: issue.status_reason.clone(),
            kind: issue.kind.clone(),
            priority: issue.priority,
            labels: issue.labels.clone(),
            assignee: issue.assignee.clone(),
            blocked_by: issue
                .blocked_by
                .iter()
                .map(|id| DependencyView::from_id(issues, *id))
                .collect(),
            blocks: issue
                .blocks
                .iter()
                .map(|id| DependencyView::from_id(issues, *id))
                .collect(),
            parent: issue
                .parent
                .map(|parent_id| DependencyView::from_id(issues, parent_id)),
            children: issue
                .children
                .iter()
                .map(|id| DependencyView::from_id(issues, *id))
                .collect(),
            links: issue
                .links
                .iter()
                .map(|link| LinkView::from_link(issues, link))
                .collect(),
            ready: issue_is_ready(issue, &by_id)?,
            created_at: issue.created_at.clone(),
            updated_at: issue.updated_at.clone(),
            closed_at: issue.closed_at.clone(),
            comments: issue.comments.clone(),
        })
    }
}

#[derive(Debug, Serialize)]
struct LinkView {
    target: DependencyView,
    label: String,
}

impl LinkView {
    fn from_link(issues: &[Issue], link: &IssueLink) -> Self {
        Self {
            target: DependencyView::from_id(issues, link.target),
            label: link.label.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct DependencyView {
    id: Uuid,
    #[serde(rename = "ref")]
    reference: Option<IssueRef>,
    status: Option<IssueStatus>,
    status_reason: Option<String>,
    closed_at: Option<String>,
}

impl DependencyView {
    fn from_id(issues: &[Issue], id: Uuid) -> Self {
        let issue = issues.iter().find(|issue| issue.id == id);
        Self {
            id,
            reference: issue.map(|issue| issue.reference.clone()),
            status: issue.map(|issue| issue.status),
            status_reason: issue.and_then(|issue| issue.status_reason.clone()),
            closed_at: issue.and_then(|issue| issue.closed_at.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::Error, model::NewIssue};

    #[test]
    fn human_roles_map_to_lscolors_indicators() {
        assert_eq!(HumanRole::IssueRef.indicator(), Indicator::SymbolicLink);
        assert_eq!(HumanRole::Title.indicator(), Indicator::Directory);
        assert_eq!(HumanRole::SectionHeading.indicator(), Indicator::Directory);
        assert_eq!(HumanRole::Metadata.indicator(), Indicator::RegularFile);
        assert_eq!(HumanRole::Muted.indicator(), Indicator::RegularFile);
        assert_eq!(HumanRole::IssueKind.indicator(), Indicator::Socket);
        assert_eq!(HumanRole::Label.indicator(), Indicator::FIFO);
        assert_eq!(HumanRole::HighPriority.indicator(), Indicator::Setuid);
        assert_eq!(HumanRole::OpenStatus.indicator(), Indicator::RegularFile);
        assert_eq!(
            HumanRole::InProgressStatus.indicator(),
            Indicator::ExecutableFile
        );
        assert_eq!(HumanRole::ClosedStatus.indicator(), Indicator::MissingFile);
        assert_eq!(
            HumanRole::Blocked.indicator(),
            Indicator::OrphanedSymbolicLink
        );
        assert_eq!(HumanRole::Divider.indicator(), Indicator::RegularFile);
    }

    #[test]
    fn in_progress_issue_keeps_in_progress_marker_when_blocked() {
        let issue = test_issue("gitrack-work", IssueStatus::InProgress, 3);

        assert_eq!(status_label(&issue, true), "IN-PROGRESS");
        assert_eq!(status_marker(&issue, true), "◆");
        assert_eq!(status_role(&issue, true), HumanRole::InProgressStatus);
    }

    #[test]
    fn priority_role_highlights_only_p0() {
        let p0_issue = test_issue("gitrack-p0", IssueStatus::Open, 0);
        let p1_issue = test_issue("gitrack-p1", IssueStatus::Open, 1);

        assert_eq!(priority_role(&p0_issue), HumanRole::HighPriority);
        assert_eq!(priority_role(&p1_issue), HumanRole::Metadata);
    }

    #[test]
    fn show_header_values_use_semantic_roles() {
        let palette = test_palette("ln=32:ex=33:or=35:fi=37");
        let parent = test_issue("gitrack-parent", IssueStatus::InProgress, 1);
        let mut issue = test_issue("gitrack-work", IssueStatus::InProgress, 3);
        issue.assignee = Some("codex".to_string());
        issue.parent = Some(parent.id);
        issue.status_reason = Some("in review".to_string());
        let issues = vec![parent, issue.clone()];

        let line = metadata_line(&palette, &issues, &issue, false, false);

        assert!(line.contains(&format!(
            "Owner: {}",
            palette.paint(HumanRole::IssueRef, "codex")
        )));
        assert!(line.contains(&format!(
            "Parent: {}",
            palette.paint(HumanRole::IssueRef, "gitrack-parent")
        )));
        assert!(line.contains(&format!(
            "Phase: {}",
            palette.paint(HumanRole::InProgressStatus, "in review")
        )));
        assert!(line.contains(&format!(
            "Availability: {}",
            palette.paint(HumanRole::InProgressStatus, "claimed")
        )));
    }

    #[test]
    fn availability_values_follow_status_roles() {
        let mut open_claimed = test_issue("gitrack-open-claimed", IssueStatus::Open, 3);
        open_claimed.assignee = Some("codex".to_string());
        let open_blocked = test_issue("gitrack-open-blocked", IssueStatus::Open, 3);
        let in_progress = test_issue("gitrack-progress", IssueStatus::InProgress, 3);
        let closed = test_issue("gitrack-closed", IssueStatus::Closed, 3);

        assert_eq!(
            availability_role(&open_blocked, true),
            HumanRole::OpenStatus
        );
        assert_eq!(
            availability_role(&open_claimed, false),
            HumanRole::InProgressStatus
        );
        assert_eq!(availability_role(&open_blocked, false), HumanRole::Blocked);
        assert_eq!(
            availability_role(&in_progress, false),
            HumanRole::InProgressStatus
        );
        assert_eq!(availability_role(&closed, false), HumanRole::ClosedStatus);
    }

    #[test]
    fn muted_role_dims_without_lscolors_entry() {
        let palette = HumanPalette {
            enabled: true,
            colours: None,
        };

        let muted = palette.paint(HumanRole::Muted, "UUID: 019efeae");

        assert!(muted.contains("\u{1b}["));
        assert!(muted.contains("UUID: 019efeae"));
        assert_eq!(
            palette.paint(HumanRole::Metadata, "UUID: 019efeae"),
            "UUID: 019efeae"
        );
    }

    #[test]
    fn summary_tree_with_ancestors_rejects_hierarchy_cycles() {
        let mut parent = test_issue("gitrack-parent", IssueStatus::Open, 1);
        let mut child = test_issue("gitrack-child", IssueStatus::Open, 1);
        parent.parent = Some(child.id);
        child.parent = Some(parent.id);
        let issues = vec![parent, child];

        let error =
            summary_tree_with_ancestors(&issues, &[&issues[0]]).expect_err("cycle rejected");

        assert!(matches!(error, Error::HierarchyCycle { .. }));
    }

    fn test_palette(lscolors: &str) -> HumanPalette {
        HumanPalette {
            enabled: true,
            colours: Some(LsColors::from_string(lscolors)),
        }
    }

    fn test_issue(reference: &str, status: IssueStatus, priority: u8) -> Issue {
        Issue::new(NewIssue {
            id: Uuid::now_v7(),
            reference: IssueRef::parse(reference).expect("valid ref"),
            title: format!("Issue {reference}"),
            body: String::new(),
            status,
            kind: IssueKind::parse("task").expect("valid kind"),
            priority,
            labels: Vec::new(),
            assignee: None,
            blocked_by: Vec::new(),
            now: "2026-06-25T10:00:00Z".to_string(),
        })
    }
}
