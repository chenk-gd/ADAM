-- Dirty queue table for state propagation
-- ============================================================================
-- Stores dirty state notifications for downstream assets

CREATE TABLE IF NOT EXISTS dirty_queue (
    id UUID PRIMARY KEY,
    -- 受影响的资产（下游）
    asset_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE CASCADE,
    -- 导致 Dirty 的上游资产
    upstream_asset_id UUID NOT NULL REFERENCES asset_instances(id),
    -- 上游新版本
    upstream_version VARCHAR(200) NOT NULL,
    -- 上游旧版本（当前有效基线）
    upstream_old_version VARCHAR(200) NOT NULL,
    -- 影响等级
    impact_level VARCHAR(20) NOT NULL CHECK (impact_level IN ('low', 'medium', 'high', 'critical')),
    -- 是否已解决
    resolved BOOLEAN NOT NULL DEFAULT false,
    -- 解决时间
    resolved_at TIMESTAMPTZ,
    -- 解决方式（manual_clean 或 republish）
    resolution_action VARCHAR(50) CHECK (resolution_action IN ('manual_clean', 'republish')),
    -- 处理人
    resolved_by VARCHAR(200),
    -- 处理备注
    resolution_comment TEXT,
    -- 幂等性键（用于重复检测）
    idempotency_key VARCHAR(500),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_dirty_queue_asset ON dirty_queue(asset_id);
CREATE INDEX IF NOT EXISTS idx_dirty_queue_upstream ON dirty_queue(upstream_asset_id);
CREATE INDEX IF NOT EXISTS idx_dirty_queue_resolved ON dirty_queue(resolved);
CREATE INDEX IF NOT EXISTS idx_dirty_queue_created ON dirty_queue(created_at);

-- 部分唯一索引：仅限制未解决的 (resolved = false) 条目唯一
-- 允许同一资产和上游组合有多个已解决的记录，但最多一个未解决的
CREATE UNIQUE INDEX IF NOT EXISTS idx_dirty_queue_unresolved
ON dirty_queue (asset_id, upstream_asset_id)
WHERE resolved = false;
