# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

pjpd is an MCP (Model Context Protocol) server for local project management, written in Rust (crate in `rust/`, built on the official `rmcp` SDK). It manages tasks and ideas for a single project rooted at the current working directory. All data lives under `<cwd>/pjpd/` as plain text files. It runs over stdio.

The original Python implementation was ported to Rust in August 2026 and removed; it is archived in `pjpd-python-src.zip` and in git history. The Rust port was verified byte-identical on real data before the Python side was retired.

## Commands

```bash
cd rust
cargo test                 # Run all tests
cargo test --test tool_tests   # Run one test file
cargo build --release      # Build binary at rust/target/release/pjpd
cargo fmt                  # Format
```

Run the server by executing `rust/target/release/pjpd` with the project root as its working directory.

## Architecture

**Entry point**: `rust/src/main.rs` — serves `PjpdServer` over stdio; `Path::cwd()` is the project root.

**`rust/src/server.rs`** is the MCP surface: parameter structs (serde + schemars) and all tools. Every tool returns `{"success": bool, "result": ..., "error": "..."}` as JSON text content via `mcp_success()`/`mcp_failure()` helpers. Every successful response includes a `project_file` property with the full path to the relevant data file.

**Single-project model**: The cwd is the project. There is one `pjpd/tasks.txt` file — no named projects, no project parameter on task tools.

**Module map** (all under `rust/src/`):
- `tasks.rs` — `Task` records + `TaskStore` backed by `pjpd/tasks.txt` (ToDo) and `pjpd/tasks_done.txt` (Done archive). `TaskPatch` expresses partial updates.
- `ideas.rs` — `Idea` records + `IdeaStore` backed by `pjpd/ideas.txt`.
- `record.rs` — the shared record shape: `Key: value` property lines + description, ID resolution, timestamps.
- `textrec.rs` — `----`-separated records in .txt files (reads 3+ hyphens for backward compat) and atomic writes (timestamped backup to `bak/`, then rename).
- `ids.rs` — tag-based ID generation (`<tag>-XXXX`, base32 alphabet a-z 2-9 excluding 1/l/o) and format checks.
- `validation.rs` — request rules mirroring the original Pydantic models.

**Stores reload from disk on every operation** — state always reflects current file contents.

**Storage layout on disk**:
```
<cwd>/pjpd/
├── tasks.txt         # ToDo tasks for the project
├── tasks_done.txt    # Done tasks (archive; not backed up to bak/)
├── ideas.txt         # All ideas
└── bak/              # Timestamped backups from atomic writes
```

## Key Design Decisions

- **CWD is the project root** — no configurable projects directory, no multi-project support.
- **The on-disk text formats are the cross-language contract** — record separators (`----`), `Priority:`/`Score:` values padded to width 4, property lines in any order, ID pattern `^[a-zA-Z0-9\-]+-[a-z2-9]{4}$`, sort orders. Any future port must preserve them; the test suite pins them.
- **Priority is a plain integer** (higher = more important). New tasks default to priority 50 when none is given; updates without a priority preserve the existing one.
- **Marking done**: Tasks get status "Done" and move to tasks_done.txt. Ideas get a "(Done)" prefix; their score is preserved so it stays visible off the books.
- **Every tool response includes `project_file`** — the full path to the file being operated on.
- **Legacy migration warning** — task tool responses include a `warning` property if `pjpd/<dir_name>.txt` exists (from the old multi-project layout).
- **Forgiving parser** — malformed records are salvaged (missing IDs regenerated); statuses stay open strings, not enums, so unknown values survive a load/save round trip.

## Development Preferences

- No one-time test scripts — unit tests live in-module (`#[cfg(test)]`), integration tests in `rust/tests/`
- Favor functional constructions over imperative
- No globals — use struct instance properties
- Keep metadata minimal
- Keep implementations simple; ask before extending functionality
- **Exception to "no defaults" rule**: `#[serde(default)]` values are acceptable on MCP parameter structs in `server.rs` so the calling model doesn't have to supply every argument. The no-defaults rule applies to internal function signatures, not the MCP API surface.
