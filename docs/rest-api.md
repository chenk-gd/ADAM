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

Run modes:

```powershell
cargo run -p adam-server -- --rest   # REST only
cargo run -p adam-server -- --mcp    # MCP stdio only
cargo run -p adam-server -- --both   # REST and MCP
```
