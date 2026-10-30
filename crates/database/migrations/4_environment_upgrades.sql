CREATE TABLE factory_identity.environment_upgrades (
    environment_id uuid PRIMARY KEY REFERENCES factory_identity.environments(id) ON DELETE CASCADE,
    operation_id uuid NOT NULL UNIQUE,
    generation bigint NOT NULL CHECK (generation > 0),
    old_container_id text NOT NULL CHECK (old_container_id ~ '^[0-9a-f]{64}$'),
    target_image_id text NOT NULL CHECK (target_image_id ~ '^sha256:[0-9a-f]{64}$'),
    target_policy_version text NOT NULL CHECK (length(target_policy_version) BETWEEN 1 AND 64),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
