# Development

## Prerequisites

- Node.js 24; `.node-version` retains the template's 24.21.0 pin.
- pnpm 12.11.2 (also pinned in `package.json`).
- A graphical Linux session and Electron's Linux system libraries to open the window.
- Network access for a first dependency install and the Electron binary download.

```bash
pnpm install --frozen-lockfile
pnpm dev
```

`pnpm dev` starts the desktop main/preload build and the Vite loopback renderer.
The desktop `predev` / `prestart` hooks run the official `install-electron` command:
Electron 45 downloads its binary on demand, while electron-vite expects it to exist.
The installer reuses an already verified installation; first launch needs network access.
Close the window or press Ctrl+C to stop development. The current application owns no
agent, database or background service. `pnpm start` previews an existing production
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

`pnpm check` runs all of these. `pnpm format` applies formatting when needed.
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

For a manual smoke check, open `pnpm dev`, verify the Factory foundation message and
that it states no agent is running/no database is open, then close the window. Repeat
with `pnpm build` and `pnpm start`. Resizing should not clip the content. There are no
interactive controls or privileged renderer methods to exercise yet.

## Persistence and agents: pending steps

PostgreSQL with Drizzle is the approved persistence stack. Docker Compose, local target
ownership checks, migrations and isolated database tests will be introduced in the
persistence task. Do not run template database commands against Factory or point it at
an existing/shared database. This increment requires no Docker, database or `.env` file.

The later Codex adapter will use an installed, compatible Codex CLI and the user's
existing sign-in. This scaffold never starts it, signs in, calls a model, modifies global
configuration or touches credentials. Protocol fixtures and the bounded live acceptance
journey will be documented when implemented; they have not run here.

## Adding code

Use the responsibilities in [architecture](ARCHITECTURE.md). Runtime and adapter
contracts belong with actual behavior, not placeholder APIs. Keep the UI appearance
replaceable. Run the boundary tests after changing path aliases or dependencies.
All local planning artifacts remain ignored under `docs/plans/`.
