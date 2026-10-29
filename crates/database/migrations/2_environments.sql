-- Additive environment intent; identity and legacy execution records remain intact.
CREATE TABLE factory_identity.environments (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL UNIQUE REFERENCES factory_identity.users(id),
    installation_id uuid NOT NULL,
    desired_state text NOT NULL CHECK (desired_state IN ('stopped','running')),
    observed_state text NOT NULL CHECK (observed_state IN ('not_created','starting','running','stopping','stopped','error','unavailable')),
    generation bigint NOT NULL DEFAULT 1 CHECK (generation > 0),
    provisioning_state text NOT NULL DEFAULT 'unprovisioned' CHECK (provisioning_state IN ('unprovisioned','volume_creating','volume_ready')),
    volume_name text NOT NULL UNIQUE,
    container_name text NOT NULL UNIQUE,
    image_id text NOT NULL CHECK (length(image_id) BETWEEN 1 AND 256),
    policy_version text NOT NULL CHECK (length(policy_version) BETWEEN 1 AND 64),
    capacity_reserved boolean NOT NULL DEFAULT false,
    lease_expires_at timestamptz,
    observed_at timestamptz,
    error_code text CHECK (error_code IN ('unauthorized','capacity_reached','conflict','unavailable','invalid_configuration','ownership_conflict','volume_missing','provisioning_uncertain','policy_mismatch','timeout')),
    next_reconcile_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (desired_state <> 'running' OR (lease_expires_at IS NOT NULL AND capacity_reserved)),
    CHECK (desired_state <> 'stopped' OR lease_expires_at IS NULL)
);
CREATE INDEX environments_reconcile ON factory_identity.environments (installation_id,next_reconcile_at);
CREATE INDEX environments_capacity ON factory_identity.environments (installation_id) WHERE capacity_reserved;
