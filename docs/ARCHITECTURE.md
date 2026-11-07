# Architecture

## Runtime and dependency direction

Factory has two independently served surfaces: the static React/TypeScript application
in `apps/web` and the Axum API in `apps/api`. The browser calls an explicit API origin
with credentials. Rust does not serve frontend files, and the built frontend requires
no Node application server. Vite development and preview are local tooling.

| Owner                   | Responsibility                                                  | Internal dependency direction                 |
| ----------------------- | --------------------------------------------------------------- | --------------------------------------------- |
| `crates/core`           | Identity, environment and conversation types and ports          | No application or infrastructure dependencies |
| `crates/application`    | Identity and conversation use cases, environment reconciliation | Core                                          |
| `crates/adapters`       | GitHub, agent providers and local Docker adapters               | Core                                          |
| `crates/database`       | SQLx identity, environment and conversation persistence         | Core                                          |
| `apps/api`              | Composition, configuration, HTTP, cookies, CORS and OpenAPI     | Application, core and concrete adapters       |
| `apps/web/src/lib`      | Browser transport, session lifecycle and query configuration    | Browser code and generated API types          |
| `apps/web/src/features` | Member and administrator interfaces                             | Shared browser libraries and components       |

Cargo and TypeScript boundary checks enforce dependency direction, including type-only
imports. Browser code must not import Rust implementation details, Node, filesystem,
execution application code or persistence adapters. Feature modules do not import one
another. Shared API and session mechanisms support both member and administrator screens.

## Browser composition and presentation

The [workspace route](../apps/web/src/routes/_authenticated.app.tsx) delegates to
[WorkspacePage](../apps/web/src/components/workspace/workspace-page.tsx), which composes
environment, provider, conversation and terminal features. The shell handles layout and
profile navigation without owning provider operations. Conversation presentation supplies
its history, title and content to that composition; terminal visibility and resizing do
not control shell-process lifetime. Feature controllers retain requests, recovery and
private state ownership. Shared [UI primitives](../apps/web/src/components/ui) provide
native buttons, dialogs, loading feedback, the wordmark and resizable panes.
[Design tokens](../apps/web/src/styles/app.css) own surfaces, typography and motion
independently of application policy.

The entry flow shows GitHub sign-in, environment preparation, provider authorization when
needed, then the workspace. Provider settings and explicit shutdown move to the profile
menu; the former environment dashboard and permanent refresh toolbars are removed.
The [design contract](DESIGN.md) defines the graphite surfaces, self-hosted Space Mono,
responsive panes, keyboard behavior and reduced-motion treatment.

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
names are presentation data. Sign-in requests repository permissions and retains the
provider token encrypted server-side only after active admission. The session is revoked
if credential persistence fails. Repository credential storage rechecks session and epoch.
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
and expiry clear private query data. BroadcastChannel carries invalidation
only, not session details. Locally observed authorization refusals notify other tabs;
received invalidations hide private content and revalidate without broadcasting again.
Initial admission and protected route entry require verification; an unconfirmed logout
cannot silently restore the session. Routine local rechecks share one pending request
and preserve the existing active snapshot until its fixed expiry. An unchanged response
does not notify resource consumers. The opaque presentation scope changes with user,
session token or role, remounting private views when necessary. Network failure/offline
state keeps an unexpired active view with a compact notice, without extending expiry;
authorization refusal, invalidation or expiry immediately removes it. Environment and
provider observers pause while hidden without erasing known observations, then refresh
silently on return. Routine focus checks do not recreate chat or terminal instances.

## Persistence and local ownership

[IdentityDatabase](../crates/database/src/identity/mod.rs) accepts only a verified target.
It does not consume an arbitrary database URL or read credential files. Startup and
readiness inspect state without DDL. The local setup and reset commands apply the single
[initial schema](../crates/database/schema.sql) to a verified local database. Reset is
explicit and deletes database records; runtime startup never resets data.

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
IdentityDatabase without exposing its SQLx pool. The initial schema records one environment
per user, desired and observed states, generation and provisioning phase. Requests
revalidate the active session under a user-then-environment lock. Generation checks
prevent stale observations from overwriting newer intent.

Provisioning records intent before volume creation and confirms the volume before
starting the container. A previously confirmed volume that disappears is an error;
ambiguous interrupted creation requires operator investigation. Neither case silently
replaces user data. Resource names and labels bind installation, environment, user and
policy. The adapter checks ownership before mutation and complete image/mount/isolation
policy before start. Foreign resources are never adopted. See the
[policy owner](../crates/adapters/src/environments/policy.rs) for enforced isolation and unlimited CPU/memory/process settings.

The [prepared host factory](../apps/api/src/bootstrap/environments.rs) retains the
installation file lock and validated preparation metadata for the API lifetime. Each
host owns a dedicated PostgreSQL advisory-lock connection, monitored every second and
checked around runtime calls. A failed check permanently closes that host's admission
and runtime mutation permissions; old requests and adapters never become usable again.

The [recovery coordinator](../apps/api/src/bootstrap/recovery.rs) drains the failed
host's reconciliation workers, then acquires a fresh database connection and advisory
lock before replacing the complete host and router. Acquisition attempts are bounded
and sequential, with exponential retry delays capped at 30 seconds. A competing
advisory-lock holder keeps the workspace unavailable. Each HTTP request selects one
coherent router, including requests on existing keep-alive connections; identity,
session revocation and quotas retain their existing owner. The replacement host must
complete its guarded sweep, and Running/Stopped observations must come from the new
host before interactive work or stopped-run recovery is allowed. Recovery preserves
persistent intent and model/run reservations; it does not replay interrupted turns or
infer cleanup. Explicit shutdown gates all hosts. Worker panics and permanent
configuration or resource-ownership errors stop automatic recovery for investigation.

Runtime persistence operations also have a client deadline covering pooled connection
acquisition and SQL, including transaction completion. Timed-out or cancelled operations
discard their connection, and normal pool recycling is bounded. This prevents stalled
session or chat queries from exhausting the pool needed by a replacement supervisor.
An uncertain commit is never retried automatically; a confirmed result is preserved
even if connection recycling fails. Docker work still settles before host replacement.

Individual Docker errors remain observable and are retried after 15 seconds. The
supervisor also repairs suspended-user intent independently of Docker work. The host
serializes work per environment and runs up to 32 environment jobs concurrently, so a
slow container does not block another user's stop when worker capacity is available.
This is bounded supervision, not a real-time shutdown guarantee under saturation. API
shutdown stops supervision while containers keep running; API restart resumes
reconciliation. No independent environment supervisor runs outside the API.

[Environment routes](../apps/api/src/environments/routes.rs) accept no target user ID or
Docker configuration. GET reads status without creating resources; POST initialize
creates intent only if no environment exists, while POST start/stop return accepted
persistent intent. Initialization and stop serialize under the user lock so a delayed
request cannot restart an existing stopped environment. There is no heartbeat endpoint.
All require an active session and mutations require Origin/CSRF checks. Public responses
contain status, observation time, and bounded error codes,
never Docker identifiers. Image/policy mismatch blocks Start and interactive use while
Stop remains useful.
The [member controller](../apps/web/src/features/environments/environment-controller.ts)
polls only while visible, online and authenticated. On verified workspace entry it
initializes an absent environment or requests start for a stopped one, after checking
recoverable runs. It consumes that opportunity even when the environment is already
running and retains the decision in memory across route remounts and session rechecks.
Explicit Stop and failed starts cannot trigger an automatic restart loop. A new browser
load or authenticated session gets a new entry opportunity. Failed recovery checks and
recoverable work keep the environment stopped until explicit retry/recovery. Session
loss, logout and unmount cancel browser requests and clear displayed private data without
changing server intent or stopping the container.

Missing preparation affects only environments when identity is otherwise ready. Prepared
metadata keeps the installation UUID and immutable image ID under `.factory/environments`.
Existing containers are not silently replaced. Linux and Windows use local Docker
transports; containers have no Docker socket, run non-root and keep a persistent home.

## Isolated egress

[DockerEgress](../crates/adapters/src/egress/runtime.rs) owns one shared proxy and one
internal bridge per user, using Docker's isolated gateway mode and disabled IPv6. User
containers have no external default route or published ports. The runtime checks exact
network membership and ownership before interactive exec. Only the proxy joins both
the user's bridge and the installation's outbound bridge; IP forwarding is disabled.

The pinned [Squid policy](../infra/egress/squid.conf) permits CONNECT on port 443 only for
the listed OpenAI, Anthropic and GitHub SSH hosts. DNS answers and actual destination addresses are
both checked against private/reserved ranges. The proxy entrypoint installs
its nftables policy, then drops privilege and all capabilities before serving requests.
There is no TLS interception or tunnel-content logging. Unsupported engine/network policy
fails closed. Proxy failure affects Codex availability without disabling the local shell;
status reads never start the proxy or user environment.

## Repository access and project imports

[RepositoryAccessService](../crates/application/src/identity/repository_access.rs) owns
the session-bound repository capability, including stable identity verification and
idempotent public SSH key registration. The GitHub adapter requests repository, public-key
and verified-email scopes; [authenticated encryption](../crates/adapters/src/identity/cipher.rs)
protects the persisted token with an automatically generated, persistent installation key.
The key is stored outside the database and user environments; if missing while encrypted
credentials exist, repository access fails closed rather than silently replacing it. The browser never receives
the token or SSH private key. Revoked OAuth access and a missing encryption key do not
invalidate a Factory identity session.

[Projects](../crates/core/src/projects/mod.rs) and their [store](../crates/database/src/projects/mod.rs)
reserve a provider repository ID once per member and fence operations by session and
environment generation. The API filters each bounded GitHub page against stored IDs.
The stable local directory is the validated repository name, checked for conflicts before
reservation and finalized without replacement; it does not change when GitHub renames a repository.
On project use, the API verifies the original clone path against GitHub's current identity
without following redirects. Redirects, missing or inaccessible paths, and different IDs
or names are reported for review; provider errors leave the status unverified. This
observation does not inspect or modify Git remotes and cannot guarantee a safe push:
GitHub can change afterward, and users can change a remote or push URL.
The [container runtime](../crates/adapters/src/projects/mod.rs) executes an immutable,
bounded controller with an application-owned staging receipt and no-replace finalization.
Deletion durably fences new project runs before moving only the verified checkout and its
owned import stage to a private quarantine. A persistent inode receipt permits a safe retry
without touching a newly created namesake. Only after filesystem cleanup does a transaction
remove the project's chats, run history and project row. Active or uncertain runs block deletion.
A result that may still have descendants remains uncertain; stopped-state recovery checks
a fresh observation, then the original on-disk receipt before confirming or retrying.
Before retry, the controller removes only the stopped operation's verified temporary checkout;
its marker remains so interrupted cleanup is safe to repeat.
The key is generated within the private home at container startup and reused on restart.
GitHub SSH traverses the exact `ssh.github.com:443` proxy allowance and verifies the
published host key. All processes in that container, including Codex, can use its key.

The workspace image includes an unmodified, checksum-verified Claude Code binary.

## Terminal and account operations

[TerminalService](../crates/application/src/terminals/mod.rs) owns one-use, 30-second
attachment tickets bound to user, session, environment generation, conversation and takeover decision.
HTTP ticket creation requires Origin and CSRF. WebSocket upgrade requires the session,
exact Origin and fixed subprotocol; the ticket travels in the first message, never the URL.
Input and resize requests revalidate access. Passive streams check at most every five
seconds and expire at the exact session deadline; local logout/suspension notifications
close them promptly. An in-flight Docker attachment is settled and detached after
revocation rather than abandoning its handle.

The [Docker terminal adapter](../crates/adapters/src/terminals/mod.rs) attaches a fixed
non-root tmux command with zsh in the current image. Each conversation owns a distinct
persistent tmux session; reattachment preserves the shell and its current directory.
tmux owns shell lifetime; the API owns only the attached client.
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
shares one environment controller across preparation, profile actions and dependent panes.
The provider authorization view and account settings share the connection controller;
opening settings or resuming the page never begins a new device authorization.
Codex logout is blocked when a conversation run is active or cleanup remains uncertain.
The [Claude account adapter](../crates/adapters/src/claude_account/mod.rs) starts the native
CLI in a private, echo-disabled PTY, validates its official sign-in URL, and forwards the
browser-issued one-time code without persisting it. The operation is bound to the initiating
session and environment generation. Cancellation interrupts the CLI and checks process exit;
only a successful account read confirms sign-in. A lock and watchdog bound abandoned logins.
Claude logout uses the same active-run guard as Codex.

## Conversations and execution

[Conversation contracts](../crates/core/src/conversations/mod.rs) and
[service](../crates/application/src/conversations/mod.rs) bind user-owned conversations,
runs, completed messages and grouped questions to the verified session and environment
generation. A conversation may reference one owned project; its immutable project
association selects the verified repository directory at every start and resume.
[PostgreSQL persistence](../crates/database/src/conversations/mod.rs)
serializes owner/session checks with changes, reserves turns idempotently and retains
completed message items. Legacy TypeScript execution history remains separate and is
not assigned to a web user.

The [conversation coordinator](../crates/application/src/conversations/coordinator.rs)
registers each provider's account service and execution runtime together. It dispatches
provider-qualified models to the [Codex](../crates/adapters/src/codex_execution/mod.rs)
or [Claude](../crates/adapters/src/claude_execution/mod.rs) adapter, persists private
provider session references before starting a turn and delivers bounded event updates.
Model discovery retains provider-reported resolved identifiers and supported reasoning
efforts. Explicit effort selections are checked against a fresh discovery before launch;
an authorized existing receipt is recovered first, and its durable model selection includes
the effort so idempotent retries cannot change it. A conclusively failed provider discovery
does not hide models from other providers; uncertain process cleanup still retains its lease
and blocks discovery.
Claude receives `--effort`; Codex receives the turn's `effort` field. An omitted effort
leaves provider defaults in control.
Both adapters use the bounded [JSON-lines process transport](../crates/adapters/src/json_stream.rs)
while owning their respective protocol correlation. Account and model
operations use per-user gates. Each conversation
admits only one active or uncertain run; independent conversations have no Factory-wide
run-count ceiling. Model probes reserve
a durable lease before starting a provider process, with at most one lease per user;
unconfirmed cleanup retains it across API restart. Overlapping discoveries for the same
user and environment share one in-flight result; each caller validates its own session
and environment, and completed results are not cached on the server. The web client
keeps each user's last successful model list in session memory without refetching on tab
changes; confirmed provider connection changes refresh it and session changes clear it.
When a connected provider has no returned models, the interface offers a retry while keeping
the other providers usable. Sending waits until the selected model is available in the catalog.
Conflict responses use
progressive retries, slowing to one attempt per minute while visible; unavailability
cancels pending reads and retries without discarding cached models.
The [API host](../apps/api/src/conversations/runtime.rs) supplies
task execution, timers and HTTP-compatible encoding; application policy stays in the
coordinator. Codex uses `dangerFullAccess` and approval policy `never`; Claude uses its
native bypass-permissions mode. Commands therefore have the container
user's access, including the home and stored credentials. The container and egress
policy remain the outer isolation boundary; there is no provider workspace sandbox in this
mode. Unexpected actionable provider requests fail visibly. Model lists are not proof of
account entitlement.

The [provider session mapping](../crates/application/src/conversations/provider_sessions.rs)
retains legacy Codex references and uses compare-and-set persistence when adding a
provider reference. When a conversation changes provider, saved messages are supplied as
context; truncated or oversized history fails rather than being silently omitted.

[HTTP and SSE routes](../apps/api/src/conversations/mod.rs) require a verified session;
mutations also require Origin and CSRF checks. Native provider session IDs remain internal. Completed
messages are durable; streaming fragments are transient. Conversation listing uses
user-owned keyset cursors, newest first, and provides older pages on request. History
presents a bounded
recent window, prioritizing active or uncertain work, and marks truncated runs, messages
and requests. Question replies are validated and claimed idempotently; a successful
write alone is not delivery, which requires a correlated provider notification. Claimed
or uncertain replies are not silently resent. Interruption
or process exit alone does not establish descendant cleanup; uncertain runs remain
blocking. Explicit recovery requires a current supervisor observation of the owned
environment stopped at a later generation and a conclusively settled launch request.
An unsettled launch stays fenced even after Stop; no verified operator recovery currently
exists. A stopped-state route exposes only owned
recoverable run identifiers. A fresh, error-free stopped observation at a later generation
also releases a settled uncertain model-probe lease in the supervisor transaction, including for
a suspended member; it does not change the environment intent or session. A separate
guarded route can check the same recovery. Recovery does not undo external effects.

## Feature workflows

[Core feature state](../crates/core/src/features/mod.rs) owns revisions, attempt claims,
review identity and delivery receipts. The [application coordinator](../crates/application/src/features/mod.rs)
reuses project and conversation ownership and provider discovery. The [feature store](../crates/database/src/features/mod.rs)
applies transitions transactionally with operation replay and environment generation
fences. The [feature API](../apps/api/src/features/mod.rs) projects only member-facing
state. The provider [codec](../crates/adapters/src/feature_codec.rs) translates structured
agent replies and computes revision fingerprints outside application policy.

The [project controller](../infra/environments/project-controller.py) uses owned Git
checkouts and validates a writer's paths before integration. The [feature execution adapter](../crates/adapters/src/environments/feature_execution/mod.rs)
mounts only the verified attempt checkout, read-only for reviewers. Disposable workers
have no direct network, GitHub SSH key or member home mount. Provider work uses a fixed
relay to the selected provider and an access-only credential seed. The selected worker
can access this seed. It does not receive refresh credentials, so an expired connection
must be renewed before retrying. Verification commands have neither provider credentials
nor network. The main environment owns the trusted Git push; the GitHub token stays on
the server.

Factory verifies the stable GitHub repository identity before and after initializing
an owned checkout. A project deletion also quarantines and removes its verified feature
workspaces. Worker checkouts share the member's Docker volume, whose capacity depends
on the host storage configuration.

Reviewers and trusted checks use the same committed snapshot. The [delivery service](../crates/application/src/features/delivery.rs)
compares the approved review with current local and remote refs, verifies the stable
repository identity and reconciles the draft receipt before retrying. Ordinary Chat
retains its own provider execution policy.

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
These packages remain porting references, not a second live web backend.

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
conversation correlation in that reference remains session-local. These mechanisms have
no per-user ownership and are not the web execution implementation.
