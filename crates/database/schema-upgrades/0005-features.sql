-- Run in a transaction against the verified Factory database.
SELECT pg_advisory_xact_lock(19742380583174472);
LOCK TABLE factory_identity.schema_info IN ACCESS EXCLUSIVE MODE;
DO $$
BEGIN
    IF (SELECT count(*) = 1 AND bool_and(version = 4) FROM factory_identity.schema_info) IS NOT TRUE THEN
        RAISE EXCEPTION 'Features upgrade requires Factory schema version 4';
    END IF;
END $$;

-- Product workflow records are separate from existing conversation history.
CREATE TABLE factory_identity.features (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES factory_identity.users(id),
    project_id uuid NOT NULL,
    conversation_id uuid NOT NULL UNIQUE REFERENCES factory_identity.conversations(id) ON DELETE CASCADE,
    environment_id uuid NOT NULL,
    create_operation_id uuid NOT NULL,
    create_request jsonb NOT NULL CHECK (jsonb_typeof(create_request) = 'object'),
    provider text NOT NULL CHECK (length(provider) BETWEEN 1 AND 64),
    workflow_revision text NOT NULL CHECK (length(workflow_revision) BETWEEN 1 AND 128),
    title text NOT NULL CHECK (length(title) BETWEEN 1 AND 256),
    stage text NOT NULL CHECK (stage IN ('scope', 'architecture', 'execution', 'review', 'result', 'draft')),
    scope jsonb CHECK (jsonb_typeof(scope) = 'object'),
    architecture jsonb CHECK (jsonb_typeof(architecture) = 'object'),
    current_review uuid,
    additional_review_rounds bigint NOT NULL DEFAULT 0 CHECK (additional_review_rounds BETWEEN 0 AND 4294967295),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (user_id, create_operation_id),
    FOREIGN KEY (project_id, user_id) REFERENCES factory_identity.projects(id, user_id),
    FOREIGN KEY (environment_id, user_id) REFERENCES factory_identity.environments(id, user_id)
);
CREATE INDEX features_owner ON factory_identity.features(user_id, created_at DESC, id DESC);
CREATE INDEX features_project ON factory_identity.features(project_id, created_at DESC, id DESC);

CREATE TABLE factory_identity.feature_revisions (
    feature_id uuid NOT NULL REFERENCES factory_identity.features(id) ON DELETE CASCADE,
    id uuid NOT NULL,
    position bigint NOT NULL CHECK (position >= 0),
    record jsonb NOT NULL CHECK (jsonb_typeof(record) = 'object'),
    PRIMARY KEY (feature_id, id),
    UNIQUE (feature_id, position)
);

CREATE TABLE factory_identity.feature_attempts (
    feature_id uuid NOT NULL REFERENCES factory_identity.features(id) ON DELETE CASCADE,
    id uuid NOT NULL,
    position bigint NOT NULL CHECK (position >= 0),
    reserved boolean NOT NULL,
    record jsonb NOT NULL CHECK (jsonb_typeof(record) = 'object'),
    PRIMARY KEY (feature_id, id),
    UNIQUE (feature_id, position)
);
CREATE INDEX feature_attempts_reserved ON factory_identity.feature_attempts(feature_id) WHERE reserved;

CREATE TABLE factory_identity.feature_reviews (
    feature_id uuid NOT NULL REFERENCES factory_identity.features(id) ON DELETE CASCADE,
    id uuid NOT NULL,
    position bigint NOT NULL CHECK (position >= 0),
    record jsonb NOT NULL CHECK (jsonb_typeof(record) = 'object'),
    PRIMARY KEY (feature_id, id),
    UNIQUE (feature_id, position)
);

CREATE TABLE factory_identity.feature_deliveries (
    feature_id uuid NOT NULL REFERENCES factory_identity.features(id) ON DELETE CASCADE,
    id uuid NOT NULL,
    position bigint NOT NULL CHECK (position >= 0),
    unsettled boolean NOT NULL,
    record jsonb NOT NULL CHECK (jsonb_typeof(record) = 'object'),
    PRIMARY KEY (feature_id, id),
    UNIQUE (feature_id, position)
);
CREATE INDEX feature_deliveries_unsettled ON factory_identity.feature_deliveries(feature_id) WHERE unsettled;

CREATE TABLE factory_identity.feature_operations (
    feature_id uuid NOT NULL REFERENCES factory_identity.features(id) ON DELETE CASCADE,
    id uuid NOT NULL,
    command jsonb NOT NULL CHECK (jsonb_typeof(command) = 'object'),
    PRIMARY KEY (feature_id, id)
);

ALTER TABLE factory_identity.schema_info DROP CONSTRAINT schema_info_version_check;
UPDATE factory_identity.schema_info SET version = 5;
ALTER TABLE factory_identity.schema_info ADD CONSTRAINT schema_info_version_check CHECK (version = 5);
