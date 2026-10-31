CREATE SCHEMA factory_identity;
CREATE TABLE factory_identity.schema_info (id boolean PRIMARY KEY DEFAULT true CHECK (id), version integer NOT NULL CHECK (version = 1));
INSERT INTO factory_identity.schema_info(id,version) VALUES(true,1);
-- Identity and user access.
CREATE TABLE factory_identity.users (
    id uuid PRIMARY KEY,
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 256),
    role text NOT NULL CHECK (role IN ('member', 'admin')),
    status text NOT NULL CHECK (status IN ('pending', 'active', 'suspended')),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
    access_enabled_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (status <> 'active' OR access_enabled_at IS NOT NULL),
    CHECK (role <> 'admin' OR status = 'active')
);
CREATE UNIQUE INDEX identity_one_admin ON factory_identity.users ((true)) WHERE role = 'admin';
CREATE INDEX identity_users_status_id ON factory_identity.users (status, id);

CREATE TABLE factory_identity.external_identities (
    user_id uuid NOT NULL UNIQUE REFERENCES factory_identity.users(id),
    provider text NOT NULL CHECK (length(provider) BETWEEN 1 AND 32),
    issuer text NOT NULL CHECK (length(issuer) BETWEEN 1 AND 256),
    subject text NOT NULL CHECK (length(subject) BETWEEN 1 AND 256),
    handle text NOT NULL CHECK (length(handle) BETWEEN 1 AND 128),
    PRIMARY KEY (provider, issuer, subject)
);

CREATE TABLE factory_identity.sessions (
    token_hash bytea PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    user_id uuid NOT NULL REFERENCES factory_identity.users(id),
    csrf_token text NOT NULL CHECK (length(csrf_token) BETWEEN 32 AND 256),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX identity_sessions_user ON factory_identity.sessions (user_id);
CREATE INDEX identity_sessions_expiry ON factory_identity.sessions (expires_at);

CREATE TABLE factory_identity.login_attempts (
    state_hash bytea PRIMARY KEY CHECK (octet_length(state_hash) = 32),
    binding_hash bytea NOT NULL CHECK (octet_length(binding_hash) = 32),
    pkce_verifier text NOT NULL CHECK (length(pkce_verifier) BETWEEN 43 AND 128),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX identity_attempts_expiry ON factory_identity.login_attempts (expires_at);

CREATE TABLE factory_identity.access_events (
    id uuid PRIMARY KEY,
    actor_user_id uuid NOT NULL REFERENCES factory_identity.users(id),
    target_user_id uuid NOT NULL REFERENCES factory_identity.users(id),
    prior_status text NOT NULL CHECK (prior_status IN ('pending', 'active', 'suspended')),
    next_status text NOT NULL CHECK (next_status IN ('active', 'suspended')),
    at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- Persistent environment intent.
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
    observed_generation bigint CHECK (observed_generation > 0),
    observed_at timestamptz,
    error_code text CHECK (error_code IN ('unauthorized','conflict','unavailable','invalid_configuration','ownership_conflict','volume_missing','provisioning_uncertain','policy_mismatch','timeout')),
    next_reconcile_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX environments_reconcile ON factory_identity.environments (installation_id,next_reconcile_at);

-- User-owned conversation history.
CREATE TABLE factory_identity.conversations (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES factory_identity.users(id),
    provider_thread text CHECK (length(provider_thread) BETWEEN 1 AND 512),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX conversations_owner ON factory_identity.conversations (user_id, created_at DESC, id);
CREATE TABLE factory_identity.conversation_runs (
    id uuid PRIMARY KEY,
    conversation_id uuid NOT NULL REFERENCES factory_identity.conversations(id),
    operation_id uuid NOT NULL,
    environment_id uuid NOT NULL REFERENCES factory_identity.environments(id),
    environment_generation bigint NOT NULL CHECK (environment_generation > 0),
    model text NOT NULL CHECK (length(model) BETWEEN 1 AND 256),
    input text NOT NULL CHECK (length(input) BETWEEN 1 AND 65536),
    status text NOT NULL CHECK (status IN ('active','completed','failed','interrupted','uncertain')),
    launch_settled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (conversation_id, operation_id)
);
CREATE UNIQUE INDEX conversation_one_active_run ON factory_identity.conversation_runs(conversation_id) WHERE status IN ('active','uncertain');
CREATE TABLE factory_identity.conversation_messages (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES factory_identity.conversation_runs(id),
    role text NOT NULL CHECK (role IN ('user','assistant')),
    content text NOT NULL CHECK (length(content) BETWEEN 1 AND 262144),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    ordinal bigint NOT NULL CHECK (ordinal >= 0),
    provider_item text CHECK (length(provider_item) BETWEEN 1 AND 512),
    CONSTRAINT conversation_message_run_ordinal UNIQUE (run_id,ordinal)
);
CREATE UNIQUE INDEX conversation_message_provider_item ON factory_identity.conversation_messages(run_id,provider_item) WHERE provider_item IS NOT NULL;
CREATE TABLE factory_identity.conversation_requests (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES factory_identity.conversation_runs(id),
    kind text NOT NULL CHECK (kind IN ('permission','question')),
    details text NOT NULL CHECK (length(details) BETWEEN 1 AND 16384),
    provider_reference text NOT NULL CHECK (length(provider_reference) BETWEEN 1 AND 512),
    response_state text NOT NULL DEFAULT 'pending' CHECK (response_state IN ('pending','claimed','delivered','uncertain','expired')),
    response_operation_id uuid,
    response_answer text CHECK (length(response_answer) BETWEEN 1 AND 16384),
    CHECK ((response_state IN ('claimed','delivered','uncertain')) = (response_operation_id IS NOT NULL AND response_answer IS NOT NULL)),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- A reservation precedes every model process; crashes cannot silently release it.
CREATE TABLE factory_identity.model_probe_leases (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL UNIQUE REFERENCES factory_identity.users(id),
    environment_id uuid NOT NULL REFERENCES factory_identity.environments(id),
    environment_generation bigint NOT NULL CHECK (environment_generation > 0),
    launch_settled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE "agents" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"parentId" text,
	CONSTRAINT "agents_runId_id_pk" PRIMARY KEY("runId","id")
);

CREATE TABLE "events" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"sequence" integer NOT NULL,
	"event" jsonb NOT NULL,
	CONSTRAINT "events_runId_id_pk" PRIMARY KEY("runId","id"),
	CONSTRAINT "events_identity" CHECK (("events"."event"->>'id' = "events"."id" and "events"."event"->>'runId' = "events"."runId" and "events"."sequence" > 0) is true)
);

CREATE TABLE "messages" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"agentId" text NOT NULL,
	CONSTRAINT "messages_runId_id_pk" PRIMARY KEY("runId","id")
);

CREATE TABLE "permissions" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"agentId" text NOT NULL,
	CONSTRAINT "permissions_runId_id_pk" PRIMARY KEY("runId","id")
);

CREATE TABLE "questions" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"agentId" text NOT NULL,
	CONSTRAINT "questions_runId_id_pk" PRIMARY KEY("runId","id")
);

CREATE TABLE "runs" (
	"id" text PRIMARY KEY NOT NULL,
	"rootAgentId" text NOT NULL,
	"version" integer NOT NULL,
	"status" text NOT NULL,
	"createdAt" text NOT NULL,
	"projection" jsonb NOT NULL,
	CONSTRAINT "runs_version_positive" CHECK ("runs"."version" > 0),
	CONSTRAINT "runs_projection_identity" CHECK (("runs"."projection"->>'id' = "runs"."id" and ("runs"."projection"->>'version')::integer = "runs"."version" and "runs"."projection"->>'status' = "runs"."status" and "runs"."projection"->>'rootAgentId' = "runs"."rootAgentId") is true),
	CONSTRAINT "runs_status_known" CHECK ("runs"."status" in ('created','running','waiting-for-user','cancelling','completed','failed','interrupted'))
);

ALTER TABLE "agents" ADD CONSTRAINT "agents_runId_runs_id_fk" FOREIGN KEY ("runId") REFERENCES "public"."runs"("id") ON DELETE cascade ON UPDATE no action;
ALTER TABLE "agents" ADD CONSTRAINT "agents_parent_owned" FOREIGN KEY ("runId","parentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;
ALTER TABLE "events" ADD CONSTRAINT "events_runId_runs_id_fk" FOREIGN KEY ("runId") REFERENCES "public"."runs"("id") ON DELETE cascade ON UPDATE no action;
ALTER TABLE "messages" ADD CONSTRAINT "messages_agent_owned" FOREIGN KEY ("runId","agentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;
ALTER TABLE "permissions" ADD CONSTRAINT "permissions_agent_owned" FOREIGN KEY ("runId","agentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;
ALTER TABLE "questions" ADD CONSTRAINT "questions_agent_owned" FOREIGN KEY ("runId","agentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;
CREATE UNIQUE INDEX "events_sequence_unique" ON "events" USING btree ("runId","sequence");
CREATE UNIQUE INDEX "runs_one_active_root" ON "runs" USING btree ((true)) WHERE "runs"."status" in ('created','running','waiting-for-user','cancelling');

-- A projection is inserted before its root agent in the same transaction.
ALTER TABLE "runs" ADD CONSTRAINT "runs_root_owned" FOREIGN KEY ("id", "rootAgentId") REFERENCES "agents"("runId", "id") DEFERRABLE INITIALLY DEFERRED;
