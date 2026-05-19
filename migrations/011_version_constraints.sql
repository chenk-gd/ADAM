-- Version constraints schema migration
-- ============================================================================
-- Adds SemVer support and version constraint fields

-- Update asset_instances to use structured SemVer
-- ----------------------------------------------------------------------------
ALTER TABLE asset_instances
ADD COLUMN IF NOT EXISTS current_version_major INTEGER,
ADD COLUMN IF NOT EXISTS current_version_minor INTEGER,
ADD COLUMN IF NOT EXISTS current_version_patch INTEGER,
ADD COLUMN IF NOT EXISTS lock_version BIGINT NOT NULL DEFAULT 1;

-- Migrate existing version data from VARCHAR to structured columns
-- Extract major, minor, patch from current_version string
UPDATE asset_instances SET
    current_version_major = CAST(SPLIT_PART(current_version, '.', 1) AS INTEGER),
    current_version_minor = CAST(SPLIT_PART(current_version, '.', 2) AS INTEGER),
    current_version_patch = CAST(SPLIT_PART(current_version, '.', 3) AS INTEGER)
WHERE current_version IS NOT NULL
  AND current_version_major IS NULL;

-- Drop old current_version column
ALTER TABLE asset_instances DROP COLUMN IF EXISTS current_version;

-- Update asset_dependencies with constraint support
-- ----------------------------------------------------------------------------
ALTER TABLE asset_dependencies
ADD COLUMN IF NOT EXISTS declared_constraint VARCHAR(255) NOT NULL DEFAULT '^1.0.0',
ADD COLUMN IF NOT EXISTS constraint_str VARCHAR(255) NOT NULL DEFAULT '^1.0.0',
ADD COLUMN IF NOT EXISTS effective_version_major INTEGER NOT NULL DEFAULT 0,
ADD COLUMN IF NOT EXISTS effective_version_minor INTEGER NOT NULL DEFAULT 0,
ADD COLUMN IF NOT EXISTS effective_version_patch INTEGER NOT NULL DEFAULT 0,
ADD COLUMN IF NOT EXISTS upgrade_policy VARCHAR(50) NOT NULL DEFAULT 'Notify',
ADD COLUMN IF NOT EXISTS lock_version BIGINT NOT NULL DEFAULT 1;

-- Migrate existing dependency data
-- Convert declared_version to constraint format (add ^ prefix)
UPDATE asset_dependencies SET
    declared_constraint = '^' || declared_version,
    constraint_str = '^' || declared_version
WHERE declared_constraint = '^1.0.0';

-- Migrate effective_version to structured columns
UPDATE asset_dependencies SET
    effective_version_major = CAST(SPLIT_PART(effective_version, '.', 1) AS INTEGER),
    effective_version_minor = CAST(SPLIT_PART(effective_version, '.', 2) AS INTEGER),
    effective_version_patch = CAST(SPLIT_PART(effective_version, '.', 3) AS INTEGER)
WHERE effective_version_major = 0
  AND effective_version_minor = 0
  AND effective_version_patch = 0
  AND effective_version IS NOT NULL;

-- Drop old version columns
ALTER TABLE asset_dependencies DROP COLUMN IF EXISTS declared_version;
ALTER TABLE asset_dependencies DROP COLUMN IF EXISTS effective_version;

-- Create indexes for efficient version lookups
-- ----------------------------------------------------------------------------
CREATE INDEX IF NOT EXISTS idx_asset_deps_version_lookup ON asset_dependencies(
    target_id,
    effective_version_major,
    effective_version_minor,
    effective_version_patch
);

-- Add check constraint for upgrade_policy values
-- ----------------------------------------------------------------------------
ALTER TABLE asset_dependencies
ADD CONSTRAINT chk_upgrade_policy
CHECK (upgrade_policy IN ('AutoPatch', 'AutoMinor', 'Notify', 'Manual', 'Pin'));

-- Add check constraint for effective_reason values (ensure consistency)
-- ----------------------------------------------------------------------------
ALTER TABLE asset_dependencies
DROP CONSTRAINT IF EXISTS asset_dependencies_effective_reason_check;

ALTER TABLE asset_dependencies
ADD CONSTRAINT chk_effective_reason
CHECK (effective_reason IN ('Publish', 'ManualClean'));
