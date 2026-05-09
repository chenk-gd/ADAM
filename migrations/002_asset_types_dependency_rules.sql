-- Asset types and dependency rules
-- ============================================================================

-- Asset types table
CREATE TABLE IF NOT EXISTS asset_types (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name VARCHAR(100) NOT NULL,
    display_name VARCHAR(200) NOT NULL,
    description TEXT,
    metadata_schema JSONB NOT NULL DEFAULT '{}',
    version_strategy VARCHAR(20) NOT NULL DEFAULT 'semver'
        CHECK (version_strategy IN ('semver', 'external_ref', 'composite')),
    retention_policy JSONB NOT NULL DEFAULT '{}',
    icon VARCHAR(200),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, name)
);

-- Dependency rules table
CREATE TABLE IF NOT EXISTS dependency_rules (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    source_type_id UUID NOT NULL REFERENCES asset_types(id),
    target_type_id UUID NOT NULL REFERENCES asset_types(id),
    relationship VARCHAR(50) NOT NULL DEFAULT 'depends_on',
    is_transitive BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, source_type_id, target_type_id)
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_asset_types_org ON asset_types(organization_id);
CREATE INDEX IF NOT EXISTS idx_asset_types_name ON asset_types(name);
CREATE INDEX IF NOT EXISTS idx_dependency_rules_org ON dependency_rules(organization_id);
