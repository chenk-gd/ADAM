-- Migration 009: Add since column to dirty_queue
-- ============================================================================
-- Fixes missing `since` column that is used by queries but not in the table

-- Add since column (when the dirty state was triggered)
ALTER TABLE dirty_queue
ADD COLUMN IF NOT EXISTS since TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Create index for efficient querying by timeframe
CREATE INDEX IF NOT EXISTS idx_dirty_queue_since ON dirty_queue(since);
