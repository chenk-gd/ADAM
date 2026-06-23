-- Workflow automation tables
-- ============================================================================
-- Implements asset-driven workflow automation: events, promotion rules,
-- workflow instances, actions, agent tasks, approval gates, and the dead
-- letter queue. See docs/plans/2026-06-15-asset-driven-workflow-automation-design.md.
--
-- This migration is purely additive; it does not modify existing tables.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- ---------------------------------------------------------------------------
-- Workflow events: append-only log of domain events that may trigger actions.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS workflow_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    project_id      UUID REFERENCES projects(id) ON DELETE CASCADE,
    correlation_id  UUID NOT NULL,
    event_type      VARCHAR(64) NOT NULL,
    source_asset_id UUID NOT NULL,
    source_asset_type_id UUID NOT NULL REFERENCES asset_types(id) ON DELETE RESTRICT,
    payload         JSONB NOT NULL DEFAULT '{}'::JSONB,
    idempotency_key VARCHAR(255) NOT NULL,
    cascade_depth   INTEGER NOT NULL DEFAULT 0,
    triggering_action_id UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_workflow_events_event_type CHECK (
        event_type IN (
            'asset_published',
            'dirty_resolved',
            'pipeline_failed',
            'action_succeeded',
            'action_failed',
            'approval_granted',
            'approval_rejected'
        )
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_events_idempotency
    ON workflow_events (organization_id, idempotency_key);

CREATE INDEX IF NOT EXISTS idx_workflow_events_correlation
    ON workflow_events (correlation_id);

CREATE INDEX IF NOT EXISTS idx_workflow_events_source_asset
    ON workflow_events (source_asset_id);

-- ---------------------------------------------------------------------------
-- Promotion rules: decide which events create which workflow actions.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS promotion_rules (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id    UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    scope              VARCHAR(16) NOT NULL,
    scope_ref          UUID,
    event_type         VARCHAR(64) NOT NULL,
    source_asset_type_id UUID REFERENCES asset_types(id) ON DELETE RESTRICT,
    mutex_group        VARCHAR(64),
    rule_version       INTEGER NOT NULL DEFAULT 1,
    priority           INTEGER NOT NULL DEFAULT 0,
    automation_level   VARCHAR(32) NOT NULL DEFAULT 'automatic',
    filters            JSONB NOT NULL DEFAULT '{}'::JSONB,
    preconditions      JSONB NOT NULL DEFAULT '[]'::JSONB,
    action_type        VARCHAR(64) NOT NULL,
    action_template    JSONB NOT NULL DEFAULT '{}'::JSONB,
    max_cascade_depth  INTEGER NOT NULL DEFAULT 5,
    effective_from     TIMESTAMPTZ,
    effective_to       TIMESTAMPTZ,
    rollout_segment    INTEGER NOT NULL DEFAULT 100,
    enabled            BOOLEAN NOT NULL DEFAULT TRUE,
    dry_run            BOOLEAN NOT NULL DEFAULT FALSE,
    audit_only         BOOLEAN NOT NULL DEFAULT FALSE,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_promotion_rules_scope CHECK (scope IN ('asset_type', 'project', 'organization')),
    CONSTRAINT chk_promotion_rules_automation_level CHECK (
        automation_level IN ('automatic', 'agent_suggested', 'human_approval_required', 'human_only')
    )
);

CREATE INDEX IF NOT EXISTS idx_promotion_rules_lookup
    ON promotion_rules (organization_id, event_type, enabled);

CREATE INDEX IF NOT EXISTS idx_promotion_rules_mutex
    ON promotion_rules (scope, event_type, mutex_group, rule_version);

-- ---------------------------------------------------------------------------
-- Workflow instances: a Saga coordinator for a chain of actions.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS workflow_instances (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    project_id      UUID REFERENCES projects(id) ON DELETE CASCADE,
    correlation_id  UUID NOT NULL,
    template        VARCHAR(64) NOT NULL,
    status          VARCHAR(32) NOT NULL DEFAULT 'pending',
    cascade_depth   INTEGER NOT NULL DEFAULT 0,
    lock_version    BIGINT NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_workflow_instances_status CHECK (
        status IN (
            'pending', 'ready', 'in_progress', 'blocked',
            'waiting_review', 'waiting_validation',
            'completed', 'failed', 'cancelled'
        )
    )
);

CREATE INDEX IF NOT EXISTS idx_workflow_instances_correlation
    ON workflow_instances (correlation_id);

-- ---------------------------------------------------------------------------
-- Workflow actions: individual steps within an instance.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS workflow_actions (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id          UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    instance_id              UUID NOT NULL REFERENCES workflow_instances(id) ON DELETE CASCADE,
    action_type              VARCHAR(64) NOT NULL,
    target_asset_id          UUID,
    target_asset_type_id     UUID REFERENCES asset_types(id) ON DELETE RESTRICT,
    status                   VARCHAR(32) NOT NULL DEFAULT 'pending',
    lock_version             BIGINT NOT NULL DEFAULT 1,
    idempotency_key          VARCHAR(255) NOT NULL,
    preconditions            JSONB NOT NULL DEFAULT '[]'::JSONB,
    postconditions           JSONB NOT NULL DEFAULT '[]'::JSONB,
    automation_level         VARCHAR(32) NOT NULL DEFAULT 'automatic',
    is_required              BOOLEAN NOT NULL DEFAULT TRUE,
    order_index              INTEGER NOT NULL DEFAULT 0,
    -- Compensation (Saga) fields; populated by Slice 3.
    compensation_action_type VARCHAR(64),
    compensation_payload     JSONB,
    compensation_policy      VARCHAR(32) NOT NULL DEFAULT 'none',
    retry_count              INTEGER NOT NULL DEFAULT 0,
    max_retries              INTEGER NOT NULL DEFAULT 3,
    next_retry_at            TIMESTAMPTZ,
    blocked_reason           VARCHAR(64),
    result_payload           JSONB,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_workflow_actions_status CHECK (
        status IN (
            'pending', 'ready', 'in_progress', 'waiting_approval',
            'blocked', 'succeeded', 'failed', 'cancelled', 'skipped'
        )
    ),
    CONSTRAINT chk_workflow_actions_automation_level CHECK (
        automation_level IN ('automatic', 'agent_suggested', 'human_approval_required', 'human_only')
    ),
    CONSTRAINT chk_workflow_actions_compensation_policy CHECK (
        compensation_policy IN ('none', 'best_effort', 'required_before_fail', 'manual_only')
    ),
    CONSTRAINT chk_workflow_actions_blocked_reason CHECK (
        blocked_reason IS NULL OR blocked_reason IN (
            'missing_dependency', 'dirty_dependency', 'waiting_approval',
            'pipeline_failed', 'policy_denied', 'executor_unavailable',
            'external_system_unavailable', 'waiting_manual_intervention'
        )
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_actions_idempotency
    ON workflow_actions (organization_id, idempotency_key);

CREATE INDEX IF NOT EXISTS idx_workflow_actions_instance
    ON workflow_actions (instance_id);

CREATE INDEX IF NOT EXISTS idx_workflow_actions_status_target
    ON workflow_actions (status, target_asset_id);

-- ---------------------------------------------------------------------------
-- Agent tasks: executable units claimed by AI agents.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS agent_tasks (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id      UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    project_id           UUID REFERENCES projects(id) ON DELETE CASCADE,
    action_id            UUID NOT NULL REFERENCES workflow_actions(id) ON DELETE CASCADE,
    capability           VARCHAR(64) NOT NULL,
    status               VARCHAR(32) NOT NULL DEFAULT 'queued',
    agent_id             VARCHAR(255),
    claimed_at           TIMESTAMPTZ,
    expires_at           TIMESTAMPTZ,
    result_payload       JSONB,
    produced_asset_ids   JSONB NOT NULL DEFAULT '[]'::JSONB,
    lock_version         BIGINT NOT NULL DEFAULT 1,
    idempotency_key      VARCHAR(255) NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_agent_tasks_status CHECK (
        status IN ('queued', 'claimed', 'running', 'succeeded', 'failed', 'cancelled', 'expired')
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_tasks_idempotency
    ON agent_tasks (action_id, idempotency_key);

CREATE INDEX IF NOT EXISTS idx_agent_tasks_status_capability
    ON agent_tasks (status, capability);

-- ---------------------------------------------------------------------------
-- Approval gates: human authorization checkpoints for actions.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS approval_gates (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    action_id       UUID NOT NULL REFERENCES workflow_actions(id) ON DELETE CASCADE,
    approver_type   VARCHAR(16) NOT NULL,
    approver_ref    VARCHAR(255) NOT NULL,
    status          VARCHAR(16) NOT NULL DEFAULT 'pending',
    decision_payload JSONB,
    deadline        TIMESTAMPTZ,
    decided_by      VARCHAR(255),
    decided_at      TIMESTAMPTZ,
    lock_version    BIGINT NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_approval_gates_approver_type CHECK (
        approver_type IN ('role', 'user', 'group')
    ),
    CONSTRAINT chk_approval_gates_status CHECK (
        status IN ('pending', 'approved', 'rejected', 'expired', 'cancelled')
    )
);

CREATE INDEX IF NOT EXISTS idx_approval_gates_action
    ON approval_gates (action_id);

CREATE INDEX IF NOT EXISTS idx_approval_gates_status
    ON approval_gates (organization_id, status);

-- ---------------------------------------------------------------------------
-- Dead letter queue: events/actions that exhausted retries.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS workflow_dead_letters (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    project_id      UUID REFERENCES projects(id) ON DELETE CASCADE,
    source_type     VARCHAR(32) NOT NULL,
    source_id       UUID NOT NULL,
    reason          VARCHAR(64) NOT NULL,
    context         JSONB NOT NULL DEFAULT '{}'::JSONB,
    status          VARCHAR(16) NOT NULL DEFAULT 'open',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at     TIMESTAMPTZ,
    CONSTRAINT chk_workflow_dead_letters_source_type CHECK (
        source_type IN ('event', 'action', 'instance')
    ),
    CONSTRAINT chk_workflow_dead_letters_status CHECK (
        status IN ('open', 'assigned', 'replayed', 'resolved', 'ignored')
    )
);

CREATE INDEX IF NOT EXISTS idx_workflow_dead_letters_status
    ON workflow_dead_letters (organization_id, status);

-- ---------------------------------------------------------------------------
-- First-slice promotion rule: requirement publish creates/updates a feature
-- work item. Seeded idempotently per organization.
-- ---------------------------------------------------------------------------
INSERT INTO promotion_rules (
    id, organization_id, scope, event_type, source_asset_type_id,
    automation_level, action_type, action_template, enabled, created_at, updated_at
)
SELECT
    gen_random_uuid(), org.id, 'organization', 'asset_published', req_type.id,
    'automatic', 'upsert_work_item',
    jsonb_build_object(
        'work_item_kind', 'feature',
        'field_mapping', jsonb_build_object(
            'title', 'payload.title',
            'description', 'payload.description',
            'source_requirement_id', 'source_asset_id'
        )
    ),
    TRUE, NOW(), NOW()
FROM organizations org
JOIN asset_types req_type ON req_type.organization_id = org.id AND req_type.name = 'requirement'
WHERE NOT EXISTS (
    SELECT 1 FROM promotion_rules existing
    WHERE existing.organization_id = org.id
      AND existing.event_type = 'asset_published'
      AND existing.source_asset_type_id = req_type.id
      AND existing.action_type = 'upsert_work_item'
      AND existing.scope = 'organization'
);

COMMENT ON TABLE workflow_events IS 'Append-only log of domain events that drive workflow automation';
COMMENT ON TABLE promotion_rules IS 'Rules deciding which events create which workflow actions';
COMMENT ON TABLE workflow_instances IS 'Saga coordinator for a chain of workflow actions';
COMMENT ON TABLE workflow_actions IS 'Individual steps within a workflow instance';
COMMENT ON TABLE agent_tasks IS 'Executable units claimed by AI agents';
COMMENT ON TABLE approval_gates IS 'Human authorization checkpoints for actions';
COMMENT ON TABLE workflow_dead_letters IS 'Events/actions that exhausted retries, pending operator review';
