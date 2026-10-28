# Architecture

## Runtime and dependency direction

Factory has two independently served surfaces: the static React/TypeScript application
in `apps/web` and the Axum API in `apps/api`. The browser calls an explicit API origin
with credentials. Rust does not serve frontend files, and the built frontend requires
no Node application server. Vite development and preview are local tooling.

| Owner                   | Responsibility                                               | Internal dependency direction                 |
| ----------------------- | ------------------------------------------------------------ | --------------------------------------------- |
| `crates/core`           | Identity types, policies and outbound ports                  | No application or infrastructure dependencies |
| `crates/application`    | Identity use cases                                           | Core                                          |
| `crates/adapters`       | GitHub identity provider                                     | Core                                          |
| `crates/database`       | SQLx identity persistence and verified local targets         | Core                                          |
| `apps/api`              | Composition, configuration, HTTP, cookies, CORS and OpenAPI  | Application, core and concrete adapters       |
| `apps/web/src/lib`      | Browser transport, session lifecycle and query configuration | Browser code and generated API types          |
| `apps/web/src/features` | Member and administrator interfaces                          | Shared browser libraries and components       |

Cargo and TypeScript boundary checks enforce dependency direction, including type-only
imports. Browser code must not import Rust implementation details, Node, filesystem,
execution application code or persistence adapters. Feature modules do not import one
another. Shared API and session mechanisms support both member and administrator screens.

## Identity and HTTP contracts

[Identity policy](../crates/core/src/identity/policy.rs) owns admission and access-change
rules. [IdentityService](../crates/application/src/identity/mod.rs) orchestrates the
provider and store ports without depending on Axum, SQLx or GitHub transport.
The API composes their concrete implementations at startup.
Its [configuration loader](../apps/api/src/config/local.rs) reads the working
directory's `.env` without mutating process variables; explicit process values win.
API startup and the operator CLI load this file. Tests and contract export use inert configuration,
and the static frontend never receives server credentials.

The [GitHub adapter](../crates/adapters/src/identity/github.rs) performs the provider
exchange and returns the stable provider/issuer/subject identity. Handles and display
names are presentation data. Provider tokens are used only for identity lookup, then
discarded; no repository permissions or provider-token persistence are implemented.
OAuth state, browser binding and PKCE protect the single-use login attempt.

Rust [HTTP contracts](../apps/api/src/contracts/mod.rs) and handler annotations produce
[the API description](../apps/api/openapi.json) and
[generated browser types](../apps/web/src/lib/api/schema.d.ts).
Generate both with `pnpm api:generate`; check parity with `pnpm api:contract`.
Export is inert and does not start the server, open a database or contact GitHub.
The [shared router composition](../apps/api/src/app.rs) uses `utoipa-axum` to register
Axum handlers and collect their OpenAPI operations in one step. Each handler owns its
path, method, responses and security annotations; request parameters derive from the
actual query structs. [OpenAPI export](../apps/api/src/openapi/mod.rs) only adds document
metadata and the shared session cookie scheme. Unconfigured identity uses the same
routes and rejects requests with `503` without constructing services.
The frontend validates security-relevant response shapes at its boundary while consuming
these generated transport types.

## Sessions and access changes

The server issues opaque HttpOnly, SameSite=Lax session cookies. Local HTTP does not use
the Secure attribute; this loopback configuration is not a production cookie policy.
Session credentials are hashed in PostgreSQL. Each protected request checks the current
session and active user state; fixed expiry, logout or suspension ends admission.
Unknown verified users are pending. The [operator CLI](../apps/api/src/bin/factory.rs)
resolves a public GitHub username without OAuth credentials, confirms the selected
profile, and calls [administrator setup](../crates/database/src/identity/setup.rs) on
the verified owned database. Setup serializes the one-admin decision, shares the
identity lock with login, and creates or promotes the selected account atomically.
It refuses all writes once an administrator exists and never issues a session.
The existing unique administrator index remains the final concurrency guard.
This operator capability is absent from the HTTP store port. Every subsequent login,
including the administrator, must start after activation. No schema migration is needed.

Mutations require the exact frontend Origin and the session CSRF token. Body mutations
also require JSON. Credentialed CORS uses an exact allowlist and is not an authorization
mechanism. Member requests to administrator endpoints are rejected by the server even
when a crafted client bypasses the frontend route guard.

Access changes lock and revalidate actor/target state, compare the expected version, and
commit the transition and session revocation atomically. The administrator cannot be
edited through member-access operations. Reactivation sets a new admission boundary;
only OAuth attempts begun after that boundary can create a new session. Stale writes
return a conflict for explicit review, never an automatic retry.

The [browser session controller](../apps/web/src/lib/auth/session-controller.ts) keeps
verified session state in memory. Route changes, focus, reconnection and expiry trigger
verification or invalidation. Unauthorized/forbidden responses, logout, account changes
and offline transitions clear private query data. BroadcastChannel carries invalidation
only, not session details. Locally observed authorization refusals notify other tabs;
received invalidations hide private content and revalidate without broadcasting again.
Protected routes hide stale content while verification fails
or remains pending; an unconfirmed logout cannot silently restore the session.

## Persistence and local ownership

[IdentityDatabase](../crates/database/src/identity/mod.rs) accepts only a verified target.
It does not consume an arbitrary database URL, read credential files or apply migrations
on connection. Startup and readiness inspect state without DDL. Explicit migration uses
[the SQLx lifecycle](../crates/database/src/migrations/mod.rs) and a dedicated connection.

Identity objects and their migration ledger live in `factory_identity`. Legacy
execution tables and the Drizzle ledger remain separate and untouched. No historical
execution is assigned to a web user, and the new API exposes no execution endpoints.

[Local target verification](../crates/database/src/local_target/mod.rs) checks the owned
Docker project, checkout, owner marker, service, volume, credentials and published
loopback port before opening SQL. Linux uses the local socket; Windows uses Docker
Desktop's local named pipe. Arbitrary contexts, remote targets and a loopback URL without
resource ownership are insufficient. See [development](DEVELOPMENT.md#local-postgresql).

## PWA and static hosting

[The service worker](../apps/web/src/pwa/sw.ts) precaches only public JavaScript, CSS,
icons and the offline page. Navigations use the network with a public offline fallback.
Private HTML, API responses, OAuth exchanges and foreign-origin requests are never cached
by the worker. Session data remains outside CacheStorage, localStorage, sessionStorage
and IndexedDB.

[Update registration](../apps/web/src/pwa/use-app-update.ts) checks on focus, online
transitions and a periodic interval. A waiting worker activates after explicit user
action, and only the consenting tab reloads automatically. An already installed client
is not evidence that the API or a session is currently available.

A static host must serve SPA navigation fallbacks and fresh entry/worker/manifest
resources; hashed assets can be immutable. The API remains a separate origin with exact
credentialed CORS. Production origins, TLS, reverse proxies and deployment are separate
work; current configuration intentionally accepts only local HTTP.

## Preserved TypeScript execution reference

The `packages/core`, `packages/application`, `packages/contracts`,
`packages/adapters` and `packages/database` boundaries remain independent of the web
identity runtime. Core owns provider-neutral models, projections and ports; application
owns execution orchestration; concrete adapters implement Codex and PostgreSQL behavior.
These packages are references for a later port, not a second live web backend.

`packages/execution-reference` retains only the Electron-free host, message relay,
client and UI model/view closure needed by the existing behavior tests. It has no
product exports, executable entry point or consumer in the web runtime. Boundary
checks prohibit importing it from product code. Electron window, preload, native
process composition and launch tooling have been removed.

Execution reserves one global active root before provider effects and commits material
events atomically with versioned projections. Repeated identical events are idempotent;
conflicting writes cannot create partial history. Completed messages are durable,
stream fragments transient, and reads detached. Explicit recovery interrupts abandoned
runs without replaying tools. Connection creation and readiness never recover or migrate.

Question submission, provider permissions, active steering and terminal continuation are
distinct operations. Continuation creates a linked run. Cancellation requires terminal
confirmation for owned native threads; disconnect or process death alone cannot prove
cleanup. Unconfirmed ownership blocks new execution even if a recorded run is interrupted.

The pinned Codex adapter keeps provider IDs and request correlation private. Generated
protocol bindings preserve provenance, unsupported actionable requests fail visibly, and
conversation correlation is currently session-local. These constraints and the absence of
per-user ownership must be resolved before exposing execution through the web API.
