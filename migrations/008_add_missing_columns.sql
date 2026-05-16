-- Migration 008: Add missing columns to asset_instances
-- ============================================================================
-- Fixes column mismatch between code and database schema

-- Add external_ref column (VARCHAR as per spec)
ALTER TABLE asset_instances
ADD COLUMN IF NOT EXISTS external_ref VARCHAR(500) NOT NULL DEFAULT '';

-- Add source column
ALTER TABLE asset_instances
ADD COLUMN IF NOT EXISTS source VARCHAR(50) NOT NULL DEFAULT 'manual';

-- Add metadata JSONB column
ALTER TABLE asset_instances
ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}';

-- Add assignees array column
ALTER TABLE asset_instances
ADD COLUMN IF NOT EXISTS assignees VARCHAR(100)[] NOT NULL DEFAULT '{}';

-- Add publisher column (nullable)
ALTER TABLE asset_instances
ADD COLUMN IF NOT EXISTS publisher VARCHAR(100);

-- Make current_version nullable (to allow unpublished assets)
ALTER TABLE asset_instances
ALTER COLUMN current_version DROP NOT NULL;

-- Update indexes
CREATE INDEX IF NOT EXISTS idx_asset_instances_source ON asset_instances(source);
CREATE INDEX IF NOT EXISTS idx_asset_instances_publisher ON asset_instances(publisher) WHERE publisher IS NOT NULL;
