-- Asset versions and dirty resolution logs
-- ============================================================================

-- Asset versions table - records version publishes
CREATE TABLE IF NOT EXISTS asset_versions (
    id UUID PRIMARY KEY,
    instance_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE CASCADE,
    version_number VARCHAR(200) NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    dependencies JSONB NOT NULL DEFAULT '[]',
    release_notes TEXT,
    suggested_type VARCHAR(20) CHECK (suggested_type IN ('major', 'minor', 'patch')),
    released_by VARCHAR(200) NOT NULL,
    released_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(instance_id, version_number)
);

-- Dirty resolution logs table
CREATE TABLE IF NOT EXISTS dirty_resolution_logs (
    id UUID PRIMARY KEY,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    asset_version VARCHAR(200) NOT NULL,
    upstream_asset_id UUID NOT NULL REFERENCES asset_instances(id),
    from_version VARCHAR(200) NOT NULL,
    to_version VARCHAR(200) NOT NULL,
    action VARCHAR(50) NOT NULL CHECK (action IN ('manual_clean', 'republish')),
    review_result VARCHAR(50) NOT NULL CHECK (review_result IN ('no_impact', 'updated')),
    comment TEXT,
    reviewed_by VARCHAR(200) NOT NULL,
    reviewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_asset_versions_instance ON asset_versions(instance_id);
CREATE INDEX IF NOT EXISTS idx_asset_versions_released ON asset_versions(released_at);
CREATE INDEX IF NOT EXISTS idx_dirty_resolution_asset ON dirty_resolution_logs(asset_id);
CREATE INDEX IF NOT EXISTS idx_dirty_resolution_upstream ON dirty_resolution_logs(upstream_asset_id);
CREATE INDEX IF NOT EXISTS idx_dirty_resolution_reviewed ON dirty_resolution_logs(reviewed_at);
