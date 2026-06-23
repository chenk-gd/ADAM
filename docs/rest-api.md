# ADAM REST API Documentation

## Overview

The ADAM REST API manages R&D assets, asset types, versions, dependencies, lifecycle state, and dirty review flows.

**Base URL:** `/api/v1`

**Authentication:** Protected endpoints require:

```text
Authorization: Bearer {org_id}:{user_id}:{role1,role2}:{project1,project2}
```

The MVP token format is intentionally simple. Production authentication is expected to use JWT or another signed token format.

## System

| Method | Path | Auth | Description |
| --- | --- | --- | --- |
| `GET` | `/health` | No | Health check. |

## Asset Types

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/api/v1/asset-types` | Create an asset type. |
| `GET` | `/api/v1/asset-types` | List asset types. |

Create request:

```json
{
  "name": "requirement",
  "display_name": "Requirement",
  "description": "Requirement assets",
  "metadata_schema": {}
}
```

## Assets

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/api/v1/assets` | Create an asset through `AssetService`. |
| `GET` | `/api/v1/assets?project_id={uuid}` | List project assets plus organization assets. |
| `GET` | `/api/v1/assets/{id}` | Read one asset. |
| `PUT` | `/api/v1/assets/{id}` | Update name, assignees, or metadata. |
| `DELETE` | `/api/v1/assets/{id}` | Delete an asset. |
| `POST` | `/api/v1/assets/{id}/archive` | Mark an asset archived. |

Create request:

```json
{
  "name": "User Service API",
  "asset_type_id": "550e8400-e29b-41d4-a716-446655440000",
  "project_id": "660e8400-e29b-41d4-a716-446655440001",
  "level": "project",
  "external_ref": "https://example.com/assets/user-service-api",
  "source": "manual",
  "metadata": {},
  "idempotency_key": "manual:user-service-api",
  "dependencies": []
}
```

Asset response includes `external_ref`, `source`, `metadata`, `assignees`, `publisher`, `current_version`, timestamps, level, and state.

## Versions And Lifecycle

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/api/v1/assets/{id}/releases` | Spec-style publish endpoint. |
| `POST` | `/api/v1/assets/{id}/publish` | Compatibility alias for publish. |
| `GET` | `/api/v1/assets/{id}/versions` | List persisted versions. |
| `GET` | `/api/v1/assets/{id}/versions/{version}` | Read one persisted version. |

Publish request:

```json
{
  "version": "1.2.0",
  "release_notes": "Adds dependency snapshot support",
  "suggested_type": "minor",
  "dependencies": [
    {
      "upstream_asset_id": "770e8400-e29b-41d4-a716-446655440002",
      "version": "^1.1.0",
      "relationship": "implements",
      "propagation_policy": "dirty",
      "upgrade_policy": "notify"
    }
  ]
}
```

Publish creates an `asset_versions` row, updates the asset's `current_version`, `publisher`, and state, updates dependency declared/effective baselines, and marks direct non-archived downstream assets Dirty.

## Dirty Review

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/api/v1/assets/{id}/manual-clean` | Spec-style manual clean endpoint. |
| `POST` | `/api/v1/assets/{id}/resolve` | Compatibility alias for manual clean. |

Manual clean request:

```json
{
  "resolved_version": "1.0.1",
  "reviewed_by": "reviewer",
  "resolutions": [
    {
      "upstream_asset_id": "770e8400-e29b-41d4-a716-446655440002",
      "from_version": "1.0.0",
      "to_version": "1.1.0",
      "review_result": "no_impact",
      "comment": "Documentation-only upstream change"
    }
  ]
}
```

Manual clean updates effective dependency baselines, resolves matching dirty queue entries, writes dirty resolution logs, and does not propagate Dirty downstream.

## Dependencies

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/api/v1/assets/{id}/dependencies` | List upstream dependency records. |
| `GET` | `/api/v1/assets/{id}/dependency-graph` | Return direct upstream and downstream dependency records. |

Dependency records include declared and effective versions plus effective baseline audit fields.

## Workflow Automation

> Slice 1/2 (Event → Action core, AgentTask claim/result). See `docs/plans/2026-06-18-workflow-automation-implementation-plan.md`.

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/api/workflow/events?correlation_id={uuid}` | List events by correlation id (or `asset_id={uuid}` for an asset's events). |
| `POST` | `/api/workflow/events` | Append an event idempotently and evaluate promotion rules to create actions. Requires an `Idempotency-Key` header. |
| `GET` | `/api/workflow/instances/{workflow_instance_id}` | Read one workflow instance (Saga coordinator). |
| `GET` | `/api/workflow/actions?target_asset_id={uuid}` | List active (non-terminal) actions targeting an asset; optional `status` filter. |
| `GET` | `/api/agent-tasks?status=queued&capability=create_virtual_asset_context` | List agent tasks in the caller's organization; optional `project_id`, `status`, and `capability` filters. |
| `POST` | `/api/agent-tasks/{task_id}/claim` | Atomically claim a queued task and set a lease. Returns `null` when another agent already claimed it. |
| `POST` | `/api/agent-tasks/{task_id}/result` | Store the task result, link produced assets, and complete the parent workflow action. |

`POST /api/workflow/events` request:

```json
{
  "event_type": "asset_published",
  "source_asset_id": "<requirement asset id>",
  "source_asset_type_id": "<requirement asset type id>",
  "project_id": "<optional project id>",
  "correlation_id": "<optional; generated if omitted>",
  "payload": { "version": "1.0.0" },
  "cascade_depth": 0
}
```

The response echoes the stored event plus `created_action_ids` (workflow actions created/reused by winning rules) and any `cascade_exceeded` violations. Replaying the same event (same `event_type`, `source_asset_id`, and `Idempotency-Key`) returns the same event and reuses the same actions — no duplicates.

`POST /api/agent-tasks/{task_id}/claim` request:

```json
{
  "agent_id": "agent-1",
  "lease_seconds": 900
}
```

`POST /api/agent-tasks/{task_id}/result` request:

```json
{
  "result_payload": { "ok": true },
  "produced_asset_ids": ["880e8400-e29b-41d4-a716-446655440003"]
}
```

MCP exposes matching Slice 2 tools: `list_pending_agent_tasks(project_id, capability_filter)`, `claim_agent_task(task_id, agent_id, lease_seconds)`, and `submit_agent_task_result(task_id, result_payload, produced_asset_ids)`.

## Status Codes

| Code | Meaning |
| --- | --- |
| `200` | Success. |
| `201` | Created. |
| `204` | Success with no body. |
| `400` | Invalid request. |
| `401` | Authentication required or invalid. |
| `403` | Permission denied. |
| `404` | Resource not found. |
| `409` | State conflict, duplicate idempotency key, or archived asset operation. |
| `422` | Request shape was syntactically valid but semantically invalid. |
| `500` | Internal server error. |

Error responses use:

```json
{
  "error": "message"
}
```

## Server Configuration

The server reads these environment variables:

| Variable | Values | Default |
| --- | --- | --- |
| `ADAM_REPOSITORY_BACKEND` | `memory`, `postgres` | `memory` |
| `ADAM_DATABASE__URL` | PostgreSQL connection URL | Required for `postgres` |
| `ADAM_SERVER__HOST` | Bind host | `0.0.0.0` |
| `ADAM_SERVER__PORT` | Bind port | `3000` |
| `ADAM_AGENT_TASK_EXPIRY_INTERVAL_SECONDS` | AgentTask expiry scan interval; `0` disables the worker | `60` |

Run modes:

```powershell
cargo run -p adam-server -- --rest   # REST only
cargo run -p adam-server -- --mcp    # MCP stdio only
cargo run -p adam-server -- --both   # REST and MCP
```
