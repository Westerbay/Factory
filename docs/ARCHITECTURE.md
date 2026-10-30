# Architecture

## Runtime and dependency direction

Factory has two independently served surfaces: the static React/TypeScript application
in `apps/web` and the Axum API in `apps/api`. The browser calls an explicit API origin
with credentials. Rust does not serve frontend files, and the built frontend requires
no Node application server. Vite development and preview are local tooling.

| Owner                   | Responsibility                                               | Internal dependency direction                 |
| ----------------------- | ------------------------------------------------------------ | --------------------------------------------- |
| `crates/core`           | Identity/environment types, policies and outbound ports      | No application or infrastructure dependencies |
| `crates/application`    | Identity use cases and environment reconciliation            | Core                                          |
| `crates/adapters`       | GitHub identity and local Docker runtime                     | Core                                          |
| `crates/database`       | SQLx identity/environment persistence and verified targets   | Core                                          |
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
including the administrator, must start after activation.

Mutations require the exact frontend Origin and the session CSRF token. Body mutations
also require JSON. Credentialed CORS uses an exact allowlist and is not an authorization
mechanism. Member requests to administrator endpoints are rejected by the server even
when a crafted client bypasses the frontend route guard.

Access changes lock and revalidate actor/target state, compare the expected version, and
commit the transition, session revocation and environment stop intent atomically.
Suspension records a stopped environment intent without calling Docker inside the transaction.
The administrator cannot be
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

## Local environment lifecycle

[Core environment ports](../crates/core/src/environments/ports.rs) separate persistent
intent, runtime observations and supervisor ownership. The
[application service](../crates/application/src/environments/service.rs) handles self-only
requests; the [reconciler](../crates/application/src/environments/reconcile.rs) performs
bounded runtime operations outside database transactions. API composition owns its
lifetime. JavaScript [preparation tooling](../scripts/environments/prepare.mjs) builds
the common image explicitly; user operations call the Rust
[Bollard adapter](../crates/adapters/src/environments/runtime.rs) directly. Neither API
startup nor a start request builds or pulls images.

The [environment store](../crates/database/src/environments/mod.rs) shares the verified
IdentityDatabase without exposing its SQLx pool. The additive
[migration](../crates/database/migrations/2_environments.sql) records one environment
per user, installation/image/policy identity, desired and observed states, generation,
provisioning phase and persistent running intent. The follow-up
[migration](../crates/database/migrations/3_persistent_environments.sql) retires the old
lease/capacity constraints without deleting existing records; legacy columns are no longer
read or written by the runtime. Requests revalidate the session and active user under a
consistent user-then-environment lock order. Observation writes match the generation so a
stale operation cannot overwrite a newer intent. There is no global capacity admission.

Provisioning records intent before volume creation and confirms the volume before
starting the container. A previously confirmed volume that disappears is an error;
ambiguous interrupted creation requires operator investigation. Neither case silently
replaces user data. Resource names and labels bind installation, environment, user and
policy. The adapter checks ownership before mutation and complete image/mount/isolation
policy before start. Foreign resources are never adopted. See the
[policy owner](../crates/adapters/src/environments/policy.rs) for enforced isolation and unlimited CPU/memory/process settings.

The host retains an installation file lock and a dedicated PostgreSQL advisory-lock
connection. It monitors the database guard every second and checks it around runtime
calls. Lock loss makes environment operations unavailable until API restart; it does
not silently reacquire ownership. Individual Docker errors remain observable and are
retried after 15 seconds. The supervisor also repairs suspended-user intent independently of Docker work. The host serializes work per environment and runs
up to 32 environment jobs concurrently, so a slow container does not block another
user's stop when worker capacity is available. This is bounded supervision, not a
real-time shutdown guarantee under saturation. API shutdown stops supervision while containers keep running; API restart resumes
reconciliation. No independent environment supervisor runs outside the API.

[Environment routes](../apps/api/src/environments/routes.rs) accept no target user ID or
Docker configuration. GET reads status without creating resources; POST start/stop return
accepted persistent intent. There is no heartbeat endpoint.
All require an active session and mutations require Origin/CSRF checks. Public responses
contain status, observation time, an upgrade-required flag and bounded error codes,
never Docker identifiers. Image/policy mismatch blocks Start and interactive use while
Stop remains useful.
The [member controller](../apps/web/src/features/environments/environment-controller.ts)
polls only while visible, online and authenticated. Session loss, logout and unmount
cancel browser requests without changing server intent or stopping the container.

Missing preparation affects only environments when identity is otherwise ready. In
contrast, a pending SQLx migration leaves identity readiness unavailable as before.
Prepared metadata keeps the installation UUID and immutable image ID under
`.factory/environments`; preparing a newer image does not migrate existing environments.
Linux socket and Windows named-pipe transports ignore ambient Docker contexts and remote
daemon settings. The API is trusted to access Docker; the browser and containers are not
given the socket. User containers run non-root with a read-only root and a persistent
home volume. CPU, memory and process counts have no
Factory-imposed ceiling; temporary storage and logs remain bounded. Named volumes do not supply a portable disk quota.

The [operator upgrade service](../crates/application/src/environments/upgrade.rs) and
[upgrade store](../crates/database/src/environments/upgrade.rs) reserve an additive
[durable upgrade intent](../crates/database/migrations/4_environment_upgrades.sql)
before removing an owned stopped container. The CLI requires both installation and
database supervisor locks, so the API must be stopped. Preview and confirmation bind
the user, generation, old container ID and target image/policy. Only the old container
is removed, never its volume. Finalization retains Stopped intent and atomically consumes
the reservation. Start refuses pending reservations. Home identity labels remain stable
across image upgrades; container labels still bind the exact image and policy.

## Isolated egress

[DockerEgress](../crates/adapters/src/egress/runtime.rs) owns one shared proxy and one
internal bridge per user, using Docker's isolated gateway mode and disabled IPv6. User
containers have no external default route or published ports. The runtime checks exact
network membership and ownership before interactive exec. Only the proxy joins both
the user's bridge and the installation's outbound bridge; IP forwarding is disabled.

The pinned [Squid policy](../infra/egress/squid.conf) permits CONNECT on port 443 only for
`auth.openai.com`, `chatgpt.com` and `api.openai.com`. DNS answers and actual destination
addresses are both checked against private/reserved ranges. The proxy entrypoint installs
its nftables policy, then drops privilege and all capabilities before serving requests.
There is no TLS interception or tunnel-content logging. Unsupported engine/network policy
fails closed. Proxy failure affects Codex availability without disabling the local shell;
status reads never start the proxy or user environment.

## Terminal and account operations

[TerminalService](../crates/application/src/terminals/mod.rs) owns one-use, 30-second
attachment tickets bound to user, session, environment generation and takeover decision.
HTTP ticket creation requires Origin and CSRF. WebSocket upgrade requires the session,
exact Origin and fixed subprotocol; the ticket travels in the first message, never the URL.
Input and resize requests revalidate access. Passive streams check at most every five
seconds and expire at the exact session deadline; local logout/suspension notifications
close them promptly. An in-flight Docker attachment is settled and detached after
revocation rather than abandoning its handle.

The [Docker terminal adapter](../crates/adapters/src/terminals/mod.rs) attaches a fixed
non-root tmux command. tmux owns shell lifetime; the API owns only the attached client.
The stream uses 16-KiB output chunks and a 256-KiB unacknowledged window. xterm acknowledges
only after processing output; a slow client is detached. Input/output buffers are bounded,
and reconnect never replays input. Explicit takeover invalidates the previous attachment
generation so its cleanup cannot detach a later writer.

[AgentConnectionService](../crates/application/src/agent_connections/mod.rs) owns account
operations through a provider-neutral port. The [Codex adapter](../crates/adapters/src/codex_account/mod.rs)
maps the pinned 0.160.0 app-server JSON-RPC account protocol. It validates IDs, message size,
depth, account projections and the official authorization URL. Device authorization is
session/generation-bound; success requires the completion notification plus a confirmed
account read. The API monitors expiry and revocation even after the initiating HTTP
handler disappears. No account operation issues `turn/start`.

Account controllers run under a fixed image wrapper with a single-controller lock,
private file permissions and a watchdog shorter than ten minutes. Cleanup shuts down
stdin and drops both Docker stream halves before waiting for exit. Credentials stay in
`/home/factory/.codex`; only a bounded account projection and pending device challenge
cross HTTP. Cancellation/logout remain local when the proxy is stopped. Browser account
polling runs only for an active pending login and visible, authenticated page. The route
shares one environment controller between the lifecycle card and dependent panels.

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
