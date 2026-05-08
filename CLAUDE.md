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
