# Reuse and attribution

## Source

Foundation: the locally available **WebApp Template / WebApp-Skull**, source commit
`97bcdec32f192134c561e2a75c8f252e967939e5`, generated with its supported English profile
into a fresh temporary staging directory before adaptation. The generator refuses an
existing output directory. Factory planning documents were preserved.

The original MIT copyright and terms are retained verbatim in [LICENSE](../LICENSE).
Generation and adaptation do not modify the source template checkout.

## Retained and adapted

- pnpm workspace and Turborepo task conventions.
- Shared strict TypeScript base configuration, Node 24 pin and package naming.
- Prettier conventions (without the unused Tailwind plugin).
- Dependency-cruiser enforcement and the temporary-fixture regression harness,
  rewritten for the hexagonal desktop boundaries.
- The documentation link checker, documentation roles and license.

ESLint uses central recommended JavaScript/TypeScript rules for the retained Electron
workspace instead of carrying web-specific lint dependencies. Electron 45.0.0,
electron-vite 6.0.0, Vite 8.3.5 and React 19.2.6 are pinned. Package registry peer/engine
metadata was checked against Node 24; [electron-vite guidance](https://electron-vite.org/guide/)
and [Electron security guidance](https://www.electronjs.org/docs/latest/tutorial/security)
were consulted during setup. Electron 45 no longer installs its binary at package
postinstall time. Its official `install-electron` executable is run by desktop start hooks
so electron-vite can find the binary on a fresh checkout.

## Execution contracts

- [Core model and reducer](../packages/core/src/index.ts) own provider-neutral run,
  agent and interaction rules. Use them from application and adapter mappings; do not
  duplicate transitions in the renderer or persistence layer.
- [AgentRuntime and RunStore](../packages/core/src/ports.ts) are the extension points
  for provider and persistence adapters. Implement their confirmation, atomicity,
  ownership and detached-read guarantees before composing them into a host.
- [ExecutionService](../packages/application/src/execution-service.ts) owns start,
  continuation, answers, cancellation and history. Host observations distinguish durable
  projections from transient fragments; diagnostics expose background/storage failures.
  Host ID and clock sources must supply stable unique IDs and ISO UTC timestamps.
- [Desktop schemas](../packages/contracts/src/index.ts) validate commands, projections,
  results and events with Zod 4.6.7. Start commands reference host-issued workspace and
  skill selections; schema validation does not replace source/ownership checks in the
  privileged bridge. Internal domain code does not depend on Zod. Schema/domain parity is
  checked in both directions without adding a source dependency between these packages.

## Deliberately excluded

NestJS, TanStack Start/SSR, Better Auth, password/mail flows, sample users, Mailpit,
Valkey and monitoring are absent from the new web/API runtime. The frontend uses
TanStack Router/Query with Vite; Rust uses Axum/Tokio. No template branding or page
layout is required. Shadcn remains deferred until a real control needs it.

## Rust and static web foundation

The supplied `webapp-template-en-rust.zip` (SHA-256
`75135695bddc6d2f888b86e693040ac5d725d20c4e87e808adf71cae64337427`)
provides the Axum/Tokio lifecycle and modular API organization reference. The API shell
adapts those patterns while excluding its Better Auth/SeaORM and email implementation.
The template shares this repository's MIT attribution; preserve [LICENSE](../LICENSE).

The browser API client owns the public API origin and credentialed fetch behavior.
Feature modules should use it rather than infer server addresses from the current page.
Rust configuration owns the explicit frontend origin; future route modules must retain
the exact-origin boundary and implement their own session/permission checks.

## Persistence lifecycle

- [createDatabase](../packages/database/src/client.ts) adapts the template's Drizzle and
  postgres-js connection/ready/close pattern. It owns the PostgreSQL RunStore, explicit
  migration and startup reconciliation lifecycle. Use it only in a privileged host with
  a verified Factory target; never import it into renderer or preload.
- [Target verification](../packages/database/src/target.ts) strengthens the template's
  local-target guard with project, checkout, ownership and published-port identity.
  [Docker inspection](../scripts/database/owned.mjs) supplies the actual resource proof
  before setup/migration/integration work and desktop startup. It pins the local Docker
  socket on Linux or named pipe on Windows; checkout labels must exactly match the
  absolute POSIX or Windows drive path. A loopback address alone is insufficient.

Drizzle ORM 0.45.4, Drizzle Kit 0.31.11 and postgres-js 3.4.9 are pinned from official
registry metadata. PostgreSQL 17.11 Bookworm is pinned by image digest. The Factory schema,
migrations and synthetic fixtures are original; template tables, credentials and existing
volumes are not reused. Connection documentation was checked against the official
[Drizzle PostgreSQL guide](https://orm.drizzle.team/docs/get-started-postgresql) and
[postgres-js transaction documentation](https://github.com/porsager/postgres#transactions).

## Codex runtime

- [CodexRuntime](../packages/adapters/src/codex/runtime.ts) implements AgentRuntime for
  the pinned app-server. Compose it only in a privileged host; keep provider IDs,
  pending-request correlation and compatibility checks inside this adapter.
- [createStdioConnector](../packages/adapters/src/codex/process.ts) owns executable version
  checks and stdio startup. Host code supplies the trusted executable (the desktop currently resolves `codex` from its host PATH); readiness
  never performs sign-in, token refresh, inference or global configuration changes.
- [Generated protocol provenance](../packages/adapters/src/codex/generated/provenance.json)
  records installed CLI output and the focused binding closure. The types are generated
  by Codex's ts-rs protocol generator; Factory's projections, transport and mappings are
  original. Zod 4.6.7 is reused from the existing workspace validation stack rather than
  introducing a second validator. Protocol behavior was checked against the installed
  generator and official [app-server documentation](https://learn.chatgpt.com/docs/app-server).

## Desktop integration

- [Desktop bridge contracts](../packages/contracts/src/desktop.ts) own the narrow API,
  host status and generation-correlated utility messages. Reuse them across main, preload
  and renderer; validate untrusted IPC at each process boundary and check sender identity
  separately. They do not grant arbitrary paths or executable configuration.
- [DesktopHost](../apps/desktop/src/runtime/host.ts) is the privileged composition adapter
  for real application commands, native-granted workspace/skill references and sanitized
  execution observations. Compose additional adapters here rather than in presentation.
- [RuntimeClient](../apps/desktop/src/main/runtime-client.ts) owns bounded request correlation
  and process-loss handling. Human responses remain independent and have no human deadline;
  utility death never proves provider cleanup.
- [Owned PostgreSQL opening](../apps/desktop/src/runtime/database.ts) reuses the CLI's original
  Docker inspection module without bundling away its checkout identity. Setup and migration
  stay explicit; desktop initialization only verifies readiness and performs recovery.

The implementation follows Electron's official [utility-process API](https://www.electronjs.org/docs/latest/api/utility-process),
[context bridge guidance](https://www.electronjs.org/docs/latest/api/context-bridge) and
[security guidance](https://www.electronjs.org/docs/latest/tutorial/security), checked against
the installed Electron 45 types. No extra IPC, state-management or UI dependency was needed;
React's external-store subscription owns renderer synchronization without effect hooks.
