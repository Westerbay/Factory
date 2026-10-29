-- Retire browser leases and capacity admission without deleting existing records.
-- Keep the legacy columns for migration compatibility; runtime code no longer reads them.
ALTER TABLE factory_identity.environments
    DROP CONSTRAINT environments_check,
    DROP CONSTRAINT environments_check1;

COMMENT ON COLUMN factory_identity.environments.lease_expires_at IS
    'Legacy browser lease; ignored since migration 3. Environments run until explicitly stopped or the account is suspended.';
COMMENT ON COLUMN factory_identity.environments.capacity_reserved IS
    'Legacy capacity reservation; ignored since migration 3. There is no environment count limit.';
