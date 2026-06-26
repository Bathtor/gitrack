//! End-to-end CLI workflow tests.

use std::{
    fs,
    path::{Path, PathBuf},
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
fn link_and_unlink_manage_hierarchy() {
    let (_temp, workdir) = initialised_workdir();
    let parent = run_json(
        &workdir,
        &["--json", "create", "Parent", "--ref", "project-parent"],
    );
    let child = run_json(
        &workdir,
        &["--json", "create", "Child", "--ref", "project-child"],
    );
    let parent_id = issue_id(&parent);
    let child_id = issue_id(&child);

    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-parent",
            "project-child",
            "--child",
        ],
    );
    assert_eq!(
        uuid_array_field(&issue_document(&workdir, &parent_id), "children"),
        vec![child_id.clone()]
    );
    assert_eq!(
        optional_uuid_field(&issue_document(&workdir, &child_id), "parent"),
        Some(parent_id.clone())
    );

    run_failure(
        &workdir,
        &["link", "project-child", "project-parent", "--child"],
    );
    run_success(&workdir, &["list"]);

    run_json(
        &workdir,
        &[
            "--json",
            "unlink",
            "project-parent",
            "project-child",
            "--child",
        ],
    );
    assert!(uuid_array_field(&issue_document(&workdir, &parent_id), "children").is_empty());
    assert_eq!(
        optional_uuid_field(&issue_document(&workdir, &child_id), "parent"),
        None
    );
}

#[test]
fn link_and_unlink_manage_blockers() {
    let (_temp, workdir) = initialised_workdir();
    let prerequisite = run_json(
        &workdir,
        &[
            "--json",
            "create",
            "Prerequisite",
            "--ref",
            "project-prereq",
        ],
    );
    let work_item = run_json(
        &workdir,
        &["--json", "create", "Work item", "--ref", "project-work"],
    );
    let prerequisite_id = issue_id(&prerequisite);
    let work_item_id = issue_id(&work_item);

    let linked = run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-work",
            "project-prereq",
            "--blocked-by",
        ],
    );
    assert_eq!(linked["blocked_by"][0]["id"], prerequisite_id);
    assert_eq!(
        uuid_array_field(&issue_document(&workdir, &prerequisite_id), "blocks"),
        vec![work_item_id]
    );

    let unlinked = run_json(
        &workdir,
        &[
            "--json",
            "unlink",
            "project-work",
            "project-prereq",
            "--blocked-by",
        ],
    );
    assert_eq!(
        unlinked["blocked_by"]
            .as_array()
            .expect("blocked_by array")
            .len(),
        0
    );
    assert!(uuid_array_field(&issue_document(&workdir, &prerequisite_id), "blocks").is_empty());

    run_failure(
        &workdir,
        &["block", "project-work", "--by", "project-prereq"],
    );
}

#[test]
fn link_and_unlink_manage_labelled_links() {
    let (_temp, workdir) = initialised_workdir();
    let source = run_json(
        &workdir,
        &["--json", "create", "Source", "--ref", "project-source"],
    );
    let target = run_json(
        &workdir,
        &["--json", "create", "Target", "--ref", "project-target"],
    );
    let source_id = issue_id(&source);
    let target_id = issue_id(&target);

    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-source",
            "project-target",
            "--label",
            "discovered from",
            "--bidirectional",
        ],
    );
    assert_eq!(
        link_entries(&issue_document(&workdir, &source_id)),
        vec![(target_id.clone(), "discovered from".to_string())]
    );
    assert_eq!(
        link_entries(&issue_document(&workdir, &target_id)),
        vec![(source_id.clone(), "discovered from".to_string())]
    );

    run_json(
        &workdir,
        &[
            "--json",
            "unlink",
            "project-source",
            "project-target",
            "--label",
            "discovered from",
            "--bidirectional",
        ],
    );
    assert!(link_entries(&issue_document(&workdir, &source_id)).is_empty());
    assert!(link_entries(&issue_document(&workdir, &target_id)).is_empty());

    run_json(
        &workdir,
        &["--json", "link", "project-source", "project-target"],
    );
    assert_eq!(
        link_entries(&issue_document(&workdir, &source_id)),
        vec![(target_id.clone(), "relates to".to_string())]
    );
}

#[test]
fn json_views_expose_relationships_without_changing_readiness() {
    let (_temp, workdir) = initialised_workdir();
    run_json(
        &workdir,
        &["--json", "create", "Parent", "--ref", "project-parent"],
    );
    run_json(
        &workdir,
        &["--json", "create", "Child", "--ref", "project-child"],
    );
    run_json(
        &workdir,
        &["--json", "create", "Source", "--ref", "project-source"],
    );
    run_json(
        &workdir,
        &["--json", "create", "Target", "--ref", "project-target"],
    );
    run_json(
        &workdir,
        &[
            "--json",
            "create",
            "Prerequisite",
            "--ref",
            "project-prereq",
        ],
    );
    run_json(
        &workdir,
        &["--json", "create", "Blocked", "--ref", "project-blocked"],
    );

    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-parent",
            "project-child",
            "--child",
        ],
    );
    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-source",
            "project-target",
            "--label",
            "relates to",
        ],
    );
    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-blocked",
            "project-prereq",
            "--blocked-by",
        ],
    );

    let parent = run_json(&workdir, &["--json", "show", "project-parent"]);
    assert_eq!(parent["children"][0]["ref"], "project-child");
    assert_eq!(parent["ready"], true);

    let child = run_json(&workdir, &["--json", "show", "project-child"]);
    assert_eq!(child["parent"]["ref"], "project-parent");
    assert_eq!(child["ready"], true);

    let source = run_json(&workdir, &["--json", "show", "project-source"]);
    assert_eq!(source["links"][0]["target"]["ref"], "project-target");
    assert_eq!(source["links"][0]["label"], "relates to");
    assert_eq!(source["ready"], true);

    let prerequisite = run_json(&workdir, &["--json", "show", "project-prereq"]);
    assert_eq!(prerequisite["blocks"][0]["ref"], "project-blocked");

    let ready = run_json(&workdir, &["--json", "ready"]);
    let ready_refs = issue_refs(&ready);
    assert!(ready_refs.contains(&"project-parent"));
    assert!(ready_refs.contains(&"project-child"));
    assert!(ready_refs.contains(&"project-source"));
    assert!(ready_refs.contains(&"project-target"));
    assert!(!ready_refs.contains(&"project-blocked"));

    let list = run_json(&workdir, &["--json", "list"]);
    assert_eq!(
        issue_by_ref(&list, "project-child")["parent"]["ref"],
        "project-parent"
    );

    let export = run_json(&workdir, &["export", "json"]);
    assert_eq!(
        issue_by_ref(&export, "project-source")["links"][0]["target"]["ref"],
        "project-target"
    );
}

#[test]
fn human_show_displays_relationship_sections() {
    let (_temp, workdir) = initialised_workdir();
    run_json(
        &workdir,
        &["--json", "create", "Parent", "--ref", "project-parent"],
    );
    run_json(
        &workdir,
        &["--json", "create", "Child", "--ref", "project-child"],
    );
    run_json(
        &workdir,
        &["--json", "create", "Prereq", "--ref", "project-prereq"],
    );
    run_json(
        &workdir,
        &["--json", "create", "Blocked", "--ref", "project-blocked"],
    );
    run_json(
        &workdir,
        &["--json", "create", "Related", "--ref", "project-related"],
    );

    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-parent",
            "project-child",
            "--child",
        ],
    );
    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-blocked",
            "project-prereq",
            "--blocked-by",
        ],
    );
    run_json(
        &workdir,
        &[
            "--json",
            "link",
            "project-parent",
            "project-related",
            "--label",
            "relates to",
        ],
    );

    let parent = run_success(&workdir, &["show", "project-parent"]);
    let parent_stdout = String::from_utf8_lossy(&parent.stdout);
    assert!(parent_stdout.contains("\nCHILDREN\n"));
    assert!(parent_stdout.contains("project-child: Child"));
    assert!(parent_stdout.contains("\nLINKS\n"));
    assert!(parent_stdout.contains("relates to:"));
    assert!(parent_stdout.contains("project-related: Related"));

    let child = run_success(&workdir, &["show", "project-child"]);
    let child_stdout = String::from_utf8_lossy(&child.stdout);
    assert!(child_stdout.contains("Parent: project-parent"));
    assert!(!child_stdout.contains("\nPARENT\n"));

    let prerequisite = run_success(&workdir, &["show", "project-prereq"]);
    let prerequisite_stdout = String::from_utf8_lossy(&prerequisite.stdout);
    assert!(prerequisite_stdout.contains("\nBLOCKS\n"));
    assert!(prerequisite_stdout.contains("project-blocked: Blocked"));

    let blocked = run_success(&workdir, &["show", "project-blocked"]);
    let blocked_stdout = String::from_utf8_lossy(&blocked.stdout);
    assert!(blocked_stdout.contains("\nBLOCKERS\n"));
    assert!(blocked_stdout.contains("project-prereq: Prereq"));
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
fn ref_command_repairs_real_merge_ref_clash_by_uuid() {
    let fixture = setup_real_merge_ref_clash();
    let workdir = &fixture.workdir;

    let dependent_path = issue_file_path(workdir, &fixture.dependent_id);
    let dependent_before = fs::read_to_string(&dependent_path).expect("read dependent issue");
    let renamed = run_json(
        workdir,
        &["--json", "ref", &fixture.right_id, "project-clash-right"],
    );
    assert_eq!(
        renamed["ref"].as_str().expect("renamed ref"),
        "project-clash-right"
    );
    let dependent_after = fs::read_to_string(&dependent_path).expect("read dependent issue");
    assert_eq!(dependent_after, dependent_before);

    assert_ref_alias_points_to(workdir, "project-clash", &fixture.left_id);
    assert_ref_alias_points_to(workdir, "project-clash-right", &fixture.right_id);

    let dependent = run_json(workdir, &["--json", "show", &fixture.dependent_id]);
    assert_eq!(dependent["blocked_by"][0]["id"], fixture.right_id);
    assert_eq!(
        dependent["blocked_by"][0]["ref"]
            .as_str()
            .expect("dependency ref"),
        "project-clash-right"
    );

    assert_merge_ref_clash_resolved(workdir);
}

#[test]
fn ref_command_repairs_ref_clash_when_renaming_current_alias_owner() {
    let fixture = setup_real_merge_ref_clash();
    let workdir = &fixture.workdir;

    let dependent_path = issue_file_path(workdir, &fixture.dependent_id);
    let dependent_before = fs::read_to_string(&dependent_path).expect("read dependent issue");
    let renamed = run_json(
        workdir,
        &["--json", "ref", &fixture.left_id, "project-clash-left"],
    );
    assert_eq!(
        renamed["ref"].as_str().expect("renamed ref"),
        "project-clash-left"
    );
    let dependent_after = fs::read_to_string(&dependent_path).expect("read dependent issue");
    assert_eq!(dependent_after, dependent_before);

    assert_ref_alias_points_to(workdir, "project-clash-left", &fixture.left_id);
    assert_ref_alias_points_to(workdir, "project-clash", &fixture.right_id);

    let dependent = run_json(workdir, &["--json", "show", &fixture.dependent_id]);
    assert_eq!(dependent["blocked_by"][0]["id"], fixture.right_id);
    assert_eq!(
        dependent["blocked_by"][0]["ref"]
            .as_str()
            .expect("dependency ref"),
        "project-clash"
    );

    assert_merge_ref_clash_resolved(workdir);
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
fn human_list_nests_descendants_under_parent_groups() {
    let (_temp, workdir) = initialised_workdir();
    create_issue_with_priority(&workdir, "Parent", "project-parent", 3);
    create_issue_with_priority(&workdir, "Child", "project-child", 2);
    create_issue_with_priority(&workdir, "Grandchild", "project-grandchild", 1);
    create_issue_with_priority(&workdir, "Unrelated", "project-unrelated", 0);
    link_child(&workdir, "project-parent", "project-child");
    link_child(&workdir, "project-child", "project-grandchild");

    let list = run_success(&workdir, &["list"]);
    let stdout = String::from_utf8_lossy(&list.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_line_prefixes(
        &lines,
        &[
            "□ project-unrelated",
            "□ project-parent",
            "  ↳ □ project-child",
            "    ↳ □ project-grandchild",
        ],
    );

    let json_list = run_json(&workdir, &["--json", "list"]);
    assert_refs(
        &json_list,
        &[
            "project-unrelated",
            "project-grandchild",
            "project-child",
            "project-parent",
        ],
    );
}

#[test]
fn human_ready_forces_ancestors_without_sibling_fanout() {
    let (_temp, workdir) = initialised_workdir();
    create_issue_with_priority(&workdir, "Parent", "project-parent", 3);
    create_issue_with_priority(&workdir, "Child", "project-child", 2);
    create_issue_with_priority(&workdir, "Grandchild", "project-grandchild", 1);
    create_issue_with_priority(&workdir, "Sibling", "project-sibling", 1);
    link_child(&workdir, "project-parent", "project-child");
    link_child(&workdir, "project-child", "project-grandchild");
    link_child(&workdir, "project-parent", "project-sibling");
    claim_issue(&workdir, "project-parent");
    claim_issue(&workdir, "project-child");
    claim_issue(&workdir, "project-sibling");

    let ready = run_success(&workdir, &["ready"]);
    let stdout = String::from_utf8_lossy(&ready.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_line_prefixes(
        &lines,
        &[
            "◆ project-parent",
            "  ↳ ◆ project-child",
            "    ↳ □ project-grandchild",
        ],
    );
    assert!(!stdout.contains("project-sibling"));

    let json_ready = run_json(&workdir, &["--json", "ready"]);
    assert_refs(&json_ready, &["project-grandchild"]);
}

#[test]
fn human_output_uses_compact_sections_and_plain_captured_text() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");

    run_success(&workdir, &["init", "--issue-dir", "issues", "--json"]);
    run_json(
        &workdir,
        &[
            "--json",
            "create",
            "Prepare API",
            "--ref",
            "project-api",
            "--priority",
            "1",
        ],
    );
    run_json(
        &workdir,
        &[
            "--json",
            "create",
            "Render issue view",
            "--ref",
            "project-render",
            "--priority",
            "3",
            "--label",
            "ui",
            "--body",
            "First line\nSecond line",
            "--blocked-by",
            "project-api",
        ],
    );
    run_success(
        &workdir,
        &[
            "comment",
            "project-render",
            "Investigated layout.\nSecond note line.",
            "--author",
            "codex",
        ],
    );

    let coloured_env = [("LS_COLORS", "di=31:ln=32:ex=33:or=35:su=41")];
    let list = run_success_with_env(&workdir, &["list"], &coloured_env);
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("□ project-api  Prepare API  [P1 · OPEN · task]"));
    assert!(list_stdout.contains(
        "! project-render  Render issue view  [P3 · BLOCKED · task · ui]  blocked by project-api"
    ));
    assert_no_ansi(&list_stdout);

    let ready = run_success_with_env(&workdir, &["ready"], &coloured_env);
    let ready_stdout = String::from_utf8_lossy(&ready.stdout);
    assert_eq!(
        ready_stdout.as_ref(),
        "□ project-api  Prepare API  [P1 · OPEN · task]\n"
    );

    let show = run_success_with_env(&workdir, &["show", "project-render"], &coloured_env);
    let show_stdout = String::from_utf8_lossy(&show.stdout);
    assert!(show_stdout.contains("! project-render [TASK] · Render issue view   [P3 · BLOCKED]"));
    assert!(show_stdout.contains("Owner: <unclaimed> · Availability: blocked · Labels: ui"));
    assert!(show_stdout.contains("\nCreated: "));
    assert!(show_stdout.contains("\nUUID: "));
    assert!(show_stdout.contains("\nDESCRIPTION\nFirst line\nSecond line\n"));
    assert!(show_stdout.contains("\nBLOCKERS\n  □ project-api: Prepare API [P1 · OPEN]\n"));
    assert!(show_stdout.contains("\nCOMMENTS\n"));
    assert!(show_stdout.contains("────────────────────────────────────────────────────────────"));
    assert!(show_stdout.contains("codex · "));
    assert!(show_stdout.contains("Investigated layout.\nSecond note line."));
    assert_no_ansi(&show_stdout);
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
    assert!(!root_stdout.contains("\n  block"));

    let claim_help = run_success(&workdir, &["claim", "--help"]);
    let claim_stdout = String::from_utf8_lossy(&claim_help.stdout);
    assert!(claim_stdout.contains("move it to in-progress"));
    assert!(claim_stdout.contains("Reopen it first"));

    let link_help = run_success(&workdir, &["link", "--help"]);
    let link_stdout = String::from_utf8_lossy(&link_help.stdout);
    assert!(link_stdout.contains("--child"));
    assert!(link_stdout.contains("--blocked-by"));
    assert!(link_stdout.contains("--bidirectional"));

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

fn create_issue_with_priority(workdir: &Path, title: &str, reference: &str, priority: u8) -> Value {
    run_json(
        workdir,
        &[
            "--json",
            "create",
            title,
            "--ref",
            reference,
            "--priority",
            &priority.to_string(),
        ],
    )
}

fn link_child(workdir: &Path, parent: &str, child: &str) -> Value {
    run_json(workdir, &["--json", "link", parent, child, "--child"])
}

fn claim_issue(workdir: &Path, reference: &str) -> Value {
    run_json(
        workdir,
        &["--json", "claim", reference, "--assignee", "agent"],
    )
}

fn initialised_workdir() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");
    fs::create_dir(workdir.join(".git")).expect("create fake git dir");
    run_success(&workdir, &["init", "--issue-dir", "issues", "--json"]);
    (temp, workdir)
}

fn run_success(workdir: &Path, args: &[&str]) -> Output {
    run_success_with_env(workdir, args, &[])
}

fn run_success_with_env(workdir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_gitrack"))
        .current_dir(workdir)
        .args(args)
        .envs(envs.iter().copied())
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

fn assert_no_ansi(output: &str) {
    assert!(
        !output.contains("\u{1b}["),
        "captured output must not contain ANSI escapes: {output:?}"
    );
}

fn assert_line_prefixes(lines: &[&str], expected_prefixes: &[&str]) {
    assert_eq!(lines.len(), expected_prefixes.len());
    for (index, (line, expected_prefix)) in lines.iter().zip(expected_prefixes).enumerate() {
        assert!(
            line.starts_with(expected_prefix),
            "line {index} should start with {expected_prefix:?}, got {line:?}"
        );
    }
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

/// Build a real Git merge state with two branches adding the same ref alias.
fn setup_real_merge_ref_clash() -> MergeRefClashFixture {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workdir = temp.path().join("project");
    fs::create_dir(&workdir).expect("create project dir");

    run_git_success(&workdir, &["init"]);
    run_git_success(&workdir, &["config", "user.name", "gitrack test"]);
    run_git_success(
        &workdir,
        &["config", "user.email", "gitrack@example.invalid"],
    );
    run_git_success(&workdir, &["config", "commit.gpgsign", "false"]);
    run_success(&workdir, &["init", "--issue-dir", "issues", "--no-agents"]);
    run_git_success(&workdir, &["add", "."]);
    run_git_success(&workdir, &["commit", "-m", "initialise gitrack"]);
    let base_branch = git_stdout(&workdir, &["branch", "--show-current"]);

    run_git_success(&workdir, &["checkout", "-b", "left"]);
    let left_issue = run_json(
        &workdir,
        &["--json", "create", "Left side", "--ref", "project-clash"],
    );
    let left_id = issue_id(&left_issue);
    run_git_success(&workdir, &["add", "."]);
    run_git_success(&workdir, &["commit", "-m", "add left issue"]);

    run_git_success(&workdir, &["checkout", &base_branch]);
    run_git_success(&workdir, &["checkout", "-b", "right"]);
    let right_issue = run_json(
        &workdir,
        &["--json", "create", "Right side", "--ref", "project-clash"],
    );
    let right_id = issue_id(&right_issue);
    let dependent_issue = run_json(
        &workdir,
        &[
            "--json",
            "create",
            "Right dependent",
            "--blocked-by",
            "project-clash",
        ],
    );
    let dependent_id = issue_id(&dependent_issue);
    run_git_success(&workdir, &["add", "."]);
    run_git_success(&workdir, &["commit", "-m", "add right issue"]);

    run_git_success(&workdir, &["checkout", "left"]);
    run_git_failure(&workdir, &["merge", "right"]);
    let conflicted_status = git_stdout(&workdir, &["status", "--short"]);
    assert!(conflicted_status.contains("AA issues/project-clash.toml"));
    assert_ref_alias_points_to(&workdir, "project-clash", &left_id);

    let failed_list = run_failure(&workdir, &["list"]);
    let failed_stderr = String::from_utf8_lossy(&failed_list.stderr);
    assert!(failed_stderr.contains("duplicate issue ref `project-clash`"));
    assert!(failed_stderr.contains("gitrack ref <uuid> <new-ref>"));

    MergeRefClashFixture {
        _temp: temp,
        workdir,
        left_id,
        right_id,
        dependent_id,
    }
}

/// Verify Git sees the ref alias conflict as resolved after CLI repair.
fn assert_merge_ref_clash_resolved(workdir: &Path) {
    run_git_success(workdir, &["add", "."]);
    let resolved_status = git_stdout(workdir, &["status", "--short"]);
    assert!(!resolved_status.contains("AA issues/project-clash.toml"));
    run_git_success(workdir, &["commit", "-m", "resolve ref clash"]);
}

fn run_git_success(workdir: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git failed\nargs: {:?}\nstatus: {}\nstdout: {}\nstderr: {}",
        args,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn run_git_failure(workdir: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        !output.status.success(),
        "git unexpectedly succeeded\nargs: {:?}\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_stdout(workdir: &Path, args: &[&str]) -> String {
    let output = run_git_success(workdir, args);
    String::from_utf8(output.stdout)
        .expect("git stdout UTF-8")
        .trim()
        .to_string()
}

fn issue_id(issue: &Value) -> String {
    issue["id"].as_str().expect("issue id").to_string()
}

fn issue_ref(issue: &Value) -> String {
    issue["ref"].as_str().expect("issue ref").to_string()
}

fn issue_refs(value: &Value) -> Vec<&str> {
    value["issues"]
        .as_array()
        .expect("issues array")
        .iter()
        .map(|issue| issue["ref"].as_str().expect("issue ref"))
        .collect()
}

fn issue_by_ref<'value>(value: &'value Value, reference: &str) -> &'value Value {
    value["issues"]
        .as_array()
        .expect("issues array")
        .iter()
        .find(|issue| issue["ref"].as_str() == Some(reference))
        .expect("issue with ref")
}

fn issue_file_path(workdir: &Path, issue_id: &str) -> std::path::PathBuf {
    workdir
        .join("issues")
        .join("issues-by-id")
        .join(format!("{issue_id}.toml"))
}

fn issue_document(workdir: &Path, issue_id: &str) -> toml::Value {
    let issue_path = issue_file_path(workdir, issue_id);
    let content = fs::read_to_string(issue_path).expect("read issue file");
    content.parse::<toml::Value>().expect("parse issue TOML")
}

fn uuid_array_field(document: &toml::Value, field: &str) -> Vec<String> {
    document
        .get(field)
        .and_then(toml::Value::as_array)
        .map_or_else(Vec::new, |values| {
            values
                .iter()
                .map(|value| value.as_str().expect("uuid string").to_string())
                .collect()
        })
}

fn optional_uuid_field(document: &toml::Value, field: &str) -> Option<String> {
    document
        .get(field)
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned)
}

fn link_entries(document: &toml::Value) -> Vec<(String, String)> {
    document
        .get("links")
        .and_then(toml::Value::as_array)
        .map_or_else(Vec::new, |links| {
            links
                .iter()
                .map(|link| {
                    let target = link["target"].as_str().expect("link target").to_string();
                    let label = link["label"].as_str().expect("link label").to_string();
                    (target, label)
                })
                .collect()
        })
}

fn assert_ref_alias_points_to(workdir: &Path, reference: &str, issue_id: &str) {
    let alias_path = workdir.join("issues").join(format!("{reference}.toml"));
    let target = fs::read_link(alias_path).expect("read ref alias");
    assert_eq!(
        target,
        Path::new("issues-by-id").join(format!("{issue_id}.toml"))
    );
}

fn set_updated_at(workdir: &Path, issue: &Value, updated_at: &str) {
    let issue_id = issue["id"].as_str().expect("issue id");
    let issue_path = issue_file_path(workdir, issue_id);
    let content = fs::read_to_string(&issue_path).expect("read issue file");
    let mut issue_document = content.parse::<toml::Value>().expect("parse issue TOML");
    issue_document["updated_at"] = toml::Value::String(updated_at.to_string());
    let serialised = toml::to_string_pretty(&issue_document).expect("serialise issue TOML");
    fs::write(&issue_path, serialised).expect("write issue file");
}

fn assert_refs(value: &Value, expected_refs: &[&str]) {
    let refs = issue_refs(value);
    assert_eq!(refs, expected_refs);
}

/// Real merge-clash fixture that keeps its temporary repository alive.
struct MergeRefClashFixture {
    /// Retained for the lifetime of `workdir`.
    _temp: tempfile::TempDir,
    workdir: PathBuf,
    left_id: String,
    right_id: String,
    dependent_id: String,
}
