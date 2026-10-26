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

PostgreSQL/Drizzle connection lifecycle and guarded local infrastructure patterns will be
adapted in the persistence task, with Factory-owned schema and identities. No source
schema, SQL migration or database service is copied in this foundation task.
