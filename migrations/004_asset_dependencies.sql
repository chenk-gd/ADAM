-- Asset dependencies table
-- ============================================================================
-- Direction: source_id (下游依赖方) -> target_id (上游被依赖方)

CREATE TABLE IF NOT EXISTS asset_dependencies (
    id UUID PRIMARY KEY,
    source_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE CASCADE,
    target_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE RESTRICT,
    relationship VARCHAR(50) NOT NULL DEFAULT 'depends_on',
    declared_version VARCHAR(200) NOT NULL,
    effective_version VARCHAR(200) NOT NULL,
    effective_updated_by VARCHAR(200) NOT NULL,
    effective_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    effective_reason VARCHAR(50) NOT NULL CHECK (effective_reason IN ('publish', 'manual_clean')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(source_id, target_id)
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_asset_deps_source ON asset_dependencies(source_id);
CREATE INDEX IF NOT EXISTS idx_asset_deps_target ON asset_dependencies(target_id);
CREATE INDEX IF NOT EXISTS idx_asset_deps_source_target ON asset_dependencies(source_id, target_id);
