-- Work item kind dependency policy
-- ============================================================================
-- Adds typed relationship policy metadata to persisted dependency rules and
-- dependency records. This migration is intentionally additive because 012 may
-- already have been applied in existing environments.

-- Dependency rule policy metadata
ALTER TABLE dependency_rules
ADD COLUMN IF NOT EXISTS source_metadata_filter JSONB,
ADD COLUMN IF NOT EXISTS target_metadata_filter JSONB,
ADD COLUMN IF NOT EXISTS propagation_policy VARCHAR(32) NOT NULL DEFAULT 'dirty';

UPDATE dependency_rules
SET propagation_policy = CASE
    WHEN relationship = 'references' THEN 'context_only'
    ELSE 'dirty'
END;

ALTER TABLE dependency_rules
DROP CONSTRAINT IF EXISTS chk_dependency_rules_relationship;

ALTER TABLE dependency_rules
ADD CONSTRAINT chk_dependency_rules_relationship
CHECK (
    relationship IN (
        'depends_on',
        'references',
        'implements',
        'fixes',
        'verifies',
        'executes',
        'produces',
        'blocks',
        'relates_to'
    )
);

ALTER TABLE dependency_rules
DROP CONSTRAINT IF EXISTS chk_dependency_rules_propagation_policy;

ALTER TABLE dependency_rules
ADD CONSTRAINT chk_dependency_rules_propagation_policy
CHECK (propagation_policy IN ('dirty', 'context_only', 'audit_only'));

ALTER TABLE dependency_rules
DROP CONSTRAINT IF EXISTS dependency_rules_organization_id_source_type_id_target_type_id_key;

CREATE UNIQUE INDEX IF NOT EXISTS idx_dependency_rules_unique_metadata
ON dependency_rules (
    organization_id,
    source_type_id,
    target_type_id,
    relationship,
    (COALESCE(source_metadata_filter::TEXT, '')),
    (COALESCE(target_metadata_filter::TEXT, ''))
);

-- Dependency record propagation policy
ALTER TABLE asset_dependencies
ADD COLUMN IF NOT EXISTS propagation_policy VARCHAR(32) NOT NULL DEFAULT 'dirty';

UPDATE asset_dependencies
SET propagation_policy = CASE
    WHEN relationship = 'references' THEN 'context_only'
    ELSE 'dirty'
END;

-- Normalize version policy columns to the snake_case values used by the domain,
-- REST API, and serde boundaries.
ALTER TABLE asset_dependencies DROP CONSTRAINT IF EXISTS chk_upgrade_policy;
ALTER TABLE asset_dependencies DROP CONSTRAINT IF EXISTS chk_effective_reason;

UPDATE asset_dependencies
SET upgrade_policy = CASE upgrade_policy
    WHEN 'AutoPatch' THEN 'auto_patch'
    WHEN 'AutoMinor' THEN 'auto_minor'
    WHEN 'Notify' THEN 'notify'
    WHEN 'Manual' THEN 'manual'
    WHEN 'Pin' THEN 'pin'
    ELSE upgrade_policy
END;

UPDATE asset_dependencies
SET effective_reason = CASE effective_reason
    WHEN 'Publish' THEN 'publish'
    WHEN 'ManualClean' THEN 'manual_clean'
    ELSE effective_reason
END;

ALTER TABLE asset_dependencies
DROP CONSTRAINT IF EXISTS chk_asset_dependencies_relationship;

ALTER TABLE asset_dependencies
ADD CONSTRAINT chk_asset_dependencies_relationship
CHECK (
    relationship IN (
        'depends_on',
        'references',
        'implements',
        'fixes',
        'verifies',
        'executes',
        'produces',
        'blocks',
        'relates_to'
    )
);

ALTER TABLE asset_dependencies
DROP CONSTRAINT IF EXISTS chk_asset_dependencies_propagation_policy;

ALTER TABLE asset_dependencies
ADD CONSTRAINT chk_asset_dependencies_propagation_policy
CHECK (propagation_policy IN ('dirty', 'context_only', 'audit_only'));

ALTER TABLE asset_dependencies
ADD CONSTRAINT chk_upgrade_policy
CHECK (upgrade_policy IN ('auto_patch', 'auto_minor', 'notify', 'manual', 'pin'));

ALTER TABLE asset_dependencies
ADD CONSTRAINT chk_effective_reason
CHECK (effective_reason IN ('publish', 'manual_clean'));

-- Extend the existing work_item metadata schema with a subtype discriminator.
UPDATE asset_types
SET metadata_schema = jsonb_set(
        jsonb_set(
            metadata_schema,
            '{properties,work_item_kind}',
            '{"type":"string","enum":["feature","bugfix","test_execution","refactor","maintenance","release"]}'::JSONB,
            true
        ),
        '{required}',
        COALESCE(metadata_schema->'required', '[]'::JSONB) || '["work_item_kind"]'::JSONB,
        true
    ),
    updated_at = NOW()
WHERE name = 'work_item'
  AND NOT (metadata_schema->'properties' ? 'work_item_kind');

-- First-slice work item subtype rules.
INSERT INTO dependency_rules (
    id,
    organization_id,
    source_type_id,
    target_type_id,
    relationship,
    is_transitive,
    source_metadata_filter,
    target_metadata_filter,
    propagation_policy,
    created_at
)
SELECT
    '13131313-0000-4000-8000-000000000001'::UUID,
    org.id,
    source_type.id,
    target_type.id,
    'implements',
    true,
    '{"work_item_kind":"feature"}'::JSONB,
    NULL,
    'dirty',
    NOW()
FROM organizations org
JOIN asset_types source_type ON source_type.organization_id = org.id AND source_type.name = 'work_item'
JOIN asset_types target_type ON target_type.organization_id = org.id AND target_type.name = 'requirement'
WHERE NOT EXISTS (
    SELECT 1
    FROM dependency_rules existing
    WHERE existing.organization_id = org.id
      AND existing.source_type_id = source_type.id
      AND existing.target_type_id = target_type.id
      AND existing.relationship = 'implements'
      AND existing.source_metadata_filter = '{"work_item_kind":"feature"}'::JSONB
);

INSERT INTO dependency_rules (
    id,
    organization_id,
    source_type_id,
    target_type_id,
    relationship,
    is_transitive,
    source_metadata_filter,
    target_metadata_filter,
    propagation_policy,
    created_at
)
SELECT
    '13131313-0000-4000-8000-000000000002'::UUID,
    org.id,
    source_type.id,
    target_type.id,
    'fixes',
    true,
    '{"work_item_kind":"bugfix"}'::JSONB,
    NULL,
    'dirty',
    NOW()
FROM organizations org
JOIN asset_types source_type ON source_type.organization_id = org.id AND source_type.name = 'work_item'
JOIN asset_types target_type ON target_type.organization_id = org.id AND target_type.name = 'requirement'
WHERE NOT EXISTS (
    SELECT 1
    FROM dependency_rules existing
    WHERE existing.organization_id = org.id
      AND existing.source_type_id = source_type.id
      AND existing.target_type_id = target_type.id
      AND existing.relationship = 'fixes'
      AND existing.source_metadata_filter = '{"work_item_kind":"bugfix"}'::JSONB
);

INSERT INTO dependency_rules (
    id,
    organization_id,
    source_type_id,
    target_type_id,
    relationship,
    is_transitive,
    source_metadata_filter,
    target_metadata_filter,
    propagation_policy,
    created_at
)
SELECT
    '13131313-0000-4000-8000-000000000003'::UUID,
    org.id,
    source_type.id,
    target_type.id,
    'references',
    false,
    '{"work_item_kind":"bugfix"}'::JSONB,
    NULL,
    'context_only',
    NOW()
FROM organizations org
JOIN asset_types source_type ON source_type.organization_id = org.id AND source_type.name = 'work_item'
JOIN asset_types target_type ON target_type.organization_id = org.id AND target_type.name = 'test_case'
WHERE NOT EXISTS (
    SELECT 1
    FROM dependency_rules existing
    WHERE existing.organization_id = org.id
      AND existing.source_type_id = source_type.id
      AND existing.target_type_id = target_type.id
      AND existing.relationship = 'references'
      AND existing.source_metadata_filter = '{"work_item_kind":"bugfix"}'::JSONB
);

INSERT INTO dependency_rules (
    id,
    organization_id,
    source_type_id,
    target_type_id,
    relationship,
    is_transitive,
    source_metadata_filter,
    target_metadata_filter,
    propagation_policy,
    created_at
)
SELECT
    '13131313-0000-4000-8000-000000000004'::UUID,
    org.id,
    source_type.id,
    target_type.id,
    'executes',
    false,
    '{"work_item_kind":"test_execution"}'::JSONB,
    NULL,
    'context_only',
    NOW()
FROM organizations org
JOIN asset_types source_type ON source_type.organization_id = org.id AND source_type.name = 'work_item'
JOIN asset_types target_type ON target_type.organization_id = org.id AND target_type.name = 'test_case'
WHERE NOT EXISTS (
    SELECT 1
    FROM dependency_rules existing
    WHERE existing.organization_id = org.id
      AND existing.source_type_id = source_type.id
      AND existing.target_type_id = target_type.id
      AND existing.relationship = 'executes'
      AND existing.source_metadata_filter = '{"work_item_kind":"test_execution"}'::JSONB
);

INSERT INTO dependency_rules (
    id,
    organization_id,
    source_type_id,
    target_type_id,
    relationship,
    is_transitive,
    source_metadata_filter,
    target_metadata_filter,
    propagation_policy,
    created_at
)
SELECT
    '13131313-0000-4000-8000-000000000005'::UUID,
    org.id,
    source_type.id,
    target_type.id,
    'verifies',
    true,
    NULL,
    NULL,
    'dirty',
    NOW()
FROM organizations org
JOIN asset_types source_type ON source_type.organization_id = org.id AND source_type.name = 'test_case'
JOIN asset_types target_type ON target_type.organization_id = org.id AND target_type.name = 'requirement'
WHERE NOT EXISTS (
    SELECT 1
    FROM dependency_rules existing
    WHERE existing.organization_id = org.id
      AND existing.source_type_id = source_type.id
      AND existing.target_type_id = target_type.id
      AND existing.relationship = 'verifies'
);

COMMENT ON COLUMN dependency_rules.source_metadata_filter IS
    'Optional top-level exact-match filter on source asset metadata for subtype-specific rules';

COMMENT ON COLUMN dependency_rules.target_metadata_filter IS
    'Optional top-level exact-match filter on target asset metadata for subtype-specific rules';

COMMENT ON COLUMN dependency_rules.propagation_policy IS
    'dirty: mark downstream dirty; context_only: graph/context only; audit_only: trace only';

COMMENT ON COLUMN asset_dependencies.propagation_policy IS
    'Resolved propagation behavior stored on each dependency edge';
