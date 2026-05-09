-- Database triggers and constraint functions
-- ============================================================================
-- These triggers enforce business rules at the database level

-- Function to check cross-organization access (BR-008)
CREATE OR REPLACE FUNCTION check_dependencies_same_org()
RETURNS TRIGGER AS $$
BEGIN
    -- Check if source and target are in the same organization
    IF (SELECT organization_id FROM asset_instances WHERE id = NEW.source_id) !=
       (SELECT organization_id FROM asset_instances WHERE id = NEW.target_id) THEN
        RAISE EXCEPTION 'Cross-organization dependency not allowed';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Function to check dependency boundary rules
CREATE OR REPLACE FUNCTION check_dependency_boundary()
RETURNS TRIGGER AS $$
DECLARE
    source_level VARCHAR(20);
    target_level VARCHAR(20);
    source_project UUID;
    target_project UUID;
BEGIN
    -- Get source and target levels and projects
    SELECT level, project_id INTO source_level, source_project
    FROM asset_instances WHERE id = NEW.source_id;

    SELECT level, project_id INTO target_level, target_project
    FROM asset_instances WHERE id = NEW.target_id;

    -- Rule 1: Project-level asset cannot depend on organization-level asset
    IF source_level = 'project' AND target_level = 'organization' THEN
        RAISE EXCEPTION 'Project-level asset cannot depend on organization-level asset';
    END IF;

    -- Rule 2: Organization-level asset can only depend on organization-level assets
    IF source_level = 'organization' AND target_level = 'project' THEN
        RAISE EXCEPTION 'Organization-level asset can only depend on organization-level assets';
    END IF;

    -- Rule 3: Cross-project dependency not allowed (both must be in same project or both org-level)
    IF source_level = 'project' AND target_level = 'project' AND source_project != target_project THEN
        RAISE EXCEPTION 'Cross-project dependency not allowed';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Function to check asset type organization consistency
CREATE OR REPLACE FUNCTION check_asset_instance_type_org()
RETURNS TRIGGER AS $$
DECLARE
    type_org UUID;
BEGIN
    SELECT organization_id INTO type_org FROM asset_types WHERE id = NEW.type_id;
    IF type_org != NEW.organization_id THEN
        RAISE EXCEPTION 'Asset type must belong to the same organization as the asset instance';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Function to check project organization consistency
CREATE OR REPLACE FUNCTION check_asset_instance_project_org()
RETURNS TRIGGER AS $$
DECLARE
    project_org UUID;
BEGIN
    IF NEW.project_id IS NOT NULL THEN
        SELECT organization_id INTO project_org FROM projects WHERE id = NEW.project_id;
        IF project_org != NEW.organization_id THEN
            RAISE EXCEPTION 'Project must belong to the same organization as the asset instance';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Function to detect cycles in dependency graph (BR-006)
CREATE OR REPLACE FUNCTION check_dependency_cycle()
RETURNS TRIGGER AS $$
DECLARE
    path UUID[];
    current_id UUID;
BEGIN
    -- Use recursive CTE to detect if adding this dependency would create a cycle
    WITH RECURSIVE dependency_path AS (
        -- Start from the target (upstream)
        SELECT target_id AS node, ARRAY[NEW.source_id, NEW.target_id] AS path
        FROM asset_dependencies
        WHERE source_id = NEW.target_id

        UNION ALL

        -- Follow the chain
        SELECT ad.target_id, dp.path || ad.target_id
        FROM asset_dependencies ad
        JOIN dependency_path dp ON ad.source_id = dp.node
        WHERE NOT ad.target_id = ANY(dp.path)  -- Prevent infinite loop
    )
    SELECT dp.path INTO path
    FROM dependency_path dp
    WHERE dp.node = NEW.source_id  -- If we can reach back to source, it's a cycle
    LIMIT 1;

    IF FOUND THEN
        RAISE EXCEPTION 'BR-006: Cycle detected in dependency graph: %', path;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Create constraint triggers
-- These are deferred triggers that run at transaction commit time

-- Trigger for cross-organization dependency check
CREATE CONSTRAINT TRIGGER trg_dependencies_same_org
    AFTER INSERT OR UPDATE ON asset_dependencies
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_dependencies_same_org();

-- Trigger for dependency boundary rules
CREATE CONSTRAINT TRIGGER trg_dependency_boundary
    AFTER INSERT OR UPDATE ON asset_dependencies
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_dependency_boundary();

-- Trigger for cycle detection
CREATE CONSTRAINT TRIGGER trg_dependency_cycle
    AFTER INSERT OR UPDATE ON asset_dependencies
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_dependency_cycle();

-- Trigger for asset type organization consistency
CREATE CONSTRAINT TRIGGER trg_asset_instances_type_org_consistency
    AFTER INSERT OR UPDATE ON asset_instances
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_asset_instance_type_org();

-- Trigger for project organization consistency
CREATE CONSTRAINT TRIGGER trg_asset_instances_project_org_consistency
    AFTER INSERT OR UPDATE ON asset_instances
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_asset_instance_project_org();
