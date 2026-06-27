# Agent Instructions

- Do not plan "migrations" of any kind at this point. We are still building up the project. There are no existing deployments or database state or any such thing we need to handle.

## Local Agent Notes

- Use `.agent_local_docs/tmp/` for temporary agent working notes that should survive ordinary machine uptime. Do not put this kind of state in `/tmp` or `/private/tmp` unless the user explicitly asks for OS-level temporary storage.
- Review-feedback working notes are temporary coordination state, not project issue tracking. Keep them under `.agent_local_docs/tmp/`.
- When the user provides substantial review feedback, capture every comment in a working note before implementation. Include stable IDs, file context, the feedback, current status, and the intended direction.
- After plan alignment, update the working note with the agreed direction before editing production files.
- During and after implementation, use the working note as the checklist. Before final response, double-check every captured feedback item and report any item that remains unresolved or intentionally deferred.

## Rust Rules

- Do not run multiple `cargo` instances in parallel! They anyway lock.
- Format Rust code according to `rustfmt.toml`.
- Keep Rust changes clippy-clean where practical. Use `-W clippy::pedantic` before a change is considered ready for review.
- Prefer readable control flow over chained iterator side effects.
- Prefer `for` and `while`/`while let` over `loop` where possible; seeing the termination condition up front is usually clearer, even when the condition contains a fallible expression such as `while let Some(item) = next_item()?`.
- When splitting a single-file Rust module into a folder module, move the original module contents to `mod.rs` in the new folder.
- Avoid nesting `?` into expressions. It's easier to read if they only occur at the end of a line. Refactor the expression into a field where needed.
- Document non-public Rust helpers, fields, variants, and local types whenever their role, invariants, lifecycle, or preconditions are non-trivial or non-obvious. Prefer documenting what the item is supposed to do before adding code that explains how it does it.
- Document all modules, even if not public. But especially thoroughly if public.
- Add loop labels when control flow spans non-trivial nested loops or retries.
- Only use the early-return-pattern if it reduces branches that are over 5 lines long or 3 nesting levels deep.
- Prefer `async move {}.boxed()` over `Box::pin(async move {})` and similar for other future creating functions, such as `future::ready(_).boxed()` instead of `Box::pin(future::ready(_))`.
- Prefer the following top-level grouping within Rust files unless there is a strong local reason not to:
    1. public items (`pub`)
    2. restricted-visibility items (`pub(<qualifier>)`)
    3. macros
    4. private items
    5. exposed test helpers
    6. tests
- Within each group, use this order:
    1. constants
    2. traits/type aliases
    3. functions
    4. structs/enums, each followed immediately by all associated `impl` blocks
- Imports should remain at the very top of the file/module/function.

### Snafu

- Use Snafu-derived error types (`#[derive(Snafu)]`) for Rust error enums.
- Prefer `context(...)` / `with_context(...)` over manual `map_err(...)` when the target error still wraps the original source. Use `with_context(...)` when building the context captures clones, allocations, or other non-trivial work.
- If the only reason to introduce a new error variant is to differentiate the use-site of an existing variant, prefer adding `location: Location` to the existing variant instead.
- Use `#[snafu(module(...))]` plus module-qualified selector names when otherwise identical selector names would collide. Do not introduce custom selector aliases like `FooBarBazSnafu` just to disambiguate use sites.
- Keep Snafu variant names generic inside one error enum. Do not bake call-site names like `PublishStoreAccess` into the variant when the enum type or selector module already provides that context.
- Reserve manual `map_err(...)` for real error translation cases that `context(...)` cannot express cleanly.
- Do not manually construct Snafu boxed-source variants with `Box::new(source)`; use `result.boxed().context(SelectorSnafu)` or `context(...)`/`with_context(...)` instead.

<!-- BEGIN GITRACK MANAGED INSTRUCTIONS -->

## Issue Tracking with gitrack

This project uses [`gitrack`](https://github.com/Bathtor/gitrack) for Git-native issue tracking. Issue state lives in ordinary tracked files in this repository.

### Tool Rules

- Use `gitrack` for project issue tracking.
- Prefer `--json` for agent-driven workflows.
- Use `gitrack ready --json` to find unblocked open work.
- Check `stats.skipped` in `gitrack list --json` and `gitrack ready --json` output; rerun with `-n <COUNT>` when the default limit hides needed work.
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

<!-- END GITRACK MANAGED INSTRUCTIONS -->

## Suggested gitrack Workflow

### Priorities

- `0` - Immediate: drop everything and do this now.
- `1` - ASAP: finish the current task, then pick this up next before lower-priority work.
- `2` - High: important work.
- `3` - Normal: default priority for ordinary work.
- `4` - Low/Backlog: nice-to-have, polish, cleanup, or future ideas.

### Agent Workflow

#### Core Loop

1. Check ready work with `gitrack ready --json`, then inspect `stats` to see whether the result was limited.
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
