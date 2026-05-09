# ADAM REST API Documentation

## Overview

The ADAM REST API provides HTTP endpoints for managing R&D assets, their dependencies, and lifecycle states.

**Base URL:** `/api/v1`

**Authentication:** All endpoints require Bearer token authentication via the `Authorization` header:
```
Authorization: Bearer {org_id}:{user_id}:{project1,project2,...}
```

For MVP, the token format is simple. In production, this will be a JWT.

---

## Endpoints

### Assets

#### Create Asset

Creates a new asset instance.

**Endpoint:** `POST /api/v1/assets`

**Authentication:** Required

**Request Body:**

```json
{
  "name": "string",           // Required: Asset name
  "asset_type_id": "uuid",    // Required: Asset type UUID
  "project_id": "uuid",       // Optional: For project-level assets
  "level": "project|organization", // Required: Asset level
  "idempotency_key": "string", // Optional: Prevent duplicate creation
  "dependencies": ["uuid"]     // Optional: Asset dependencies
}
```

**Response:**

- `201 Created` - Asset created successfully
- `400 Bad Request` - Invalid request body
- `401 Unauthorized` - Missing or invalid authentication
- `409 Conflict` - Duplicate idempotency key

**Example:**

```bash
curl -X POST http://localhost:8080/api/v1/assets \
  -H "Authorization: Bearer org-123:user-456:proj-789" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "User Service API",
    "asset_type_id": "550e8400-e29b-41d4-a716-446655440000",
    "level": "project",
    "project_id": "660e8400-e29b-41d4-a716-446655440001"
  }'
```

**Response Body:**

```json
{
  "id": "770e8400-e29b-41d4-a716-446655440002",
  "name": "User Service API",
  "asset_type_id": "550e8400-e29b-41d4-a716-446655440000",
  "organization_id": "org-123",
  "project_id": "660e8400-e29b-41d4-a716-446655440001",
  "level": "project",
  "current_state": "clean",
  "created_at": "2026-05-09T10:30:00Z",
  "updated_at": "2026-05-09T10:30:00Z"
}
```

---

#### List Assets

Retrieves assets for a specific project, including both project-level and organization-level assets (per FR-026).

**Endpoint:** `GET /api/v1/assets?project_id={uuid}`

**Authentication:** Required

**Query Parameters:**

- `project_id` (required): UUID of the project to query

**Response:**

- `200 OK` - Assets retrieved successfully
- `400 Bad Request` - Missing required project_id parameter
- `401 Unauthorized` - Missing or invalid authentication

**Example:**

```bash
curl "http://localhost:8080/api/v1/assets?project_id=660e8400-e29b-41d4-a716-446655440001" \
  -H "Authorization: Bearer org-123:user-456:proj-789"
```

**Response Body:**

```json
[
  {
    "id": "770e8400-e29b-41d4-a716-446655440002",
    "name": "User Service API",
    "asset_type_id": "550e8400-e29b-41d4-a716-446655440000",
    "organization_id": "org-123",
    "project_id": "660e8400-e29b-41d4-a716-446655440001",
    "level": "project",
    "current_state": "clean",
    "created_at": "2026-05-09T10:30:00Z",
    "updated_at": "2026-05-09T10:30:00Z"
  },
  {
    "id": "880e8400-e29b-41d4-a716-446655440003",
    "name": "Organization Coding Standards",
    "asset_type_id": "550e8400-e29b-41d4-a716-446655440001",
    "organization_id": "org-123",
    "project_id": null,
    "level": "organization",
    "current_state": "clean",
    "created_at": "2026-05-01T08:00:00Z",
    "updated_at": "2026-05-01T08:00:00Z"
  }
]
```

---

#### Get Asset

Retrieves a specific asset by ID.

**Endpoint:** `GET /api/v1/assets/{id}`

**Authentication:** Required

**Path Parameters:**

- `id`: Asset UUID

**Response:**

- `200 OK` - Asset found
- `401 Unauthorized` - Missing or invalid authentication
- `404 Not Found` - Asset does not exist

**Example:**

```bash
curl "http://localhost:8080/api/v1/assets/770e8400-e29b-41d4-a716-446655440002" \
  -H "Authorization: Bearer org-123:user-456:proj-789"
```

---

#### Publish Asset Version

Publishes a new version of an asset, triggering dirty state propagation to downstream dependencies.

**Endpoint:** `POST /api/v1/assets/{id}/publish`

**Authentication:** Required

**Path Parameters:**

- `id`: Asset UUID

**Request Body:**

```json
{
  "version": "string"  // Required: Semantic version (e.g., "v2.0.0")
}
```

**Response:**

- `200 OK` - Published successfully
- `401 Unauthorized` - Missing or invalid authentication
- `404 Not Found` - Asset does not exist
- `409 Conflict` - Cannot publish archived asset

**Example:**

```bash
curl -X POST "http://localhost:8080/api/v1/assets/770e8400-e29b-41d4-a716-446655440002/publish" \
  -H "Authorization: Bearer org-123:user-456:proj-789" \
  -H "Content-Type: application/json" \
  -d '{"version": "v2.0.0"}'
```

**Response Body:**

```json
{
  "affected_assets": [
    "990e8400-e29b-41d4-a716-446655440004",
    "aa0e8400-e29b-41d4-a716-446655440005"
  ]
}
```

---

#### Resolve Dirty State

Marks an asset's dirty state as resolved after reviewing upstream changes.

**Endpoint:** `POST /api/v1/assets/{id}/resolve`

**Authentication:** Required

**Path Parameters:**

- `id`: Asset UUID

**Request Body:**

```json
{
  "resolved_version": "string"  // Required: Version being resolved to
}
```

**Response:**

- `204 No Content` - Resolved successfully
- `401 Unauthorized` - Missing or invalid authentication
- `404 Not Found` - Asset or dirty queue entry does not exist

**Example:**

```bash
curl -X POST "http://localhost:8080/api/v1/assets/990e8400-e29b-41d4-a716-446655440004/resolve" \
  -H "Authorization: Bearer org-123:user-456:proj-789" \
  -H "Content-Type: application/json" \
  -d '{"resolved_version": "v2.0.0"}'
```

---

### Health

#### Health Check

Public endpoint for service health monitoring.

**Endpoint:** `GET /health`

**Authentication:** None

**Response:**

- `200 OK` - Service is healthy

**Example:**

```bash
curl http://localhost:8080/health
```

**Response Body:**

```json
{
  "status": "healthy",
  "version": "0.1.0"
}
```

---

## Error Responses

All error responses follow this format:

```json
{
  "error": "Error message describing what went wrong"
}
```

### HTTP Status Codes

| Code | Meaning |
|------|---------|
| 200 | Success |
| 201 | Created |
| 204 | No Content |
| 400 | Bad Request - Invalid input |
| 401 | Unauthorized - Authentication required |
| 403 | Forbidden - Permission denied |
| 404 | Not Found - Resource doesn't exist |
| 409 | Conflict - Resource conflict (duplicate, invalid state) |
| 422 | Unprocessable Entity - Validation error |
| 500 | Internal Server Error |

---

## Asset States

Assets have three possible states:

- **Clean**: Asset is up-to-date with upstream dependencies
- **Dirty**: An upstream dependency has a newer version; review required
- **Archived**: Asset is read-only and no longer maintained

### State Transitions

```
Clean → Dirty:  Upstream dependency published new version
Dirty → Clean:  Manual resolution via /resolve endpoint
Clean → Archived: Archive operation
Dirty → Archived: Archive operation
```

Archived assets cannot transition to other states.

---

## Data Types

### AssetLevel

- `project`: Asset belongs to a specific project
- `organization`: Asset is shared across all projects in the organization

### AssetState

- `clean`: Up-to-date
- `dirty`: Review required
- `archived`: Read-only

---

## OpenAPI Specification

For machine-readable API documentation, see `openapi.yaml` in the repository root.
