-- Virtual query contexts for MCP virtual assets
-- ============================================================================

CREATE TABLE IF NOT EXISTS virtual_instances (
    id UUID PRIMARY KEY,
    target_type_id UUID NOT NULL REFERENCES asset_types(id),
    target_type_name VARCHAR(200) NOT NULL,
    anchor_ids UUID[] NOT NULL DEFAULT '{}',
    organization_id UUID NOT NULL REFERENCES organizations(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    created_by VARCHAR(200) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    context_summary TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_virtual_instances_project ON virtual_instances(project_id);
CREATE INDEX IF NOT EXISTS idx_virtual_instances_expires ON virtual_instances(expires_at);
