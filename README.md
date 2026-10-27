# Factory

A local desktop workspace for agent workflows, built with Electron, React and TypeScript.
The workspace provides the developer foundation, provider-neutral execution rules and
validated desktop contracts. The window remains an honest foundation screen.
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
and the production build. No check requires model calls,
credentials, Docker or a database in this increment.

## Documentation

- [Context](docs/CONTEXT.md)
- [Product](docs/PRODUCT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Design](docs/DESIGN.md)
- [Development](docs/DEVELOPMENT.md)
- [Reuse and attribution](docs/REUSE.md)

Derived from WebApp Template under the [MIT license](LICENSE). The template's web API,
authentication, mail and monitoring services are excluded. PostgreSQL and Drizzle are
approved for the later persistence step; there are no database commands or schema yet.
