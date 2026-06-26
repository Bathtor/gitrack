//! AGENTS.md instruction generation for gitrack-aware repositories.

use std::{fmt, fs, path::Path};

use serde::Serialize;
use snafu::ResultExt;

use crate::error::{
    InvalidAgentsFileSnafu, ReadFileSnafu, ReadMetadataSnafu, Result, WriteFileSnafu,
};

pub(crate) fn update_agents_file(
    root: &Path,
    include_workflow: bool,
) -> Result<AgentsUpdateResult> {
    let path = root.join(AGENTS_FILE);
    let original = read_existing_agents_file(&path)?;
    let (with_managed_section, managed_section) = update_managed_section(&original, &path)?;
    let (content, workflow_section) =
        append_workflow_section(with_managed_section, include_workflow);

    if content != original {
        fs::write(&path, &content).context(WriteFileSnafu { path: path.clone() })?;
    }

    Ok(AgentsUpdateResult {
        file: AGENTS_FILE.to_string(),
        managed_section,
        workflow_section,
        changed: content != original,
    })
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AgentsUpdateResult {
    file: String,
    managed_section: AgentsSectionAction,
    workflow_section: AgentsSectionAction,
    changed: bool,
}

impl AgentsUpdateResult {
    pub(crate) fn print_human(&self) {
        let changed = if self.changed { "changed" } else { "unchanged" };
        println!(
            "{}: managed section {}, workflow section {}, {changed}",
            self.file, self.managed_section, self.workflow_section
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AgentsSectionAction {
    Created,
    Updated,
    Unchanged,
    Skipped,
}

impl fmt::Display for AgentsSectionAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
            Self::Skipped => "skipped",
        };
        formatter.write_str(name)
    }
}

const AGENTS_FILE: &str = "AGENTS.md";
const MANAGED_BEGIN_MARKER: &str = "<!-- BEGIN GITRACK MANAGED INSTRUCTIONS -->";
const MANAGED_END_MARKER: &str = "<!-- END GITRACK MANAGED INSTRUCTIONS -->";

const MANAGED_SECTION_BODY: &str = r"## Issue Tracking with gitrack

This project uses `gitrack` for Git-native issue tracking. Issue state lives in ordinary tracked files in this repository.

### Tool Rules

- Use `gitrack` for project issue tracking.
- Prefer `--json` for agent-driven workflows.
- Use `gitrack ready --json` to find unblocked open work.
- Use `gitrack show <ref> --json` before changing an issue.
- Use `gitrack claim <ref> --assignee <name> --json` before starting assigned work.
- Use `gitrack update <ref> --body <text> --json` to keep the current issue description and plan up to date.
- Use `gitrack link <parent> <child> --child --json` when splitting work into child issues.
- Use `gitrack link <issue> <blocker> --blocked-by --json` when one issue must wait for another.
- Use labelled `gitrack link <source> <target> --label <label> --json` for loose one-way context.
- Use comments for chronological notes, review observations, and progress history.
- Close issues with `gitrack close <ref> --reason <reason> --json`.
- Do not create parallel TODO lists when the item should be tracked as an issue.

### Git Workflow Notes

- When creating a branch for a new task, create the branch first, then claim the issue so the claim is committed on that branch.
- Before committing completed work, update the issue state first so the issue change is included in the same commit.
";

const SUGGESTED_WORKFLOW_SECTION: &str = r#"## Suggested gitrack Workflow

### Priorities

- `0` - Immediate: drop everything and do this now.
- `1` - ASAP: finish the current task, then pick this up next before lower-priority work.
- `2` - High: important work.
- `3` - Normal: default priority for ordinary work.
- `4` - Low/Backlog: nice-to-have, polish, cleanup, or future ideas.

### Agent Workflow

#### Core Loop

1. Check ready work with `gitrack ready --json`.
2. Claim the selected issue with `gitrack claim <ref> --assignee <name> --json`.
3. Read the issue with `gitrack show <ref> --json`.
4. Set `status_reason = "planning"` while preparing the implementation plan.
5. Align on a concrete plan with the user before implementation.
6. Store the agreed plan in the issue body.
7. Once the user agrees, set `status_reason = "plan agreed"`.
8. Implement against the agreed plan.
9. Before handing work over for review, compare the result against the issue body and agreed plan.
10. Set `status_reason = "in review"` when ready for user review.

#### When a Branch Is Needed

Create the branch before claiming the issue so the claim is committed on that branch.

#### When Work Splits Into Children

Create child issues and link them with `gitrack link <parent> <child> --child --json`.

If the split issues have ordering constraints, link them with `gitrack link <issue> <blocker> --blocked-by --json`.

#### When New Work Is Discovered

Create the new issue, then link it back to the source issue with `gitrack link <new-ref> <source-ref> --label "discovered from" --json`.

#### Before Committing

Update issue state before committing so issue changes travel with the code or documentation changes they describe.

#### Closing Work

Only close the issue after the user agrees it is complete.

When closing, use `gitrack close <ref> --reason <reason> --json` with a concise reason such as `completed`, `won't do`, or `duplicate`.
"#;

fn read_existing_agents_file(path: &Path) -> Result<String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => InvalidAgentsFileSnafu {
            path: path.to_path_buf(),
            reason: "expected a file, found a directory".to_string(),
        }
        .fail(),
        Ok(_metadata) => fs::read_to_string(path).context(ReadFileSnafu {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(source).context(ReadMetadataSnafu {
            path: path.to_path_buf(),
        }),
    }
}

fn update_managed_section(original: &str, path: &Path) -> Result<(String, AgentsSectionAction)> {
    let managed_block = managed_block();
    let begin_matches = marker_offsets(original, MANAGED_BEGIN_MARKER);
    let end_matches = marker_offsets(original, MANAGED_END_MARKER);

    if begin_matches.len() > 1 || end_matches.len() > 1 {
        return InvalidAgentsFileSnafu {
            path: path.to_path_buf(),
            reason: "duplicate gitrack managed instruction markers".to_string(),
        }
        .fail();
    }
    if begin_matches.is_empty() && end_matches.is_empty() {
        return Ok((
            append_section(original, &managed_block),
            AgentsSectionAction::Created,
        ));
    }
    if begin_matches.len() != end_matches.len() {
        return InvalidAgentsFileSnafu {
            path: path.to_path_buf(),
            reason: "gitrack managed instruction markers must have matching begin and end markers"
                .to_string(),
        }
        .fail();
    }

    let begin = begin_matches[0];
    let end = end_matches[0];
    if begin > end {
        return InvalidAgentsFileSnafu {
            path: path.to_path_buf(),
            reason: "gitrack managed instruction end marker appears before begin marker"
                .to_string(),
        }
        .fail();
    }

    let replacement_end = end + MANAGED_END_MARKER.len();
    let mut content = String::new();
    content.push_str(&original[..begin]);
    content.push_str(&managed_block);
    content.push_str(&original[replacement_end..]);
    let action = if content == original {
        AgentsSectionAction::Unchanged
    } else {
        AgentsSectionAction::Updated
    };
    Ok((content, action))
}

fn append_workflow_section(
    content: String,
    include_workflow: bool,
) -> (String, AgentsSectionAction) {
    if !include_workflow {
        return (content, AgentsSectionAction::Skipped);
    }

    (
        append_section(&content, SUGGESTED_WORKFLOW_SECTION.trim_end()),
        AgentsSectionAction::Created,
    )
}

fn managed_block() -> String {
    format!(
        "{MANAGED_BEGIN_MARKER}\n\n{}\n\n{MANAGED_END_MARKER}",
        MANAGED_SECTION_BODY.trim_end()
    )
}

fn append_section(original: &str, section: &str) -> String {
    if original.trim().is_empty() {
        return format!("{section}\n");
    }

    let mut content = original.trim_end().to_string();
    content.push_str("\n\n");
    content.push_str(section);
    content.push('\n');
    content
}

fn marker_offsets(content: &str, marker: &str) -> Vec<usize> {
    content
        .match_indices(marker)
        .map(|(offset, _match)| offset)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn managed_section_is_created_for_missing_file() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let result = update_agents_file(temp.path(), false).expect("update agents file");
        let content = fs::read_to_string(temp.path().join(AGENTS_FILE)).expect("read agents file");

        assert_eq!(result.managed_section, AgentsSectionAction::Created);
        assert_eq!(result.workflow_section, AgentsSectionAction::Skipped);
        assert!(result.changed);
        assert!(content.contains(MANAGED_BEGIN_MARKER));
        assert!(content.contains("Use `gitrack` for project issue tracking."));
        assert!(!content.contains("Suggested gitrack Workflow"));
    }

    #[test]
    fn managed_section_is_updated_in_place() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let path = temp.path().join(AGENTS_FILE);
        fs::write(
            &path,
            format!("# Agent Instructions\n\n{MANAGED_BEGIN_MARKER}\nold\n{MANAGED_END_MARKER}\n"),
        )
        .expect("write agents file");

        let result = update_agents_file(temp.path(), false).expect("update agents file");
        let content = fs::read_to_string(path).expect("read agents file");

        assert_eq!(result.managed_section, AgentsSectionAction::Updated);
        assert!(content.starts_with("# Agent Instructions\n\n"));
        assert!(content.contains("Git-native issue tracking"));
        assert!(!content.contains("\nold\n"));
    }

    #[test]
    fn identical_managed_section_is_unchanged() {
        let temp = tempfile::tempdir().expect("create tempdir");
        update_agents_file(temp.path(), false).expect("create agents file");

        let result = update_agents_file(temp.path(), false).expect("update agents file");

        assert_eq!(result.managed_section, AgentsSectionAction::Unchanged);
        assert_eq!(result.workflow_section, AgentsSectionAction::Skipped);
        assert!(!result.changed);
    }

    #[test]
    fn workflow_section_is_appended_without_markers() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let result = update_agents_file(temp.path(), true).expect("update agents file");
        let content = fs::read_to_string(temp.path().join(AGENTS_FILE)).expect("read agents file");

        assert_eq!(result.workflow_section, AgentsSectionAction::Created);
        assert!(result.changed);
        assert!(content.contains("## Suggested gitrack Workflow"));
        assert!(content.contains("### Agent Workflow"));
        assert!(content.contains("#### Core Loop"));
        assert!(content.contains("#### When New Work Is Discovered"));
        assert!(
            content
                .contains("gitrack link <new-ref> <source-ref> --label \"discovered from\" --json")
        );
        assert!(!content.contains("BEGIN GITRACK SUGGESTED WORKFLOW"));
    }

    #[test]
    fn malformed_markers_are_rejected_without_editing() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let path = temp.path().join(AGENTS_FILE);
        let original = format!("# Agent Instructions\n\n{MANAGED_BEGIN_MARKER}\n");
        fs::write(&path, &original).expect("write agents file");

        let error = update_agents_file(temp.path(), false).expect_err("malformed markers");
        let after_error = fs::read_to_string(path).expect("read agents file");

        assert!(matches!(error, Error::InvalidAgentsFile { .. }));
        assert_eq!(after_error, original);
    }

    #[test]
    fn duplicate_markers_are_rejected_without_editing() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let path = temp.path().join(AGENTS_FILE);
        let original = format!(
            "{MANAGED_BEGIN_MARKER}\nold\n{MANAGED_END_MARKER}\n{MANAGED_BEGIN_MARKER}\nold\n{MANAGED_END_MARKER}\n"
        );
        fs::write(&path, &original).expect("write agents file");

        let error = update_agents_file(temp.path(), false).expect_err("duplicate markers");
        let after_error = fs::read_to_string(path).expect("read agents file");

        assert!(matches!(error, Error::InvalidAgentsFile { .. }));
        assert_eq!(after_error, original);
    }

    #[test]
    fn agents_path_must_not_be_a_directory() {
        let temp = tempfile::tempdir().expect("create tempdir");
        fs::create_dir(temp.path().join(AGENTS_FILE)).expect("create agents directory");

        let error = update_agents_file(temp.path(), false).expect_err("agents directory");

        assert!(matches!(error, Error::InvalidAgentsFile { .. }));
    }
}
