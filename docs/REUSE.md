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
- [The browser API client](../apps/web/src/lib/api/client.ts) owns the explicit public API
  origin, credentialed requests, no-store behavior and authorization invalidation.
  Features use it rather than infer addresses from the page location.
- [Session context and controller](../apps/web/src/lib/auth/session-context.tsx) serve
  member and administrator views with shared verification, expiry, logout and cross-tab
  invalidation. Keep private session state in memory and clear private queries through
  this lifecycle; do not add feature-specific session caches.

## Persistence and ownership

- [IdentityDatabase and PgIdentityStore](../crates/database/src/identity/mod.rs) own SQLx
  connection/readiness/close and transactional identity persistence. Explicit migrations
  use the isolated identity schema and ledger, preserving existing execution data.
  Operator-only administrator setup shares identity locking/upsert with login but is
  deliberately excluded from the HTTP `IdentityStore` port.
- [Rust target verification](../crates/database/src/local_target/mod.rs) and the retained
  [TypeScript verifier](../packages/database/src/target.ts) enforce equivalent ownership
  expectations. [Docker inspection](../scripts/database/owned.mjs) supplies the local
  resource proof for explicit setup and integration. A loopback address alone is not
  proof of ownership.
- [createDatabase](../packages/database/src/client.ts) retains the template's
  Drizzle/postgres-js connection, readiness and close pattern for the execution RunStore.
  Its migration and recovery operations remain explicit and separate from readiness.

The Factory schemas, migrations and synthetic fixtures are original. PostgreSQL 17.11
Bookworm is pinned by image digest; existing template tables, credentials and volumes
are not adopted. Windows checkout labels and Docker named-pipe handling are preserved.
Connection patterns were checked against the official
[Drizzle PostgreSQL guide](https://orm.drizzle.team/docs/get-started-postgresql) and
[postgres-js transaction documentation](https://github.com/porsager/postgres#transactions).

## Execution porting reference

The temporary `packages/execution-reference` package preserves the former host and
presentation behavior for regression tests only. It is not a reusable product
dependency: port the relevant behavior to Rust before adding web execution. Native
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
