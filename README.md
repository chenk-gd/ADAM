# ADAM — Agent-oriented Development Asset Management System

[![Rust](https://img.shields.io/badge/Rust-1.85+-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**ADAM** (Agent-oriented Development Asset Management) is a platform for managing R&D assets with dependency tracking and lifecycle management. It serves as a context service for Large Language Models (LLMs), providing structured access to requirements, design documents, code commits, test cases, and pipelines.

---

## Overview

ADAM provides a unified platform for:

- **Asset Type Management** — Define and manage various R&D asset types with JSON Schema metadata
- **Dependency Tracking** — Maintain DAG-based dependency relationships between assets
- **Lifecycle Management** — Track asset states (Clean, Dirty, Archived) with version control
- **LLM Context Service** — Provide structured asset context for AI agents via MCP protocol
- **CI/CD Integration** — Automatic asset registration through Git hooks and pipeline integration

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    ADAM Platform                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐   │
│  │Requirements│ │Work Items│ │ Design   │ │  Code    │   │
│  │          │  │          │  │ Documents│  │ Commits  │   │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘   │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                  │
│  │ Test     │  │Pipelines │  │ Custom   │                  │
│  │ Cases    │  │          │  │ Assets   │                  │
│  └──────────┘  └──────────┘  └──────────┘                  │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │         Dependency Engine & State Management         │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
                    ┌─────────┴─────────┐
                    ▼                   ▼
            ┌─────────────┐     ┌─────────────┐
            │  MCP Server  │     │  REST API   │
            │  Interface   │     │  Interface  │
            └─────────────┘     └─────────────┘
```

## Project Structure

This is a Rust workspace organized into multiple crates:

| Crate | Path | Description |
|-------|------|-------------|
| `adam-domain` | `crates/adam-domain` | Core domain models, entities, and business logic |
| `adam-application` | `crates/adam-application` | Application services, use cases, and state propagation |
| `adam-infrastructure` | `crates/adam-infrastructure` | Database repositories and external service adapters |
| `adam-adapters` | `crates/adam-adapters` | MCP and REST API interface implementations |
| `adam-server` | `crates/adam-server` | Application entry point and configuration |

## Core Concepts

### Asset Types
Pre-defined and extensible asset types:
- `requirement` — Feature and functional requirements
- `work_item` — Tasks and development work items
- `design_doc` — Architecture and detailed design documents
- `code_commit` — Code changes and pull requests
- `test_case` — Test scenarios and cases
- `pipeline` — CI/CD pipeline definitions

### Asset States
- **Clean** — Asset is up-to-date with upstream dependencies
- **Dirty** — Upstream dependency has newer version, awaiting review
- **Archived** — Asset is read-only, no longer maintained
- **Final** — Immutable asset (code_commit, pipeline_run), created in final state

### Asset Mutability
| Asset Type | State | Receives Dirty |
|------------|-------|----------------|
| requirement, design_doc, work_item, test_case | Clean/Dirty/Archived | Yes |
| code_commit, pipeline_run | **Final** | **No** |

## Quick Start

### Prerequisites

- Rust 1.85+ (2024 edition)
- PostgreSQL (for persistence)
- MCP-compatible client (optional, for AI integration)

### Build

```bash
# Debug build
cargo build

# Release build
cargo build --release
```

### Run Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture
```

### Run the Application

```bash
# Run the server
cargo run --bin adam-server

# With configuration
cargo run --bin adam-server -- --config config.toml
```

### Lint & Format

```bash
# Format code
cargo fmt

# Run linter
cargo clippy

# Check without building
cargo check
```

## MCP Server Integration

ADAM exposes an MCP (Model Context Protocol) server interface for AI agents:

- `query_assets` — Query assets by project, type, or state
- `create_virtual_asset` — Create temporary query contexts
- `publish_asset` — Publish new asset versions
- `get_asset_dependencies` — Retrieve dependency information

## Configuration

Example `config.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080

[database]
url = "postgres://user:pass@localhost/adam"

[mcp]
enabled = true
server_name = "adam-mcp-server"
```

## Documentation

- [Requirements Specification (Chinese)](docs/spec.md) — Complete functional requirements
- [MCP Interface Guide](docs/mcp-guide.md) — MCP server usage
- [API Reference](docs/api.md) — REST API documentation

## Contributing

Contributions are welcome! Please ensure:

1. Code follows Rust idioms and passes `cargo clippy`
2. New features include tests (80%+ coverage)
3. Documentation is updated for API changes

## License

MIT License — see [LICENSE](LICENSE) for details.

---

<p align="center">
  <a href="./README.zh.md">中文版</a>
</p>
