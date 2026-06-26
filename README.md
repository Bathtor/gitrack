# gitrack - Tracking issues alongside the code

gitrack is a small Git-native issue tracker CLI. Issue state lives in ordinary
tracked files in the current Git working tree, so normal commits, branches,
merges, pulls, and diffs explain and transport the tracker state. There are no
hidden refs, remotes, servers, databases, or sync commands.

The tool is designed to be predictable for coding agents: command names are
stable, JSON output is deterministic, workflows are non-interactive, and errors
are intended to be explicit.

## Contents

- [Install](#install)
- [Quick Start](#quick-start)
- [Storage Layout](#storage-layout)
- [Common Workflows](#common-workflows)
- [Agent Instructions](#agent-instructions)
- [Licence](#licence)

## Install

From a checkout of this repository:

```bash
./install.sh # Install the current checkout globally.
```

That installs the current checkout with `cargo install --path ... --locked --force`.

## Quick Start

Initialise tracking in a Git repository:

```bash
gitrack init # Initialise issue tracking in this repository.
```

Create and claim work:

```bash
gitrack create "Fix parser" --body "Handle escaped delimiters" # Create work.
gitrack ready # Show unclaimed, unblocked work.
gitrack claim gitrack-abc --assignee agent # Assign work to an agent.
```

Inspect, update, and close work:

```bash
gitrack show gitrack-abc # Inspect one issue.
gitrack update gitrack-abc --priority 2 --label parser # Update metadata.
gitrack close gitrack-abc --reason completed # Close resolved work.
```

Use `--json` on commands when another program or agent is consuming the output:

```bash
gitrack --json ready # Emit ready work as JSON.
gitrack --json show gitrack-abc # Emit one issue as JSON.
gitrack export json --pretty # Emit all issues as pretty JSON.
```

## Storage Layout

`gitrack init` creates:

```text
.gitrack/config.toml
issues/
```

The default issue directory is `issues`, but it can be changed during init:

```bash
gitrack init --issue-dir tasks # Store issue files under ./tasks.
```

Issue contents are stored by stable UUID:

```text
issues/issues-by-id/019f02e4-13f5-7dc1-b59a-c0ed1663cfee.toml
```

User-visible refs are symlinks at the top level of the issue directory:

```text
issues/gitrack-xny.toml -> issues-by-id/019f02e4-13f5-7dc1-b59a-c0ed1663cfee.toml
```

This makes refs easy to browse in file managers and on Git hosts, while
dependencies can still point at stable UUIDs internally. Use `gitrack ref` to
rename user-visible refs; dependency links use UUIDs internally, so ref renames
do not rewrite other issue files. If a merge produces a ref clash, use the UUID
form to rename one side, for example `gitrack ref <uuid> <new-ref>`, then stage
the resolved issue files with Git.

Issue files are readable TOML and include fields for UUID, ref, title, body,
status, status reason, type, priority, labels, assignee, blockers, timestamps,
and comments.

## Common Workflows

[Bootstrap](#bootstrap-tracking) | [Find work](#find-work) | [Create and organise](#create-and-organise-work) | [Claim and update](#claim-and-update-work) | [Manage blockers](#manage-blockers) | [Close or reopen](#close-or-reopen-work) | [Export](#export-for-agents-and-tools)

### Bootstrap Tracking

TL;DR:

```bash
gitrack init # Initialise issue tracking in the current Git repository.
```

To bootstrap tracking in a new repository, run `gitrack init` from the
repository root. This creates `.gitrack/config.toml`, creates the default issue
directory, and adds gitrack workflow instructions to the repository's
`AGENTS.md` file.

Pick one of these variants when the defaults are not right for the project:

```bash
gitrack init --issue-dir tasks # Store issue files under ./tasks.
gitrack init --no-agents # Skip creating or updating AGENTS.md.
```

### Find Work

TL;DR:

```bash
gitrack ready # Show open, unclaimed, unblocked work.
gitrack show gitrack-abc # Inspect one issue before claiming it.
```

Use `gitrack ready` when choosing what to work on next. It filters out claimed
work and work blocked by unresolved dependencies. Use `gitrack show` before
claiming when an agent or human needs the full body, metadata, blockers, and
comments.

Use `gitrack list` for broader scans:

```bash
gitrack list # List unresolved issues.
gitrack list --all # Include closed issues.
gitrack list --status closed # Show only closed issues.
```

#### Ready Work

`gitrack ready` lists work that is:

- `open`
- unclaimed
- not blocked by any issue that is still open or in progress

Claiming an issue assigns it and moves it to `in-progress`, so claimed work no
longer appears in `ready`.

### Create And Organise Work

TL;DR:

```bash
gitrack create "Fix parser" --body "Handle escaped delimiters" # Create a new issue.
gitrack create "Escaped delimiters" --ref gitrack-abc.1 # Create a child issue.
```

Create issues with a title and, when useful, a body describing the work.
Refs are generated automatically by default. Use `--ref` mainly for child issues
or other cases where a human-chosen ref is useful.

After creation, use `update`, `ref`, and `comment` to organise the issue without
rewriting its stable UUID:

```bash
gitrack update gitrack-abc --priority 2 --label parser # Change metadata.
gitrack ref gitrack-abc gitrack-parser # Rename the user-visible ref.
gitrack comment gitrack-abc "Captured the failing input." # Add a note.
```

### Claim And Update Work

TL;DR:

```bash
gitrack claim gitrack-abc --assignee agent # Assign and move to in-progress.
```

Use `gitrack claim` when taking ownership of an open issue. Claiming sets the
assignee and moves the issue to `in-progress`, which removes it from `ready`.

#### Statuses And Reasons

Statuses are fixed:

- `open`
- `in-progress`
- `closed`

Use `status_reason` for workflow detail that does not need another fixed status:

```bash
gitrack update gitrack-abc --status-reason "planning" # Record the workflow phase.
gitrack update gitrack-abc --status-reason "in review" # Mark work ready for review.
```

Common project-specific reasons include `planning`, `plan agreed`, `in review`,
`completed`, and `won't do`.

### Manage Blockers

TL;DR:

```bash
gitrack link gitrack-abc gitrack-def --blocked-by # Add gitrack-def as a blocker.
gitrack unlink gitrack-abc gitrack-def --blocked-by # Remove that blocker.
gitrack ready # Recompute ready work after dependency changes.
```

Use blockers when one issue cannot proceed until another issue is closed. The
`--blocked-by` selector is explicit about direction: the target issue blocks
the source issue.

```bash
gitrack link gitrack-abc gitrack-def --blocked-by # Make gitrack-def block gitrack-abc.
```

### Close Or Reopen Work

TL;DR:

```bash
gitrack close gitrack-abc --reason completed # Close resolved work.
```

Use `gitrack close` when work is resolved. The optional reason is stored as
`status_reason` and recorded as a comment, so the resolution remains visible in
normal Git diffs.

Use explicit reasons for non-completion outcomes, and reopen when more work is
needed:

```bash
gitrack close gitrack-abc --reason "won't do" # Close work without completing it.
gitrack reopen gitrack-abc # Move a closed issue back to open.
```

### Export For Agents And Tools

TL;DR:

```bash
gitrack --json ready # Deterministic JSON for ready work.
gitrack export json --pretty # Deterministic JSON for every issue.
```

Use `--json` on individual commands when another program or coding agent is
consuming the result. Use `gitrack export json` when a tool needs the full issue
set in one deterministic payload:

```bash
gitrack --json show gitrack-abc # Emit one issue as JSON.
gitrack export json # Emit every issue as compact JSON.
```

## Agent Instructions

`gitrack init` creates or updates a managed gitrack block in `AGENTS.md` unless
`--no-agents` is supplied.

To refresh those instructions later:

```bash
gitrack agents update # Refresh the managed gitrack block in AGENTS.md.
```

To append an editable suggested workflow section:

```bash
gitrack agents update --with-workflow # Append an editable workflow section.
```

That workflow text is intentionally not duplicated here; generated
instructions are the source to keep current.

## Licence

gitrack is distributed under the GNU General Public License version 2 or, at
your option, any later version. See [LICENSE](LICENSE).
