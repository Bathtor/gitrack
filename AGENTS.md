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
- Inside the sandbox, `cargo test` may run either one explicit test or a broader test selection only when passed `-- --test-threads=1`. Any grouped or repeated multi-threaded `cargo test` run must be executed outside the sandbox.
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
