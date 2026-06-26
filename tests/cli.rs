//! End-to-end CLI workflow tests.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::Value;

#[test]
fn cli_tracks_blocked_and_ready_work() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");

    run_success(&workdir, &["init", "--issue-dir", "issues", "--json"]);

    let setup_issue = run_json(&workdir, &["--json", "create", "Set up storage"]);
    let setup_ref = issue_ref(&setup_issue);
    assert!(setup_ref.starts_with("project-"));

    let command_issue = run_json(
        &workdir,
        &[
            "--json",
            "create",
            "Build command flow",
            "--blocked-by",
            &setup_ref,
        ],
    );
    let command_ref = issue_ref(&command_issue);
    assert!(command_ref.starts_with("project-"));
    assert_ne!(command_ref, setup_ref);
    assert_eq!(
        command_issue["blocked_by"][0]["ref"]
            .as_str()
            .expect("blocker ref"),
        setup_ref
    );
    assert_eq!(command_issue["ready"], false);

    let ready_before_close = run_json(&workdir, &["--json", "ready"]);
    assert_refs(&ready_before_close, &[setup_ref.as_str()]);

    run_json(&workdir, &["--json", "close", &setup_ref]);

    let ready_after_close = run_json(&workdir, &["--json", "ready"]);
    assert_refs(&ready_after_close, &[command_ref.as_str()]);

    let export = run_json(&workdir, &["export", "json"]);
    assert_refs(&export, &[setup_ref.as_str(), command_ref.as_str()]);
}

#[test]
fn ref_command_generates_refs_and_accepts_explicit_child_refs() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");

    run_success(&workdir, &["init", "--issue-dir", "issues", "--json"]);

    let parent = run_json(&workdir, &["--json", "create", "Parent work"]);
    let parent_ref = issue_ref(&parent);
    let child = run_json(&workdir, &["--json", "create", "Child work"]);
    let child_ref = issue_ref(&child);

    let renamed_parent = run_json(&workdir, &["--json", "ref", &parent_ref]);
    let renamed_parent_ref = issue_ref(&renamed_parent);
    assert!(renamed_parent_ref.starts_with("project-"));
    assert_ne!(renamed_parent_ref, parent_ref);

    let explicit_child_ref = format!("{renamed_parent_ref}.1");
    let renamed_child = run_json(
        &workdir,
        &["--json", "ref", &child_ref, &explicit_child_ref],
    );
    assert_eq!(
        renamed_child["ref"].as_str().expect("child ref"),
        explicit_child_ref
    );

    run_failure(
        &workdir,
        &["--json", "ref", &renamed_parent_ref, &explicit_child_ref],
    );
}

#[test]
fn claim_sets_in_progress_and_rejects_closed_issues() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");

    run_success(&workdir, &["init", "--issue-dir", "issues", "--json"]);

    let created_issue = run_json(&workdir, &["--json", "create", "Implement workflow"]);
    let issue_ref = issue_ref(&created_issue);
    let claimed_issue = run_json(
        &workdir,
        &["--json", "claim", &issue_ref, "--assignee", "agent"],
    );
    assert_eq!(claimed_issue["status"], "in-progress");
    assert_eq!(claimed_issue["assignee"], "agent");

    let closed_issue = run_json(
        &workdir,
        &["--json", "close", &issue_ref, "--reason", "completed"],
    );
    assert_eq!(closed_issue["status"], "closed");
    assert_eq!(closed_issue["status_reason"], "completed");

    let failed_claim = run_failure(
        &workdir,
        &["--json", "claim", &issue_ref, "--assignee", "agent"],
    );
    let stderr = String::from_utf8_lossy(&failed_claim.stderr);
    assert!(stderr.contains("reopen it before claiming"));
}

#[test]
fn list_and_ready_sort_by_priority_then_recent_update() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");

    run_success(&workdir, &["init", "--issue-dir", "issues", "--json"]);

    let p2_old = run_json(&workdir, &["--json", "create", "P2 old", "--priority", "2"]);
    let p1_old = run_json(&workdir, &["--json", "create", "P1 old", "--priority", "1"]);
    let p1_recent = run_json(
        &workdir,
        &["--json", "create", "P1 recent", "--priority", "1"],
    );
    let p3_recent = run_json(
        &workdir,
        &["--json", "create", "P3 recent", "--priority", "3"],
    );

    set_updated_at(&workdir, &p2_old, "2026-06-26T10:00:00Z");
    set_updated_at(&workdir, &p1_old, "2026-06-26T11:00:00Z");
    set_updated_at(&workdir, &p1_recent, "2026-06-26T12:00:00Z");
    set_updated_at(&workdir, &p3_recent, "2026-06-26T13:00:00Z");

    let expected = [
        issue_ref(&p1_recent),
        issue_ref(&p1_old),
        issue_ref(&p2_old),
        issue_ref(&p3_recent),
    ];
    let expected_refs = expected.iter().map(String::as_str).collect::<Vec<_>>();

    let list = run_json(&workdir, &["--json", "list"]);
    assert_refs(&list, &expected_refs);

    let ready = run_json(&workdir, &["--json", "ready"]);
    assert_refs(&ready, &expected_refs);
}

#[test]
fn help_text_describes_common_agent_workflows() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");

    let root_help = run_success(&workdir, &["--help"]);
    let root_stdout = String::from_utf8_lossy(&root_help.stdout);
    assert!(root_stdout.contains("deterministic output suitable for coding agents"));
    assert!(root_stdout.contains("gitrack ready"));
    assert!(!root_stdout.contains("\n  help     "));

    let claim_help = run_success(&workdir, &["claim", "--help"]);
    let claim_stdout = String::from_utf8_lossy(&claim_help.stdout);
    assert!(claim_stdout.contains("move it to in-progress"));
    assert!(claim_stdout.contains("Reopen it first"));

    let agents_help = run_success(&workdir, &["agents", "--help"]);
    let agents_stdout = String::from_utf8_lossy(&agents_help.stdout);
    assert!(!agents_stdout.contains("\n  help    "));

    let nested_help = run_success(&workdir, &["agents", "update", "--help"]);
    let nested_stdout = String::from_utf8_lossy(&nested_help.stdout);
    assert!(nested_stdout.contains("--with-workflow"));

    let failed_help_subcommand = run_failure(&workdir, &["help"]);
    let failed_stderr = String::from_utf8_lossy(&failed_help_subcommand.stderr);
    assert!(failed_stderr.contains("unrecognized subcommand"));
}

#[test]
fn init_creates_agents_file_by_default_and_supports_opt_out() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let default_workdir = temp.path().join("default-project");
    fs::create_dir(&default_workdir).expect("create default project dir");
    fs::create_dir(default_workdir.join(".git")).expect("create fake git dir");

    let init = run_json(&default_workdir, &["init", "--json"]);
    let agents_path = default_workdir.join("AGENTS.md");
    let agents_content = fs::read_to_string(&agents_path).expect("read agents file");
    assert_eq!(init["agents"]["file"], "AGENTS.md");
    assert_eq!(init["agents"]["managed_section"], "created");
    assert_eq!(init["agents"]["workflow_section"], "skipped");
    assert!(agents_content.contains("BEGIN GITRACK MANAGED INSTRUCTIONS"));
    assert!(agents_content.contains("Use `gitrack` for project issue tracking."));

    let opt_out_workdir = temp.path().join("opt-out-project");
    fs::create_dir(&opt_out_workdir).expect("create opt-out project dir");
    fs::create_dir(opt_out_workdir.join(".git")).expect("create fake git dir");

    let init = run_json(&opt_out_workdir, &["init", "--json", "--no-agents"]);
    assert!(init["agents"].is_null());
    assert!(!opt_out_workdir.join("AGENTS.md").exists());
}

#[test]
fn agents_update_replaces_managed_block_and_appends_workflow() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");
    fs::write(
        workdir.join("AGENTS.md"),
        "# Agent Instructions\n\n<!-- BEGIN GITRACK MANAGED INSTRUCTIONS -->\nold\n<!-- END GITRACK MANAGED INSTRUCTIONS -->\n",
    )
    .expect("write agents file");

    let update = run_json(&workdir, &["--json", "agents", "update", "--with-workflow"]);
    let agents_content = fs::read_to_string(workdir.join("AGENTS.md")).expect("read agents file");

    assert_eq!(update["file"], "AGENTS.md");
    assert_eq!(update["managed_section"], "updated");
    assert_eq!(update["workflow_section"], "created");
    assert_eq!(update["changed"], true);
    assert!(agents_content.contains("Git-native issue tracking"));
    assert!(agents_content.contains("## Suggested gitrack Workflow"));
    assert!(!agents_content.contains("\nold\n"));
}

#[test]
fn agents_update_rejects_malformed_managed_markers() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");
    let original = "# Agent Instructions\n\n<!-- BEGIN GITRACK MANAGED INSTRUCTIONS -->\n";
    fs::write(workdir.join("AGENTS.md"), original).expect("write malformed agents file");

    let failed_update = run_failure(&workdir, &["agents", "update"]);
    let stderr = String::from_utf8_lossy(&failed_update.stderr);
    let agents_content = fs::read_to_string(workdir.join("AGENTS.md")).expect("read agents file");

    assert!(stderr.contains("matching begin and end markers"));
    assert_eq!(agents_content, original);
}

fn run_json(workdir: &Path, args: &[&str]) -> Value {
    let output = run_success(workdir, args);
    serde_json::from_slice(&output.stdout).expect("parse JSON output")
}

fn run_success(workdir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_gitrack"))
        .current_dir(workdir)
        .args(args)
        .output()
        .expect("run gitrack");
    assert!(
        output.status.success(),
        "gitrack failed\nstatus: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn run_failure(workdir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_gitrack"))
        .current_dir(workdir)
        .args(args)
        .output()
        .expect("run gitrack");
    assert!(
        !output.status.success(),
        "gitrack unexpectedly succeeded\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn issue_ref(issue: &Value) -> String {
    issue["ref"].as_str().expect("issue ref").to_string()
}

fn set_updated_at(workdir: &Path, issue: &Value, updated_at: &str) {
    let issue_id = issue["id"].as_str().expect("issue id");
    let issue_path = workdir
        .join("issues")
        .join("issues-by-id")
        .join(format!("{issue_id}.toml"));
    let content = fs::read_to_string(&issue_path).expect("read issue file");
    let mut issue_document = content.parse::<toml::Value>().expect("parse issue TOML");
    issue_document["updated_at"] = toml::Value::String(updated_at.to_string());
    let serialised = toml::to_string_pretty(&issue_document).expect("serialise issue TOML");
    fs::write(&issue_path, serialised).expect("write issue file");
}

fn assert_refs(value: &Value, expected_refs: &[&str]) {
    let refs = value["issues"]
        .as_array()
        .expect("issues array")
        .iter()
        .map(|issue| issue["ref"].as_str().expect("issue ref"))
        .collect::<Vec<_>>();
    assert_eq!(refs, expected_refs);
}
