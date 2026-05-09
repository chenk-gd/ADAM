-- Asset instances table
-- ============================================================================

CREATE TABLE IF NOT EXISTS asset_instances (
    id UUID PRIMARY KEY,
    type_id UUID NOT NULL REFERENCES asset_types(id),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    name VARCHAR(500) NOT NULL,
    external_ref TEXT NOT NULL,
    source VARCHAR(50) NOT NULL,
    level VARCHAR(20) NOT NULL CHECK (level IN ('project', 'organization')),
    project_id UUID REFERENCES projects(id),
    current_version VARCHAR(200) NOT NULL,
    current_state VARCHAR(20) NOT NULL CHECK (current_state IN ('clean', 'dirty', 'archived')),
    archived_at TIMESTAMPTZ,
    archived_reason TEXT,
    archived_version VARCHAR(200),
    metadata JSONB NOT NULL DEFAULT '{}',
    assignees JSONB NOT NULL DEFAULT '[]',
    -- 幂等性：按组织 + 外部引用唯一
    idempotency_key VARCHAR(500),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 幂等性约束（仅针对非空值）
    UNIQUE(organization_id, idempotency_key)
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_asset_instances_org ON asset_instances(organization_id);
CREATE INDEX IF NOT EXISTS idx_asset_instances_type ON asset_instances(type_id);
CREATE INDEX IF NOT EXISTS idx_asset_instances_project ON asset_instances(project_id);
CREATE INDEX IF NOT EXISTS idx_asset_instances_state ON asset_instances(current_state);
CREATE INDEX IF NOT EXISTS idx_asset_instances_level ON asset_instances(level);
CREATE INDEX IF NOT EXISTS idx_asset_instances_external_ref ON asset_instances(external_ref);
CREATE INDEX IF NOT EXISTS idx_asset_instances_idempotency ON asset_instances(idempotency_key) WHERE idempotency_key IS NOT NULL;
