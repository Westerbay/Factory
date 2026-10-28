# Development

## Prerequisites

- Node.js 24; `.node-version` retains the template's 24.21.0 pin.
- pnpm 12.11.2 (also pinned in `package.json`).
- A graphical Linux session and Electron's Linux system libraries to open the window.
- Network access for a first dependency install and the Electron binary download.

```bash
pnpm install --frozen-lockfile
pnpm db:setup   # explicit owned local Docker setup and migrations
pnpm dev
```

`pnpm dev` starts the desktop main/preload build and the Vite loopback renderer.
The desktop `predev` hook first builds its workspace dependency closure. Both `predev`
and `prestart` run the official `install-electron` command:
Electron 45 downloads its binary on demand, while electron-vite expects it to exist.
The installer reuses an already verified installation; first launch needs network access.
The native close dialog defaults to keep open; choose stop and close to wait for owned
provider cleanup and pool shutdown. Do not use Ctrl+C as a substitute for confirmed agent
cleanup. A failed cleanup remains visible and keeps the window open. Stop active work
before editing main/utility source: development hot restarts and externally forced exits
cannot establish native descendant cleanup. `pnpm start` previews an existing production
build; run `pnpm build` first. Installers and packaging are not part of this increment.

## Checks

```bash
pnpm format:check
pnpm lint
pnpm typecheck
pnpm boundaries:check
pnpm test
pnpm docs:check
pnpm build
```

`pnpm check` runs the routine checks. `pnpm format` applies formatting when needed.
The Node test runner exercises temporary dependency graphs, rejecting forbidden imports
including type-only, resolved aliases, dynamic imports and Node builtins. Temporary
fixture directories are removed after each test; no persistent seed data is created.
`pnpm execution:test` builds the domain/application/contracts packages, checks independent
wire/domain type parity, and runs deterministic execution and schema tests with Node's
built-in runner. It covers concurrent starts, parent/child scope, blocking/nonblocking
questions, explicit continuation, provider permissions, duplicate/stale requests,
response/stop/disconnect ordering and persistence cleanup failure. Test adapters are
confined to test directories and do not start real agents or create persistent seed data.
The package runtime exports use built `dist` entries; TypeScript consumers resolve source
types. Build the owning packages before invoking their JavaScript entrypoints directly.

`pnpm desktop:test` builds the real host/application boundary and checks generation/source
validation, native workspace ownership, concurrent multi-question replies, persistence
readiness/recovery failure, stop/history and worker loss using deterministic doubles. It is
part of `pnpm check` and never connects to Docker or Codex.

For an explicit built-window acceptance, run `pnpm desktop:smoke`. It requires the same
graphical Linux session as the application and permission to access local Docker. It
creates only a unique identity-verified `factory-test-*` project, applies migrations,
builds a **separate test-only main and utility entry**, and uses a synthetic provider with
no model, tools or account. The production entry has no fixture/provider environment flag.
The journey exercises actual sandboxed preload IPC, workspace selection, independently
pending questions, explicit permission, inert malicious markup, stop, linked continuation,
worker death and restart recovery/history against PostgreSQL. Wide/narrow PNGs and bounded
logs are written to the reported temporary directory. Cleanup removes only that fixture's
verified container, volume and network. No development database is created or deleted.
This is desktop integration evidence, not native Codex execution or cleanup proof.

For manual UI inspection, run `pnpm dev` or `pnpm build && pnpm start`. Confirm readiness,
choose a workspace, inspect enabled skill names/paths and persisted history, and verify
narrow layout, focus outlines and explicit request states. Provider probe starts the
installed app-server only for managed-account metadata. Starting a task or continuing a
conversation invokes a model and should be deliberate. The bounded real-agent journey is
a separate acceptance step. Missing database setup should leave the interface unavailable
with recovery instructions; startup must not silently provision or migrate it.

## Local PostgreSQL

The utility host requires the verified owned database; renderer and preload remain
independent of database implementations. `pnpm check` runs deterministic
guard checks without Docker or secrets; `pnpm test:integration` explicitly requires a
local Docker Engine and Compose with permission to access its socket. First use may
pull the pinned official PostgreSQL 17.11 Bookworm image. No real `.env` file is read,
created or changed. The Compose file is operated through the guarded scripts below;
do not run template commands or supply an existing/shared database.

```bash
pnpm db:setup       # create/start owned factory-local PostgreSQL, then apply migrations
pnpm db:status      # read-only readiness; never creates tables or reconciles runs
pnpm db:migrate     # explicitly apply committed migrations to the verified local target
pnpm db:stop        # stop the exact owned service; retain its volume and marker
pnpm test:integration
```

Development uses project `factory-local`, database `factory_local`, user `factory`,
port `127.0.0.1:55432` and volume `factory-local_postgres_data`. The password
`local-fixture-only` is deliberately public synthetic local configuration, not a secret
or a production credential. The ignored `.factory/database-owner` file contains a
non-secret UUID ownership marker. Keep it with the checkout while its volume exists.
Setup rejects unknown preexisting resources instead of adopting them. A missing marker
with existing resources, conflicting port, Docker permission error or failed health
check requires resolving that exact condition; never reset/prune volumes to bypass it.
Running `db:setup` again preserves data and idempotently applies pending migrations.
Keep the marker after `db:stop`; no database reset/delete command is provided.

Every connection/migration target is checked against Docker project, owner and checkout
labels, container name, database identity, credentials, volume and exact loopback port.
Docker is pinned to `unix:///var/run/docker.sock`, independently of stored contexts or
`DOCKER_HOST`. Compose receives `--env-file /dev/null` and a restricted environment. Remote, shared,
unknown and mismatched targets are rejected before SQL. Database readiness returns
`pending-migrations`, `unavailable` or `closed`; normal readiness has no migration,
recovery or provisioning side effects. PostgreSQL clients and URLs remain privileged.

Integration creates a unique `factory-test-<UUID>` Compose project and owner token with
an OS-allocated loopback port and a separate durable test volume. It applies migrations
twice and exercises the actual ExecutionService with PostgreSQL and a test-only runtime:
created reservation, confirmed start, persisted question/answer and cancellation. It also
verifies cross-run foreign keys, concurrent active roots and versions,
idempotent event handling, full transaction/recovery rollback, answered and pending
questions, history pagination, reconnect, explicit restart recovery, unavailable and
closed connections. It never starts an agent or replays tools. Cleanup first checks all
resource identities, then removes only that fixture's exact container, volume and
network. Development resources are never deleted. If cleanup is interrupted, retain the
reported failure and inspect that exact `factory-test-*` project before manual removal.

For schema changes, run `pnpm --filter @workspace/database generate` and inspect SQL and
snapshots. The migration has an annotated manual `DEFERRABLE INITIALLY DEFERRED` root
ownership foreign key, which Drizzle's schema DSL cannot express; preserve it and run
integration after changes. No temporary fixture content belongs in migrations.

## Codex integration

The provider adapter targets **codex-cli 0.160.0**. Supply the absolute path of an existing
compatible executable to diagnostics; the desktop uses `codex` from its trusted host PATH.
Factory does not install or upgrade Codex. Sign in through Codex's
own supported UI/CLI outside Factory. The adapter uses the existing managed ChatGPT
sign-in and never reads token files, requests token refresh, changes global configuration,
or accepts an API key. A missing sign-in or incompatible version reports unavailable.

```bash
pnpm adapters:test
pnpm codex:protocol /absolute/path/to/codex --check
pnpm codex:probe /absolute/path/to/codex
```

`adapters:test` is part of `pnpm check`: sanitized in-process stdio fixtures exercise the
real transport and application layer without starting Codex, calling a model or reading
credentials. They cover framing/correlation/errors, skills, questions/permissions,
continuation, native child mappings and cancellation/disconnect races.

`codex:probe` is explicit and outside routine checks/CI. It checks the executable version,
initializes app-server, requests `account/read` with `refreshToken:false`, prints only
availability/capability booleans and safe reasons, then closes the owned server. It does
not start a thread or a model turn. Capability flags mean the pinned mapping is present,
not that native delegation or every model feature has been verified with this account.
The command may require permission to start local processes. Never paste account output,
provider stderr, credential files or real `.env` contents into troubleshooting reports.

For intentional binding regeneration, run `pnpm codex:protocol /absolute/path/to/codex`
and inspect the generated closure and provenance hashes. The generator rejects a changed
version: a provider upgrade requires a separate compatibility review, updated projections,
fixtures and pin. Generated files are preserved verbatim apart from ESM import extensions;
they are excluded from formatting. `--check` verifies exact installed-generator parity.

Current limitations: the current skill runtime port returns enabled names/paths without
descriptions or disabled entries; the complete intended catalog is not yet implemented. Conversation correlation survives only within one runtime instance. Secret
questions, file-change approvals without a supported patch presentation, and unsupported
actionable requests fail visibly. Stop waits for terminal
notifications from every observed native thread, including children discovered while
stopping. If confirmation is absent, it closes its server and reports cleanup unconfirmed;
it does not infer success from an interrupt RPC response or process exit. This unresolved
ownership also blocks reconnection/new runs and makes shutdown fail after disconnect. Real model,
skill execution, delegation and native cleanup remain unverified until the bounded live
acceptance journey. No live inference is required or authorized by routine diagnostics.

## Bounded real Codex acceptance

Human QA is accepted by a person. Keep agent-authored acceptance/browser scripts and artifacts in ignored local scratch. Meaningful unit and integration regression tests remain versioned. Live Codex work requires explicit authorization, a deliberately selected disposable workspace and confirmation that all owned agents have stopped.

## Adding code

Use the responsibilities in [architecture](ARCHITECTURE.md). Runtime and adapter
contracts belong with actual behavior, not placeholder APIs. Keep the UI appearance
replaceable. Run the boundary tests after changing path aliases or dependencies.
All local planning artifacts remain ignored under `docs/plans/`.

The provider's `serverRequest/resolved` notification can mean answered or cleared. Factory
correlates it only after sending the exact response and applies invalidation guards; the
non-inference diagnostic does not prove acceptance behavior for every possible provider
notification race. That remains part of bounded live integration acceptance.
