# Factory

A local desktop workspace for agent workflows, built with Electron, React and TypeScript.
The workspace provides the developer foundation, provider-neutral execution rules and
validated desktop contracts, and a PostgreSQL persistence adapter. The window remains an honest foundation screen.
It does not execute agents or connect to a database yet.

## Get started

Use Node.js 24 (see `.node-version`) and pnpm 12.11.2. A graphical Linux session is
required to open the desktop window.

```bash
pnpm install --frozen-lockfile
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
Setup creates only Factory-owned infrastructure; the desktop is not connected to it yet.
