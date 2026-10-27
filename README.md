# Factory

A local desktop workspace for agent workflows, built with Electron, React and TypeScript.
The workspace provides the developer foundation, provider-neutral execution rules and
validated desktop contracts, a PostgreSQL persistence adapter and a pinned Codex app-server adapter. A separate Electron utility host connects these implementations to a minimal operator
interface: native workspace selection, tasks/skills, agent relationships, inert output,
explicit questions/permissions, stop, conversation continuation and persisted history.

## Get started

Use Node.js 24 (see `.node-version`) and pnpm 12.11.2. A graphical Linux session is
required to open the desktop window.

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

Derived from WebApp Template under the [MIT license](LICENSE). The template's web API,
authentication, mail and monitoring services are excluded. PostgreSQL and Drizzle persist runs through an isolated database package.

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
