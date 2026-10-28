# Factory

Factory is transitioning to a Rust API and an independently hosted React/TypeScript
web application. The first web increment currently provides a static frontend shell
and loopback-only API health endpoints. Authentication, users, PWA installation and
agent execution are not yet available in the web application.

The existing local desktop workspace remains available during this transition.
The workspace provides the developer foundation, provider-neutral execution rules and
validated desktop contracts, a PostgreSQL persistence adapter and a pinned Codex app-server adapter. A separate Electron utility host connects these implementations to a minimal operator
interface: native workspace selection, tasks/skills, agent relationships, inert output,
explicit questions/permissions, stop, conversation continuation and persisted history.

## Get started

For the new web foundation, use Node.js 24.22.1, pnpm 12.11.2 and Rust 1.90.0
(including rustfmt/clippy and the native linker for your platform). After
`pnpm install --frozen-lockfile`, run these in two terminals:

```bash
pnpm dev:api
pnpm dev:web
```

Open `http://localhost:3000`. The frontend calls the separate Rust API at
`http://localhost:3001`; Rust does not serve the frontend. These commands need no
database, provider credentials or model calls. The readiness endpoint intentionally
returns 503 until the identity/storage increment exists. See
[web foundation development](docs/DEVELOPMENT.md#web-foundation) for configuration,
build checks and local toolchain setup.

### Existing desktop

Use Node.js 24 (see `.node-version`) and pnpm 12.11.2. Open the desktop window from
a Windows desktop or a graphical Linux session. Local PostgreSQL requires Docker
Engine and Compose; on Windows, start Docker Desktop in Linux-container mode first.

```bash
pnpm install --frozen-lockfile
pnpm db:setup   # explicitly provision/migrate the verified local Docker target
pnpm dev
```

To preview the production bundle:

```bash
pnpm build
pnpm start
```

Run `pnpm check` for formatting, lint, types, boundary regression tests, dependency
rules, deterministic execution/interaction tests, wire-schema parity, documentation links
and the production build. This routine command needs no model calls, credentials,
Docker or database. The explicit integration command below requires local Docker.

## Documentation

- [Context](docs/CONTEXT.md)
- [Product](docs/PRODUCT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Design](docs/DESIGN.md)
- [Development](docs/DEVELOPMENT.md)
- [Reuse and attribution](docs/REUSE.md)

Derived from WebApp Template under the [MIT license](LICENSE), with selected mechanisms
from its Rust variant. Better Auth, mail and monitoring services are excluded.
PostgreSQL and Drizzle still persist desktop runs through an isolated database package.

For explicit local database setup (Docker Engine and Compose required):

```bash
pnpm db:setup
pnpm db:status
pnpm test:integration
pnpm db:stop
```

See [database lifecycle](docs/DEVELOPMENT.md#local-postgresql) before running these commands.
Setup creates only Factory-owned infrastructure. Desktop startup verifies it and recovers
abandoned runs without provisioning, migration or tool replay. Missing setup is visible
in the window.

The Codex adapter is verified with deterministic protocol fixtures and an explicit
non-inference readiness command. See [Codex integration](docs/DEVELOPMENT.md#codex-integration)
for the pinned executable and verification limits. The desktop resolves compatible `codex`
from its host PATH and uses its existing managed sign-in. Live Codex execution remains a
separate bounded acceptance step. `pnpm desktop:smoke` explicitly exercises the built window
with a separate synthetic provider and isolated real PostgreSQL, without inference or
credentials; it requires Docker and a graphical session.

Human acceptance remains separate from unit and integration checks. See [development](docs/DEVELOPMENT.md#bounded-real-codex-acceptance) before any separately authorized live model run.
