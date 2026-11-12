# Reuse and attribution

## Source and selected mechanisms

Foundation: the locally available **WebApp Template / WebApp-Skull**, source commit
`97bcdec32f192134c561e2a75c8f252e967939e5`, generated with its supported English profile
into a fresh temporary staging directory before adaptation. The source checkout remains
unchanged. Original MIT copyright and terms are retained verbatim in [LICENSE](../LICENSE).

The supplied `webapp-template-en-rust.zip` (SHA-256
`75135695bddc6d2f888b86e693040ac5d725d20c4e87e808adf71cae64337427`)
provides the Axum/Tokio lifecycle and modular API organization reference. Factory adapts
selected mechanisms rather than copying the template application or its authentication.

Retained conventions include pnpm/Turborepo workspaces, strict TypeScript configuration,
package naming, formatting, documentation roles and link checks. Dependency-cruiser and
its temporary-fixture regression approach are adapted to Factory's boundaries, alongside
Cargo dependency enforcement. The static frontend uses React, Vite, TanStack Router/Query
and Zod; Rust uses Axum, Tokio and SQLx. Exact versions belong in the workspace manifests
and lockfiles.

NestJS, TanStack Start/SSR, Better Auth, password/mail flows, sample users, Mailpit,
Valkey and monitoring are excluded from the web/API runtime. No template visual identity
is required. Shadcn primitives remain deferred until an actual interaction needs them.

## Shared browser presentation

- [Button and IconButton](../apps/web/src/components/ui/button.tsx) preserve native button
  props and provide shared primary, secondary, ghost and danger variants. Icon-only
  actions require an accessible label; navigation remains a link.
- [Modal](../apps/web/src/components/ui/modal.tsx) wraps the native dialog with a named
  heading, controlled visibility, Escape handling and initial focus. Shutdown and member
  suspension supply their own consequences and confirmation actions; the primitive owns
  no mutation or authorization policy.
- [LoadingState](../apps/web/src/components/ui/loading-state.tsx) provides concise live
  status and a reduced-motion-safe indicator for preparation states.
  [Brand](../apps/web/src/components/ui/brand.tsx) provides the shared workspace link.
- [SplitPane](../apps/web/src/components/ui/split-pane.tsx) owns pointer and keyboard
  resizing for the chat and terminal panes and an expanded secondary pane. Visibility
  and expansion preserve mounted content and the requested split width; hidden content
  is inert. Its exported minimum widths let surrounding layout preserve usable panes.
  The component does not start or stop a terminal process.
- [MessageMarkdown](../apps/web/src/features/conversations/message-markdown.tsx) renders
  completed assistant messages and live deltas with the same scoped typography and
  overflow behavior. The MIT-licensed `react-markdown` and `remark-gfm` packages supply
  parsing and GFM support. The component ignores raw HTML, keeps images inert, and
  accepts only explicit HTTP(S) and mail links without embedded credentials. User
  messages remain plain text; do not bypass this boundary with raw HTML rendering.
- [Shared tokens](../apps/web/src/styles/app.css) and
  [control styles](../apps/web/src/components/ui/ui.css) own replaceable visual conventions.
  The [workspace composition](../apps/web/src/components/workspace/workspace-page.tsx) joins feature
  content with sidebar, profile navigation and terminal layout. Provider operations stay
  in their feature controllers; shell layout does not issue provider requests.

Space Mono 400/700 is self-hosted from [bundled font files](../apps/web/public/fonts/space-mono)
with its [SIL Open Font License 1.1](../apps/web/public/fonts/space-mono/OFL.txt), copyright
2016 The Space Mono Project Authors. The bundled [GitHub mark](../apps/web/public/brand/github.svg)
comes from Simple Icons and retains its [CC0 1.0 license](../apps/web/public/brand/SIMPLE-ICONS-LICENSE.txt).
Outline interface icons use the installed `lucide-react` package. Font and icon assets
are public static resources; loading the interface does not require a remote font service.

## Identity contracts and composition

- [Identity types, policy and ports](../crates/core/src/identity/mod.rs) own stable
  identity, admission, sessions and access-change semantics. Provider and persistence
  implementations consume these contracts rather than inventing parallel rules.
- [IdentityService](../crates/application/src/identity/mod.rs) owns use-case orchestration.
  HTTP handlers compose it through the API; adapters never depend on the HTTP layer.
- [Rust HTTP contracts](../apps/api/src/contracts/mod.rs) own browser transport shapes.
  Handler annotations and `utoipa-axum` registration in the
  [shared router composition](../apps/api/src/app.rs) own HTTP operations; `ToSchema`
  and `IntoParams` derive their shapes from Rust types, without a parallel path list.
  [The generation command](../scripts/api-contract.mjs) exports OpenAPI and TypeScript
  without runtime configuration. Regenerate instead of manually editing generated output.
- [The browser API client](../apps/web/src/lib/api/client.ts) owns the public API
  origin, credentialed requests, no-store behavior and authorization invalidation.
  It resolves `VITE_API_BASE_URL=/` to the page origin for the bundled deployment.
  Features reuse the client and its resolved origin.
- [Session context and controller](../apps/web/src/lib/auth/session-context.tsx) serve
  member and administrator views with shared verification, expiry, logout and cross-tab
  rechecks and invalidation. Keep private session state in memory and clear private queries through
  this lifecycle; do not add feature-specific session caches. Routine checks coalesce
  without replacing an active snapshot or its expiry deadline. An opaque presentation
  scope keys private UI by user, session token and role; it never authorizes a request.
  Hard invalidation or expiry destroys it. Transient connection notices do not reset
  an unexpired workspace.

- [Repository access contracts](../crates/core/src/identity/repository_access.rs) and
  [service](../crates/application/src/identity/repository_access.rs) define a
  session-bound provider capability authorized at sign-in. Reuse the service for
  repository operations so token renewal and the authorization epoch are enforced
  centrally. Adapters implement provider I/O and encrypt both credentials.
- [Project contracts](../crates/core/src/projects/mod.rs) own stable repository identity,
  bounded import reservations and stopped-state recovery. The
  [project adapter](../crates/adapters/src/projects/mod.rs) owns typed container operations;
  the API never accepts browser-selected filesystem paths or clone URLs.

## Persistence and ownership

- [IdentityDatabase and PgIdentityStore](../crates/database/src/identity/mod.rs) own SQLx
  connection/readiness/close and transactional identity persistence. The
  [API database selector](../apps/api/src/bootstrap.rs) is shared by startup and the
  administrator CLI. It uses a verified development target or the private production
  Compose database. Both environments use the shared initial schema.
  Operator-only administrator setup shares identity locking/upsert with login but is
  deliberately excluded from the HTTP `IdentityStore` port.
- [Bounded database operations](../crates/database/src/connection.rs) own runtime pool
  acquisition, query deadlines and connection disposal for identity, environments,
  conversations and readiness. Reuse this boundary so a silent socket cannot retain
  pool capacity indefinitely. Preserve transaction ordering and confirmed results;
  a timeout never establishes rollback and must not trigger automatic write replay.
- [Rust target verification](../crates/database/src/local_target/mod.rs) and the retained
  [TypeScript verifier](../packages/database/src/target.ts) enforce equivalent ownership
  expectations. [Docker inspection](../scripts/database/owned.mjs) supplies the local
  resource proof for explicit setup and integration. A loopback address alone is not
  proof of ownership.
- [createDatabase](../packages/database/src/client.ts) retains the template's
  Drizzle/postgres-js connection, readiness and close pattern for the execution RunStore.
  Its initialization and recovery operations remain explicit and separate from readiness.

The Factory schema and synthetic fixtures are original. PostgreSQL 17.11
Bookworm is pinned by image digest; existing template tables, credentials and volumes
are not adopted. Windows checkout labels and Docker named-pipe handling are preserved.
Connection patterns were checked against the official
[Drizzle PostgreSQL guide](https://orm.drizzle.team/docs/get-started-postgresql) and
[postgres-js transaction documentation](https://github.com/porsager/postgres#transactions).

## Environment contracts

- [Environment types and ports](../crates/core/src/environments/mod.rs) own durable
  persistent intent, observations and provisioning semantics. Terminal, account and
  conversation execution consumes these contracts rather than creating another lifecycle.
- [EnvironmentService and EnvironmentReconciler](../crates/application/src/environments/mod.rs)
  separate authenticated requests from serialized runtime effects. Generation checks
  protect newer intent; ownership loss permanently closes that host's availability.
  The [API recovery coordinator](../apps/api/src/bootstrap/recovery.rs) replaces the
  complete host after old workers settle and a new database guard is acquired. Reuse
  fresh host services together; never reopen an old availability latch or republish
  observations made before the replacement host started.
- [Environment persistence](../crates/database/src/environments/mod.rs) uses the verified
  IdentityDatabase and transactional user/session revalidation. Suspension and environment
  requests share user locking; browser presence and session expiry never change running
  intent. The supervisor owns a dedicated database connection.
- [DockerEnvironmentRuntime](../crates/adapters/src/environments/mod.rs) implements
  local-only inspection, provisioning, start and stop with resource-ownership and
  isolation checks. [Preparation](../scripts/environments/prepare.mjs) owns image builds
  and stable installation metadata. Reuse these owners without exposing daemon access,
  Docker IDs or mutable policy parameters to browser features.
- [useEnvironment](../apps/web/src/features/environments/use-environment.ts) owns one
  browser lifecycle subscription shared by workspace preparation, profile actions and
  dependent panes. Its controller retains automatic-entry consumption across route
  remounts in memory and checks recovery before starting a stopped environment. Consumers
  use confirmed state rather than starting another polling loop or inferring availability
  from an earlier accepted operation. Explicit stop, retry and recovery remain deliberate.

## Interactive processes and account connections

- [Terminal contracts](../crates/core/src/terminals/mod.rs) and the
  [terminal service](../crates/application/src/terminals/mod.rs) own authenticated
  attachments, single-use tickets and explicit takeover. The
  [HTTP/WebSocket module](../apps/api/src/terminals/mod.rs) owns browser framing and
  session supervision. Reuse these boundaries without exposing arbitrary Docker exec
  commands or treating a socket lifetime as the shell lifetime.
- [Identity revocation notifications](../crates/application/src/identity/mod.rs) inform
  terminal and account hosts immediately after local session or user revocation. Hosts
  also revalidate access against persistence and enforce exact session expiry; local
  notification alone is not an authorization guarantee.
- [AgentConnectionPort](../crates/core/src/agent_connections/mod.rs) projects account
  state and login challenges and optionally forwards a browser-issued one-time code. The
  [application service](../crates/application/src/agent_connections/mod.rs) binds pending
  operations to the verified session and environment generation. Exact Factory operation
  IDs prevent a stale cancellation from cancelling a newer sign-in.
- [CodexAccountAdapter](../crates/adapters/src/codex_account/mod.rs) implements that
  account port using the pinned native app-server protocol. It shares the internal
  [fixed Docker process transport](../crates/adapters/src/environments/process.rs) with
  the terminal adapter. Account operations never start agent turns or expose provider
  tokens; the container home remains the credential source of truth.
- [ClaudeAccountAdapter](../crates/adapters/src/claude_account/mod.rs) owns the native
  CLI sign-in process and logout. It forwards the one-time browser code without persisting
  it; account tokens remain in the private container home.
- [The account HTTP lifecycle](../apps/api/src/agent_connections/lifecycle.rs) owns
  detached request completion, pending-operation monitoring and revocation cleanup.
  Browser disconnection must not abandon an authorized process without an owner.
- [DockerEgress](../crates/adapters/src/egress/mod.rs) owns installation proxy and private
  network inspection. Network availability is separate from local terminal availability;
  features must not infer permission to access arbitrary destinations from a running
  container or a healthy proxy.

## Conversation execution

- [Conversation contracts](../crates/core/src/conversations/mod.rs) and the
  [application service](../crates/application/src/conversations/mod.rs) own user and
  environment-bound conversation, run, question and recovery behavior. Use their
  validated operations instead of reading another user's persistence directly.
- [ExecutionRuntime](../crates/core/src/conversations/execution.rs) separates provider
  control from bounded events. The [Codex](../crates/adapters/src/codex_execution/mod.rs)
  and [Claude](../crates/adapters/src/claude_execution/mod.rs) adapters implement that
  contract; native session references remain private. Model results include the
  provider-reported resolved model and supported effort levels; validated selections
  carry an optional effort through the same durable idempotency boundary.
- [Conversation coordinator](../crates/application/src/conversations/coordinator.rs)
  owns durable claims, provider registration, model routing, lifecycle and session checks.
  Its [provider session mapping](../crates/application/src/conversations/provider_sessions.rs)
  preserves old Codex sessions and transfers bounded conversation context on provider
  changes. The
  [API routes](../apps/api/src/conversations/mod.rs) and
  [host facilities](../apps/api/src/conversations/runtime.rs) own HTTP, event transport,
  task execution and encoding. The event stream accepts either the configured frontend
  Origin or the browser's same-origin Fetch Metadata on a shared-origin deployment when
  Origin is absent. Uncertain cleanup requires explicit recovery after a
  verified environment stop and a conclusively settled launch; reconnecting must not
  replay a turn. A stopped observation alone cannot release an unsettled launch. Consumers must honor
  conversation-page cursors, history truncation and claimed/uncertain question receipt
  states. Concurrent model readers share one in-flight discovery per user/environment,
  with each caller validating its own access before and after waiting. Model discovery
  reserves a durable lease before starting a provider process;
  confirmed cleanup or a fresh later-generation stopped observation releases it only
  after the launch request has settled. The
  supervisor can release the lease for suspended users without changing environment
  intent or session state; the explicit stopped-state recovery route remains available.

## Feature workflow capabilities

- [Feature contracts](../crates/core/src/features/ports.rs) define the application-owned
  execution, Git, storage, and codec capabilities. Keep provider messages and concrete
  persistence out of workflow transitions.
- The [feature API client](../apps/web/src/features/feature-workflows/api.ts) treats
  workflow IDs as opaque hexadecimal GUIDs. Persisted revisions and review attempts
  may use hash-derived IDs without standard UUID version or variant bits.
- [ArtifactPreview](../apps/web/src/features/artifacts/artifact-preview.tsx) owns the
  shared isolated Mermaid and HTML presentation used by feature architecture and
  explicit conversation previews. Its labels come from the consuming interface.

## Execution porting reference

The temporary `packages/execution-reference` package preserves the former host and
presentation behavior for regression tests only. It is not a reusable product
dependency: use the Rust conversation runtime for web execution. Native
window and process-launch mechanisms are retired rather than exposed to the browser.

- [Core model and reducer](../packages/core/src/index.ts) define provider-neutral run,
  agent and interaction rules. Do not duplicate transitions in a UI or store.
- [AgentRuntime and RunStore](../packages/core/src/ports.ts) require confirmation,
  atomicity, ownership and detached reads from implementations.
- [ExecutionService](../packages/application/src/execution-service.ts) owns explicit
  start, continuation, responses, cancellation and history. Durable projections remain
  distinct from transient output. Its ID and clock sources require unique stable IDs
  and ISO UTC timestamps.
- [Validated contracts](../packages/contracts/src/index.ts) preserve the execution wire
  shapes independently of core. Schema/domain parity checks retain the boundary without
  introducing a dependency between those packages.
- [CodexRuntime](../packages/adapters/src/codex/runtime.ts) implements the pinned
  app-server mapping. [createStdioConnector](../packages/adapters/src/codex/process.ts)
  owns executable checks and process startup; a future execution host must supply a
  trusted executable, preserve explicit cleanup and keep provider IDs private.
- [Generated protocol provenance](../packages/adapters/src/codex/generated/provenance.json)
  records installed CLI output and the focused binding closure. Types come from Codex's
  ts-rs generator; Factory's projections, transport and mappings are original. Protocol
  behavior was checked against the installed generator and official
  [app-server documentation](https://learn.chatgpt.com/docs/app-server).

These execution mechanisms have no web-user ownership and must not be composed into
member routes until that boundary is designed. They do not grant access to legacy history,
provider credentials or global configuration.

## Web interface language

The [`@workspace/i18n`](../packages/i18n/) package owns the English and French
Paraglide message catalogs and generated, typed browser runtime. The web locale adapter
in [`apps/web/src/lib/i18n`](../apps/web/src/lib/i18n/) selects the browser language
when available and persists explicit choices locally. Use generated messages for interface
copy; provider output remains user content.
