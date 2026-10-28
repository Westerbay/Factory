-- Factory identity is additive; legacy public tables and Drizzle history are untouched.
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
