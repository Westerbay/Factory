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
  future bridge. Internal domain code does not depend on Zod. Schema/domain parity is
  checked in both directions without adding a source dependency between these packages.

## Deliberately excluded

API/NestJS, SSR/router/authentication/sample users, mail/Mailpit, Valkey and monitoring
are absent from runtime dependencies and scripts. No template stylesheet, font or page
layout is retained. Shadcn is deferred until a real control is needed.

## Persistence lifecycle

- [createDatabase](../packages/database/src/client.ts) adapts the template's Drizzle and
  postgres-js connection/ready/close pattern. It owns the PostgreSQL RunStore, explicit
  migration and startup reconciliation lifecycle. Use it only in a privileged host with
  a verified Factory target; never import it into renderer or preload.
- [Target verification](../packages/database/src/target.ts) strengthens the template's
  local-target guard with project, checkout, ownership and published-port identity.
  [Docker inspection](../scripts/database/owned.mjs) supplies the actual resource proof
  before setup/migration/integration work. A loopback address alone is insufficient.

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
  checks and stdio startup. Host code supplies an explicit executable path; readiness
  never performs sign-in, token refresh, inference or global configuration changes.
- [Generated protocol provenance](../packages/adapters/src/codex/generated/provenance.json)
  records installed CLI output and the focused binding closure. The types are generated
  by Codex's ts-rs protocol generator; Factory's projections, transport and mappings are
  original. Zod 4.6.7 is reused from the existing workspace validation stack rather than
  introducing a second validator. Protocol behavior was checked against the installed
  generator and official [app-server documentation](https://learn.chatgpt.com/docs/app-server).
