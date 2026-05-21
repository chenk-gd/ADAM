-- Default dependency rules for standard asset types
-- ============================================================================
-- This migration creates default dependency rules that define the standard
-- R&D workflow: Requirement → Design → Work Item → Code Commit → Pipeline Run
--
-- Relationship types:
--   - depends_on: Dirty propagation enabled (upstream change affects downstream)
--   - references: Dirty propagation disabled (reference only, no state change)
--
-- Note: Final state assets (code_commit, pipeline_run) don't receive dirty
-- propagation even with depends_on relationship, but rules are still needed
-- for query context and validation.
-- ============================================================================

-- Insert default asset types first (if not exists)
-- These are standard types that every organization should have

-- Requirement asset type (root of dependency chain)
INSERT INTO asset_types (id, organization_id, name, display_name, description, metadata_schema, version_strategy, created_at)
SELECT
    '11111111-1111-1111-1111-111111111111'::UUID,
    org.id,
    'requirement',
    '需求',
    '功能需求和业务需求',
    '{"type": "object", "properties": {"title": {"type": "string"}, "priority": {"type": "string"}, "description": {"type": "string"}}}',
    'semver',
    NOW()
FROM organizations org
WHERE NOT EXISTS (SELECT 1 FROM asset_types WHERE name = 'requirement');

-- Design Doc asset type
INSERT INTO asset_types (id, organization_id, name, display_name, description, metadata_schema, version_strategy, created_at)
SELECT
    '22222222-2222-2222-2222-222222222222'::UUID,
    org.id,
    'design_doc',
    '设计文档',
    '架构设计和详细设计文档',
    '{"type": "object", "properties": {"title": {"type": "string"}, "design_type": {"type": "string"}, "content": {"type": "string"}}}',
    'semver',
    NOW()
FROM organizations org
WHERE NOT EXISTS (SELECT 1 FROM asset_types WHERE name = 'design_doc');

-- Work Item asset type
INSERT INTO asset_types (id, organization_id, name, display_name, description, metadata_schema, version_strategy, created_at)
SELECT
    '33333333-3333-3333-3333-333333333333'::UUID,
    org.id,
    'work_item',
    '工作项',
    '开发任务和工作项',
    '{"type": "object", "properties": {"title": {"type": "string"}, "status": {"type": "string"}, "assignee": {"type": "string"}}}',
    'semver',
    NOW()
FROM organizations org
WHERE NOT EXISTS (SELECT 1 FROM asset_types WHERE name = 'work_item');

-- Code Commit asset type (uses external_ref for version)
INSERT INTO asset_types (id, organization_id, name, display_name, description, metadata_schema, version_strategy, created_at)
SELECT
    '44444444-4444-4444-4444-444444444444'::UUID,
    org.id,
    'code_commit',
    '代码提交',
    'Git代码提交记录',
    '{"type": "object", "properties": {"hash": {"type": "string"}, "message": {"type": "string"}, "author": {"type": "string"}, "branch": {"type": "string"}}}',
    'external_ref',
    NOW()
FROM organizations org
WHERE NOT EXISTS (SELECT 1 FROM asset_types WHERE name = 'code_commit');

-- Pipeline Run asset type (uses external_ref for version)
INSERT INTO asset_types (id, organization_id, name, display_name, description, metadata_schema, version_strategy, created_at)
SELECT
    '55555555-5555-5555-5555-555555555555'::UUID,
    org.id,
    'pipeline_run',
    '流水线执行',
    'CI/CD流水线执行记录',
    '{"type": "object", "properties": {"build_number": {"type": "string"}, "status": {"type": "string"}, "trigger": {"type": "string"}, "duration_ms": {"type": "integer"}}}',
    'external_ref',
    NOW()
FROM organizations org
WHERE NOT EXISTS (SELECT 1 FROM asset_types WHERE name = 'pipeline_run');

-- Test Case asset type
INSERT INTO asset_types (id, organization_id, name, display_name, description, metadata_schema, version_strategy, created_at)
SELECT
    '66666666-6666-6666-6666-666666666666'::UUID,
    org.id,
    'test_case',
    '测试用例',
    '自动化测试用例',
    '{"type": "object", "properties": {"title": {"type": "string"}, "test_type": {"type": "string"}, "automation": {"type": "boolean"}}}',
    'semver',
    NOW()
FROM organizations org
WHERE NOT EXISTS (SELECT 1 FROM asset_types WHERE name = 'test_case');

-- Coding Standard asset type (organization-level)
INSERT INTO asset_types (id, organization_id, name, display_name, description, metadata_schema, version_strategy, created_at)
SELECT
    '77777777-7777-7777-7777-777777777777'::UUID,
    org.id,
    'coding_standard',
    '编码规范',
    '组织编码规范和标准',
    '{"type": "object", "properties": {"category": {"type": "string"}, "rules": {"type": "array"}}}',
    'semver',
    NOW()
FROM organizations org
WHERE NOT EXISTS (SELECT 1 FROM asset_types WHERE name = 'coding_standard');

-- ============================================================================
-- Dependency Rules
-- ============================================================================

-- Work Item depends on Requirement (dirty propagation)
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa'::UUID,
    org.id,
    '33333333-3333-3333-3333-333333333333'::UUID, -- work_item
    '11111111-1111-1111-1111-111111111111'::UUID, -- requirement
    'depends_on',
    true
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '33333333-3333-3333-3333-333333333333'::UUID
    AND target_type_id = '11111111-1111-1111-1111-111111111111'::UUID
);

-- Work Item depends on Design Doc (dirty propagation)
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb'::UUID,
    org.id,
    '33333333-3333-3333-3333-333333333333'::UUID, -- work_item
    '22222222-2222-2222-2222-222222222222'::UUID, -- design_doc
    'depends_on',
    true
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '33333333-3333-3333-3333-333333333333'::UUID
    AND target_type_id = '22222222-2222-2222-2222-222222222222'::UUID
);

-- Code Commit depends on Work Item (dirty propagation)
-- Note: code_commit is Final state, so it won't actually become dirty,
-- but this rule enables query context (commit → work item → requirement)
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'cccccccc-cccc-cccc-cccc-cccccccccccc'::UUID,
    org.id,
    '44444444-4444-4444-4444-444444444444'::UUID, -- code_commit
    '33333333-3333-3333-3333-333333333333'::UUID, -- work_item
    'depends_on',
    true
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '44444444-4444-4444-4444-444444444444'::UUID
    AND target_type_id = '33333333-3333-3333-3333-333333333333'::UUID
);

-- Code Commit depends on Design Doc (dirty propagation)
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'dddddddd-dddd-dddd-dddd-dddddddddddd'::UUID,
    org.id,
    '44444444-4444-4444-4444-444444444444'::UUID, -- code_commit
    '22222222-2222-2222-2222-222222222222'::UUID, -- design_doc
    'depends_on',
    true
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '44444444-4444-4444-4444-444444444444'::UUID
    AND target_type_id = '22222222-2222-2222-2222-222222222222'::UUID
);

-- Code Commit references Coding Standard (no dirty propagation)
-- This is a reference relationship - code follows the standard at creation time
-- Standard updates don't make existing commits "dirty"
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee'::UUID,
    org.id,
    '44444444-4444-4444-4444-444444444444'::UUID, -- code_commit
    '77777777-7777-7777-7777-777777777777'::UUID, -- coding_standard
    'references',
    false
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '44444444-4444-4444-4444-444444444444'::UUID
    AND target_type_id = '77777777-7777-7777-7777-777777777777'::UUID
);

-- Pipeline Run depends on Code Commit (no dirty propagation needed - both Final)
-- This enables query context: pipeline_run → code_commit → work_item → requirement
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'ffffffff-ffff-ffff-ffff-ffffffffffff'::UUID,
    org.id,
    '55555555-5555-5555-5555-555555555555'::UUID, -- pipeline_run
    '44444444-4444-4444-4444-444444444444'::UUID, -- code_commit
    'depends_on',
    false
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '55555555-5555-5555-5555-555555555555'::UUID
    AND target_type_id = '44444444-4444-4444-4444-444444444444'::UUID
);

-- Test Case depends on Code Commit (dirty propagation)
-- When code changes, test case may need update
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'::UUID,
    org.id,
    '66666666-6666-6666-6666-666666666666'::UUID, -- test_case
    '44444444-4444-4444-4444-444444444444'::UUID, -- code_commit
    'depends_on',
    false
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '66666666-6666-6666-6666-666666666666'::UUID
    AND target_type_id = '44444444-4444-4444-4444-444444444444'::UUID
);

-- Test Case depends on Requirement (dirty propagation)
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'bbbbbbbb-cccc-dddd-eeee-ffffffffffff'::UUID,
    org.id,
    '66666666-6666-6666-6666-666666666666'::UUID, -- test_case
    '11111111-1111-1111-1111-111111111111'::UUID, -- requirement
    'depends_on',
    true
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '66666666-6666-6666-6666-666666666666'::UUID
    AND target_type_id = '11111111-1111-1111-1111-111111111111'::UUID
);

-- Design Doc depends on Requirement (dirty propagation)
INSERT INTO dependency_rules (id, organization_id, source_type_id, target_type_id, relationship, is_transitive)
SELECT
    'cccccccc-dddd-eeee-ffff-aaaaaaaaaaaa'::UUID,
    org.id,
    '22222222-2222-2222-2222-222222222222'::UUID, -- design_doc
    '11111111-1111-1111-1111-111111111111'::UUID, -- requirement
    'depends_on',
    true
FROM organizations org
WHERE NOT EXISTS (
    SELECT 1 FROM dependency_rules
    WHERE source_type_id = '22222222-2222-2222-2222-222222222222'::UUID
    AND target_type_id = '11111111-1111-1111-1111-111111111111'::UUID
);

-- ============================================================================
-- Comments
-- ============================================================================

COMMENT ON TABLE dependency_rules IS 'Defines allowed dependencies between asset types and dirty propagation rules';

COMMENT ON COLUMN dependency_rules.relationship IS
    'depends_on: upstream changes propagate dirty to downstream; references: no dirty propagation';

COMMENT ON COLUMN dependency_rules.is_transitive IS
    'If true, included in transitive dependency queries (e.g., commit → work_item → requirement)';
