-- Run in a transaction against the verified Factory database.
SELECT pg_advisory_xact_lock(19742380583174472);
LOCK TABLE factory_identity.schema_info IN ACCESS EXCLUSIVE MODE;
DO $$
BEGIN
    IF (SELECT count(*) = 1 AND bool_and(version = 5) FROM factory_identity.schema_info) IS NOT TRUE THEN
        RAISE EXCEPTION 'Attachments upgrade requires Factory schema version 5';
    END IF;
END $$;

-- Conversation files. Bytes live in the member's volume; only metadata is stored here.
CREATE TABLE factory_identity.conversation_attachments (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES factory_identity.users(id),
    conversation_id uuid NOT NULL REFERENCES factory_identity.conversations(id),
    run_id uuid REFERENCES factory_identity.conversation_runs(id),
    message_id uuid REFERENCES factory_identity.conversation_messages(id),
    source text NOT NULL CHECK (source IN ('user','agent')),
    state text NOT NULL CHECK (state IN ('uploading','staged','attached','unavailable')),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 255 AND octet_length(name) <= 255 AND name !~ '[/[:cntrl:]]' AND name NOT IN ('.','..')),
    media_type text NOT NULL CHECK (length(media_type) BETWEEN 1 AND 255),
    kind text NOT NULL CHECK (kind IN ('image','svg','video','pdf','text','html','file')),
    size_bytes bigint NOT NULL CHECK (size_bytes BETWEEN 0 AND 52428800),
    sha256 text CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    ordinal smallint CHECK (ordinal BETWEEN 0 AND 9),
    unavailable_code text CHECK (length(unavailable_code) BETWEEN 1 AND 64),
    environment_id uuid NOT NULL REFERENCES factory_identity.environments(id),
    environment_generation bigint NOT NULL CHECK (environment_generation > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT conversation_attachment_state_shape CHECK (
        CASE state
            WHEN 'uploading' THEN source = 'user' AND run_id IS NULL AND message_id IS NULL AND ordinal IS NULL
                AND sha256 IS NULL AND unavailable_code IS NULL AND size_bytes = 0
            WHEN 'staged' THEN source = 'user' AND run_id IS NULL AND message_id IS NULL AND ordinal IS NULL
                AND sha256 IS NOT NULL AND unavailable_code IS NULL
            WHEN 'attached' THEN run_id IS NOT NULL AND message_id IS NOT NULL AND ordinal IS NOT NULL
                AND sha256 IS NOT NULL AND unavailable_code IS NULL
            ELSE source = 'agent' AND run_id IS NOT NULL AND message_id IS NOT NULL AND ordinal IS NOT NULL
                AND sha256 IS NULL AND unavailable_code IS NOT NULL AND size_bytes = 0
        END
    )
);
CREATE INDEX conversation_attachments_conversation ON factory_identity.conversation_attachments(conversation_id, created_at, id);
CREATE INDEX conversation_attachments_message ON factory_identity.conversation_attachments(message_id);
CREATE INDEX conversation_attachments_pending ON factory_identity.conversation_attachments(user_id, created_at) WHERE state IN ('uploading','staged');
CREATE UNIQUE INDEX conversation_attachment_message_ordinal ON factory_identity.conversation_attachments(message_id, ordinal) WHERE message_id IS NOT NULL;
CREATE UNIQUE INDEX conversation_attachment_run_ordinal ON factory_identity.conversation_attachments(run_id, ordinal) WHERE source = 'user' AND run_id IS NOT NULL;

ALTER TABLE factory_identity.schema_info DROP CONSTRAINT schema_info_version_check;
UPDATE factory_identity.schema_info SET version = 6;
ALTER TABLE factory_identity.schema_info ADD CONSTRAINT schema_info_version_check CHECK (version = 6);
