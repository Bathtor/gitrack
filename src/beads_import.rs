//! Unsupported Beads JSONL importer.

use std::{
    collections::{HashMap, hash_map::Entry},
    fs,
    path::Path,
};

use serde::{Deserialize, Serialize};
use snafu::{ResultExt, ensure};
use uuid::Uuid;

use crate::{
    error::{ImportSnafu, ReadFileSnafu, Result},
    model::{Comment, Issue, IssueKind, IssueLink, IssueRef, IssueStatus, NewIssue},
    store::{Store, normalise_labels, normalise_optional},
    views::print_json,
};

const DISCOVERED_FROM_LABEL: &str = "discovered from";

/// Import a Beads JSONL export into the current empty gitrack store.
pub(crate) fn import_beads(path: &Path, json: bool) -> Result<()> {
    let store = Store::open(Path::new("."))?;
    let summary = import_beads_into_store(&store, path)?;
    if json {
        print_json(&summary, true)
    } else {
        println!(
            "Imported {} issues from {}",
            summary.imported,
            path.display()
        );
        Ok(())
    }
}

fn import_beads_into_store(store: &Store, path: &Path) -> Result<BeadsImportSummary> {
    let existing = store.load_issues()?;
    ensure!(
        existing.is_empty(),
        ImportSnafu {
            reason: "target gitrack store must be empty".to_string()
        }
    );

    let input = fs::read_to_string(path).context(ReadFileSnafu {
        path: path.to_path_buf(),
    })?;
    let records = parse_records(&input)?;
    let mut issues = build_issues(&records)?;
    apply_relationships(&records, &mut issues)?;
    for issue in &issues {
        store.save_issue(issue)?;
    }
    let loaded = store.load_issues()?;
    ensure!(
        loaded.len() == issues.len(),
        ImportSnafu {
            reason: format!(
                "loaded {} imported issues, expected {}",
                loaded.len(),
                issues.len()
            )
        }
    );

    Ok(BeadsImportSummary {
        imported: issues.len(),
    })
}

fn parse_records(input: &str) -> Result<Vec<BeadsIssue>> {
    let mut records = Vec::new();
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<BeadsIssue>(line).map_err(|source| {
            ImportSnafu {
                reason: format!(
                    "line {} is not a supported Beads issue: {source}",
                    index + 1
                ),
            }
            .build()
        })?;
        ensure!(
            record.record_type == "issue",
            ImportSnafu {
                reason: format!(
                    "line {} has unsupported record type `{}`",
                    index + 1,
                    record.record_type
                )
            }
        );
        records.push(record);
    }
    ensure!(
        !records.is_empty(),
        ImportSnafu {
            reason: "input did not contain any issue records".to_string()
        }
    );
    Ok(records)
}

fn build_issues(records: &[BeadsIssue]) -> Result<Vec<Issue>> {
    let mut ids_by_ref = HashMap::new();
    for record in records {
        match ids_by_ref.entry(record.id.clone()) {
            Entry::Occupied(_) => {
                return ImportSnafu {
                    reason: format!("duplicate Beads issue id `{}`", record.id),
                }
                .fail();
            }
            Entry::Vacant(entry) => {
                entry.insert(Uuid::now_v7());
            }
        }
    }

    let mut issues = Vec::with_capacity(records.len());
    for record in records {
        let id = *ids_by_ref
            .get(&record.id)
            .expect("id map must contain every parsed record");
        let status = parse_status(record.status.as_str())?;
        let mut issue = Issue::new(NewIssue {
            id,
            reference: IssueRef::parse(record.id.clone())?,
            title: record.title.clone(),
            body: body_from_record(record),
            status,
            kind: IssueKind::parse(record.issue_type.clone())?,
            priority: record.priority,
            labels: normalise_labels(record.labels.clone()),
            assignee: normalise_optional(record.assignee.clone()),
            blocked_by: Vec::new(),
            now: record.created_at.clone(),
        });
        issue.updated_at.clone_from(&record.updated_at);
        if status.is_resolved() {
            issue.closed_at.clone_from(&record.closed_at);
            issue.status_reason = normalise_optional(record.close_reason.clone());
        }
        issue.comments = comments_from_record(record)?;
        issues.push(issue);
    }
    Ok(issues)
}

fn apply_relationships(records: &[BeadsIssue], issues: &mut [Issue]) -> Result<()> {
    let ids_by_ref = issues
        .iter()
        .map(|issue| (issue.reference.as_str().to_string(), issue.id))
        .collect::<HashMap<_, _>>();
    let indexes_by_id = issues
        .iter()
        .enumerate()
        .map(|(index, issue)| (issue.id, index))
        .collect::<HashMap<_, _>>();

    for record in records {
        for dependency in &record.dependencies {
            ensure!(
                dependency.issue_id == record.id,
                ImportSnafu {
                    reason: format!(
                        "dependency on `{}` is attached to `{}`",
                        dependency.issue_id, record.id
                    )
                }
            );
            let source = lookup_ref(&ids_by_ref, dependency.issue_id.as_str())?;
            let target = lookup_ref(&ids_by_ref, dependency.depends_on_id.as_str())?;
            match dependency.kind.as_str() {
                "blocked-by" => add_blocking(issues, &indexes_by_id, source, target),
                "blocks" => add_blocking(issues, &indexes_by_id, target, source),
                "parent-child" => add_parent_child(issues, &indexes_by_id, target, source)?,
                "discovered-from" => add_link(issues, &indexes_by_id, source, target),
                other => {
                    return ImportSnafu {
                        reason: format!("unsupported dependency type `{other}`"),
                    }
                    .fail();
                }
            }
        }
    }

    for issue in issues {
        issue.blocked_by.sort();
        issue.blocked_by.dedup();
        issue.blocks.sort();
        issue.blocks.dedup();
        issue.children.sort();
        issue.children.dedup();
        issue.links.sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then_with(|| left.label.cmp(&right.label))
        });
        issue
            .links
            .dedup_by(|left, right| left.target == right.target && left.label == right.label);
    }
    Ok(())
}

fn lookup_ref(ids_by_ref: &HashMap<String, Uuid>, reference: &str) -> Result<Uuid> {
    ids_by_ref.get(reference).copied().ok_or_else(|| {
        ImportSnafu {
            reason: format!("dependency references missing issue `{reference}`"),
        }
        .build()
    })
}

fn add_blocking(
    issues: &mut [Issue],
    indexes_by_id: &HashMap<Uuid, usize>,
    blocked_issue: Uuid,
    blocking_issue: Uuid,
) {
    let blocked_issue_index = indexes_by_id[&blocked_issue];
    let blocking_issue_index = indexes_by_id[&blocking_issue];
    issues[blocked_issue_index].blocked_by.push(blocking_issue);
    issues[blocking_issue_index].blocks.push(blocked_issue);
}

fn add_parent_child(
    issues: &mut [Issue],
    indexes_by_id: &HashMap<Uuid, usize>,
    parent: Uuid,
    child: Uuid,
) -> Result<()> {
    let parent_index = indexes_by_id[&parent];
    let child_index = indexes_by_id[&child];
    ensure!(
        issues[child_index].parent.is_none() || issues[child_index].parent == Some(parent),
        ImportSnafu {
            reason: format!(
                "issue {} has multiple parents",
                issues[child_index].reference
            )
        }
    );
    issues[child_index].parent = Some(parent);
    issues[parent_index].children.push(child);
    Ok(())
}

fn add_link(
    issues: &mut [Issue],
    indexes_by_id: &HashMap<Uuid, usize>,
    source: Uuid,
    target: Uuid,
) {
    let source_index = indexes_by_id[&source];
    issues[source_index].links.push(IssueLink {
        target,
        label: DISCOVERED_FROM_LABEL.to_string(),
    });
}

fn parse_status(status: &str) -> Result<IssueStatus> {
    match status {
        "open" => Ok(IssueStatus::Open),
        "in_progress" => Ok(IssueStatus::InProgress),
        "closed" => Ok(IssueStatus::Closed),
        other => ImportSnafu {
            reason: format!("unsupported Beads status `{other}`"),
        }
        .fail(),
    }
}

fn body_from_record(record: &BeadsIssue) -> String {
    let mut body = record.description.clone();
    append_section(&mut body, "Design", record.design.as_deref());
    append_section(
        &mut body,
        "Acceptance Criteria",
        record.acceptance_criteria.as_deref(),
    );
    append_section(&mut body, "Notes", record.notes.as_deref());
    body
}

fn append_section(body: &mut String, title: &str, value: Option<&str>) {
    let Some(value) = value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }) else {
        return;
    };
    if !body.is_empty() {
        body.push_str("\n\n");
    }
    body.push_str("## ");
    body.push_str(title);
    body.push_str("\n\n");
    body.push_str(value);
}

fn comments_from_record(record: &BeadsIssue) -> Result<Vec<Comment>> {
    let mut comments = Vec::with_capacity(record.comments.len());
    for comment in &record.comments {
        let id = Uuid::parse_str(comment.id.as_str()).map_err(|source| {
            ImportSnafu {
                reason: format!("comment `{}` has invalid UUID: {source}", comment.id),
            }
            .build()
        })?;
        comments.push(Comment {
            id,
            author: comment.author.clone(),
            body: comment.text.clone(),
            created_at: comment.created_at.clone(),
        });
    }
    Ok(comments)
}

#[derive(Debug, Serialize)]
struct BeadsImportSummary {
    imported: usize,
}

#[derive(Debug, Deserialize)]
struct BeadsIssue {
    #[serde(rename = "_type")]
    record_type: String,
    id: String,
    title: String,
    description: String,
    #[serde(default)]
    design: Option<String>,
    #[serde(default)]
    acceptance_criteria: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    status: String,
    priority: u8,
    issue_type: String,
    #[serde(default)]
    assignee: Option<String>,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    closed_at: Option<String>,
    #[serde(default)]
    close_reason: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    dependencies: Vec<BeadsDependency>,
    #[serde(default)]
    comments: Vec<BeadsComment>,
}

#[derive(Debug, Deserialize)]
struct BeadsDependency {
    issue_id: String,
    depends_on_id: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct BeadsComment {
    id: String,
    author: String,
    text: String,
    created_at: String,
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn imports_representative_beads_export() {
        let (_temp, store, input) = import_store();
        fs::write(
            &input,
            r#"{"_type":"issue","id":"project-parent","title":"Parent","description":"Parent body","status":"open","priority":1,"issue_type":"epic","created_at":"2026-01-01T00:00:00Z","created_by":"Lars","updated_at":"2026-01-02T00:00:00Z","owner":"owner","dependency_count":0,"dependent_count":0,"comment_count":0}
{"_type":"issue","id":"project-child","title":"Child","description":"Child body","design":"Design text","acceptance_criteria":"Done means done","notes":"Carry this over","status":"in_progress","priority":2,"issue_type":"task","assignee":"Lars Kroll","created_at":"2026-01-03T00:00:00Z","created_by":"Lars","updated_at":"2026-01-04T00:00:00Z","owner":"owner","labels":["area:demo"],"dependencies":[{"issue_id":"project-child","depends_on_id":"project-parent","type":"parent-child","created_at":"2026-01-03T00:00:00Z","created_by":"Lars","metadata":"{}"},{"issue_id":"project-child","depends_on_id":"project-blocker","type":"blocked-by","created_at":"2026-01-03T00:00:00Z","created_by":"Lars","metadata":"{}"},{"issue_id":"project-child","depends_on_id":"project-source","type":"discovered-from","created_at":"2026-01-03T00:00:00Z","created_by":"Lars","metadata":"{}"}],"dependency_count":2,"dependent_count":0,"comment_count":0}
{"_type":"issue","id":"project-blocker","title":"Blocker","description":"Blocker body","status":"closed","priority":0,"issue_type":"bug","created_at":"2026-01-05T00:00:00Z","created_by":"Lars","updated_at":"2026-01-06T00:00:00Z","closed_at":"2026-01-06T00:00:00Z","close_reason":"Completed","owner":"owner","dependencies":[{"issue_id":"project-blocker","depends_on_id":"project-source","type":"blocks","created_at":"2026-01-05T00:00:00Z","created_by":"Lars","metadata":"{}"}],"comments":[{"id":"019d6239-cd3c-7236-a264-dfb31f7cfed0","issue_id":"project-blocker","author":"Lars Kroll","text":"Comment body","created_at":"2026-01-06T00:00:00Z"}],"dependency_count":0,"dependent_count":1,"comment_count":1}
{"_type":"issue","id":"project-source","title":"Source","description":"Source body","status":"open","priority":3,"issue_type":"feature","created_at":"2026-01-07T00:00:00Z","created_by":"Lars","updated_at":"2026-01-08T00:00:00Z","owner":"owner","dependency_count":0,"dependent_count":1,"comment_count":0}
"#,
        )
        .expect("write beads input");

        let summary = import_beads_into_store(&store, &input).expect("import beads");
        assert_eq!(summary.imported, 4);

        let issues = store.load_issues().expect("load imported issues");
        let child = Store::resolve_issue(&issues, "project-child").expect("child");
        let parent = Store::resolve_issue(&issues, "project-parent").expect("parent");
        let blocker = Store::resolve_issue(&issues, "project-blocker").expect("blocker");
        let source = Store::resolve_issue(&issues, "project-source").expect("source");

        assert_eq!(child.parent, Some(parent.id));
        assert_eq!(parent.children, vec![child.id]);
        assert_eq!(child.blocked_by, vec![blocker.id]);
        assert!(blocker.blocks.contains(&child.id));
        assert!(source.blocked_by.contains(&blocker.id));
        assert_eq!(blocker.status_reason.as_deref(), Some("Completed"));
        assert_eq!(blocker.comments[0].body, "Comment body");
        assert!(child.body.contains("## Design"));
        assert_eq!(child.links[0].target, source.id);
        assert_eq!(child.links[0].label, DISCOVERED_FROM_LABEL);
    }

    fn import_store() -> (TempDir, Store, PathBuf) {
        let temp = tempfile::tempdir().expect("create tempdir");
        let workdir = temp.path().join("project");
        fs::create_dir(&workdir).expect("create workdir");
        fs::create_dir(workdir.join(".git")).expect("create git dir");
        let store = Store::init(
            &workdir,
            Some("project".to_string()),
            "issues".to_string(),
            Some("task".to_string()),
            Some(3),
        )
        .expect("init store");
        let input = workdir.join("issues.jsonl");
        (temp, store, input)
    }
}
