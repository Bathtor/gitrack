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

    let blocker = run_json(&workdir, &["--json", "create", "Set up storage"]);
    let blocker_ref = issue_ref(&blocker);
    assert!(blocker_ref.starts_with("project-"));

    let blocked = run_json(
        &workdir,
        &[
            "--json",
            "create",
            "Build command flow",
            "--blocked-by",
            &blocker_ref,
        ],
    );
    let blocked_ref = issue_ref(&blocked);
    assert!(blocked_ref.starts_with("project-"));
    assert_ne!(blocked_ref, blocker_ref);
    assert_eq!(
        blocked["blocked_by"][0]["ref"]
            .as_str()
            .expect("blocker ref"),
        blocker_ref
    );
    assert_eq!(blocked["ready"], false);

    let ready_before_close = run_json(&workdir, &["--json", "ready"]);
    assert_refs(&ready_before_close, &[blocker_ref.as_str()]);

    run_json(&workdir, &["--json", "close", &blocker_ref]);

    let ready_after_close = run_json(&workdir, &["--json", "ready"]);
    assert_refs(&ready_after_close, &[blocked_ref.as_str()]);

    let export = run_json(&workdir, &["export", "json"]);
    assert_refs(&export, &[blocker_ref.as_str(), blocked_ref.as_str()]);
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

fn assert_refs(value: &Value, expected_refs: &[&str]) {
    let refs = value["issues"]
        .as_array()
        .expect("issues array")
        .iter()
        .map(|issue| issue["ref"].as_str().expect("issue ref"))
        .collect::<Vec<_>>();
    assert_eq!(refs, expected_refs);
}
