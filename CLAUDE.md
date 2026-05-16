# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ADAM is a **Research and Development Asset Management System** (研发资产管理系统) written in Rust. It provides a platform for managing R&D assets (requirements, design documents, code commits, test cases, pipelines) with dependency tracking and lifecycle management.

### Core Purpose
- Asset type management with JSON Schema metadata
- Dependency relationship management (DAG-based)
- Asset lifecycle states: Clean, Dirty, Archived
- Version management with SemVer support
- Virtual asset instances for AI context queries
- MCP Server interface for AI Agent integration
- Git Hooks/CI integration for automatic asset registration

## Architecture Overview

See `docs/spec.md` for complete requirements specification (in Chinese).

### Key Domain Concepts

**Asset Types**: requirement, work_item, design_doc, code_commit, test_case, pipeline

**Asset Levels**:
- Project-level: Belongs to a specific project
- Organization-level: Shared across projects (e.g., coding standards)

**States**:
- `Clean`: Asset is up-to-date with upstream dependencies
- `Dirty`: Upstream dependency has newer version, awaiting review/update
- `Archived`: Asset no longer maintained, read-only

**Dependency Model**:
- Strict DAG (no cycles allowed)
- Declared version (snapshot at publish time)
- Effective version (current baseline for dirty checking)
- State propagation: Only publish triggers downstream dirty

### Data Model (Core Entities)

- `AssetType`: Type definitions with metadata schema
- `AssetInstance`: Actual asset with state, version, assignments
- `AssetDependency`: Instance-level dependency with declared/effective versions
- `AssetVersion`: Publish history with dependency snapshots
- `DirtyResolutionLog`: Manual clean review records
- `VirtualInstance`: Temporary query context for AI agents
- `PipelineRun`: CI/CD execution records (separate from asset versions)

## Common Commands

### Build
```bash
cargo build              # Debug build
cargo build --release    # Release build
```

### Test
```bash
cargo test               # Run all tests
cargo test <test_name>   # Run single test
cargo test -- --nocapture  # Show println! output
```

### Lint & Format
```bash
cargo fmt                # Format code
cargo clippy             # Run linter
cargo clippy -- -D warnings  # Fail on warnings
```

### Check
```bash
cargo check              # Fast syntax/type check (no codegen)
```

### Run
```bash
cargo run                # Run the application
cargo run -- <args>      # Run with arguments
```

### Dependencies
```bash
cargo add <crate>        # Add dependency
cargo update             # Update dependencies
cargo tree               # Show dependency tree
```

## Development Notes

- Project is in early stage; only specification exists in `docs/spec.md`
- External content storage: assets reference external systems (Git, Wiki, Jira) rather than storing content locally
- MCP Server interface defined for AI Agent integration (tools: query_assets, create_virtual_asset, publish_asset, etc.)
- Git Hooks integration planned for automatic asset registration on commit
- Query requires project_id; returns project-level assets + organization-level assets

## Specification Reference

Full requirements (Chinese): `docs/spec.md`
Key sections:
- Section 3: Functional requirements (FR-001 through FR-027)
- Section 5: Data model definitions
- Section 6: API endpoints
- Section 7: Business rules (DAG validation, state propagation, etc.)

## Agent skills

### Issue tracker
GitHub (chenk-gd/ADAM). See `docs/agents/issue-tracker.md`.

### Triage labels
Default vocabulary (needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix). See `docs/agents/triage-labels.md`.

### Domain docs
Single-context layout. See `docs/agents/domain.md`.

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **ADAM** (1128 symbols, 2403 relationships, 98 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## When Debugging

1. `gitnexus_query({query: "<error or symptom>"})` — find execution flows related to the issue
2. `gitnexus_context({name: "<suspect function>"})` — see all callers, callees, and process participation
3. `READ gitnexus://repo/ADAM/process/{processName}` — trace the full execution flow step by step
4. For regressions: `gitnexus_detect_changes({scope: "compare", base_ref: "main"})` — see what your branch changed

## When Refactoring

- **Renaming**: MUST use `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` first. Review the preview — graph edits are safe, text_search edits need manual review. Then run with `dry_run: false`.
- **Extracting/Splitting**: MUST run `gitnexus_context({name: "target"})` to see all incoming/outgoing refs, then `gitnexus_impact({target: "target", direction: "upstream"})` to find all external callers before moving code.
- After any refactor: run `gitnexus_detect_changes({scope: "all"})` to verify only expected files changed.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Tools Quick Reference

| Tool | When to use | Command |
|------|-------------|---------|
| `query` | Find code by concept | `gitnexus_query({query: "auth validation"})` |
| `context` | 360-degree view of one symbol | `gitnexus_context({name: "validateUser"})` |
| `impact` | Blast radius before editing | `gitnexus_impact({target: "X", direction: "upstream"})` |
| `detect_changes` | Pre-commit scope check | `gitnexus_detect_changes({scope: "staged"})` |
| `rename` | Safe multi-file rename | `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` |
| `cypher` | Custom graph queries | `gitnexus_cypher({query: "MATCH ..."})` |

## Impact Risk Levels

| Depth | Meaning | Action |
|-------|---------|--------|
| d=1 | WILL BREAK — direct callers/importers | MUST update these |
| d=2 | LIKELY AFFECTED — indirect deps | Should test |
| d=3 | MAY NEED TESTING — transitive | Test if critical path |

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/ADAM/context` | Codebase overview, check index freshness |
| `gitnexus://repo/ADAM/clusters` | All functional areas |
| `gitnexus://repo/ADAM/processes` | All execution flows |
| `gitnexus://repo/ADAM/process/{name}` | Step-by-step execution trace |

## Self-Check Before Finishing

Before completing any code modification task, verify:
1. `gitnexus_impact` was run for all modified symbols
2. No HIGH/CRITICAL risk warnings were ignored
3. `gitnexus_detect_changes()` confirms changes match expected scope
4. All d=1 (WILL BREAK) dependents were updated

## Keeping the Index Fresh

After committing code changes, the GitNexus index becomes stale. Re-run analyze to update it:

```bash
npx gitnexus analyze
```

If the index previously included embeddings, preserve them by adding `--embeddings`:

```bash
npx gitnexus analyze --embeddings
```

To check whether embeddings exist, inspect `.gitnexus/meta.json` — the `stats.embeddings` field shows the count (0 means no embeddings). **Running analyze without `--embeddings` will delete any previously generated embeddings.**

> Claude Code users: A PostToolUse hook handles this automatically after `git commit` and `git merge`.

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
