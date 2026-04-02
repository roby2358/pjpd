# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

pjpd is an MCP (Model Context Protocol) server for local project management. It manages tasks and ideas for a single project rooted at the current working directory. All data lives under `<cwd>/pjpd/` as plain text files. It runs over stdio using the FastMCP framework.

## Commands

```bash
uv sync                              # Install dependencies
pytest -v                            # Run all tests
pytest -v tests/projects/test_update_task_tool.py  # Run one test file
pytest -v tests/projects/test_update_task_tool.py::TestUpdateTaskTool::test_update_task_description  # Run one test
pytest -v --cov=src                  # Run with coverage
black src/ tests/                    # Format
isort src/ tests/                    # Sort imports
mypy src/                            # Type check
python pjpd.py                       # Run the MCP server
```

## Architecture

**Entry point**: `pjpd.py` → adds `src/` to `sys.path` → calls `mcp_wrapper.main()`.

**`src/mcp_wrapper.py`** is the core file. It creates the FastMCP server, instantiates two managers (`Projects`, `Ideas`) both rooted at `Path.cwd()`, and registers all `@mcp.tool()` functions. Every tool returns `{"success": bool, "result": ..., "error": "..."}` via `mcp_success()`/`mcp_failure()` helpers. Every successful response includes a `project_file` property with the full path to the relevant data file. All tool inputs are validated through Pydantic models in `src/validation.py`.

**Single-project model**: The cwd is the project. There is one `pjpd/tasks.txt` file — no named projects, no project parameter on task tools.

**Two domain modules** follow the same pattern:
- `src/projects/` — `Projects` (manager) owns a `Project` instance via its `.project` property, which reloads from `pjpd/tasks.txt` on every access. `Project` handles loading/saving/filtering tasks. Each `Task` is a dataclass parsed from the file.
- `src/ideas/` — `Ideas` manages `Idea` records in `pjpd/ideas.txt`. Unlike Projects, Ideas handles loading/saving directly (no intermediate collection class).

**Shared infrastructure**:
- `src/textrec/text_records.py` — Parses `----`-separated records from `.txt` files (reads 3+ hyphens for backward compat) and provides `write_atomic()` (timestamped backup to `bak/` directory, then `os.replace`).
- `src/textrec/record_id.py` — Generates tag-based IDs in format `<tag>-<4char>` using a base32 alphabet (a-z, 2-9, excluding visually ambiguous chars).
- `src/config.py` — `Config` class loads `projectmcp.toml`. Instances are callable: `config(key, default)`. Currently only has `max_results`.

**Storage layout on disk**:
```
<cwd>/pjpd/
├── tasks.txt         # All tasks for the project
├── ideas.txt         # All ideas
└── bak/              # Timestamped backups from atomic writes
```

## Key Design Decisions

- **CWD is the project root** — no configurable projects directory, no multi-project support.
- **`Projects.project` property reloads from disk every access** — always reflects current file state. `Ideas.ideas` also reloads on every access.
- **IDs use format `<tag>-XXXX`** where XXXX is 4 base32 chars. Validated by regex `^[a-zA-Z0-9\-]+-[a-z2-9]{4}$`.
- **Priority is a plain integer** (higher = more important). Tasks default to priority 50.
- **Marking done**: Tasks get status "Done". Ideas get "(Done)" prepended to description (score is preserved).
- **Timestamps**: `Created` set once at creation, `Updated` refreshed on every write. Both ISO-8601 UTC. Legacy records without timestamps get both set on next update.
- **Sorting on save**: Tasks sort ToDo-first then priority descending. Ideas sort active-first then score descending.
- **Every tool response includes `project_file`** — the full path to the file being operated on.
- **Legacy migration warning** — task tool responses include a `warning` property if `pjpd/<dir_name>.txt` exists (from the old multi-project layout).

## Development Preferences

- No one-time test scripts — all tests go in `tests/`
- Favor functional constructions over imperative
- No globals — use class instance properties
- Keep metadata minimal
- Keep implementations simple; ask before extending functionality
- **Exception to "no defaults" rule**: Default parameter values are acceptable in MCP tool signatures (`@mcp.tool` functions in `mcp_wrapper.py`) so the calling model doesn't have to supply every argument. The no-defaults rule applies to internal function signatures, not the MCP API surface.
