# Factory

Factory is a private React/TypeScript web application with a Rust API. Its current
foundation provides GitHub sign-in, administrator-controlled admission, revocable
server sessions and a user-access backoffice. The frontend builds to static assets
hosted independently of Rust and is installable as a PWA.

The member workspace deliberately has no agent, repository or terminal controls yet.
The existing TypeScript execution packages remain references for a later Rust port;
their history is not assigned or exposed to web users. Hosting and deployment are
outside the current local increment.

## Get started

Use Node.js 24.22.1, pnpm 12.11.2 and Rust 1.90.0 with rustfmt, clippy and a native
linker. Windows Rust builds need the MSVC C++ tools and Windows SDK. After
`pnpm install --frozen-lockfile`, run these in separate terminals:

```bash
pnpm dev:api
pnpm dev:web
```

`pnpm dev` starts both processes together. After `pnpm build`, `pnpm start` runs
the release API and local static preview together; it is not a deployment command.

Open `http://localhost:3000`. The browser calls the separate API at
`http://localhost:3001`. The application can start without provider credentials;
sign-in remains unavailable until OAuth configuration, the verified owned PostgreSQL
target and identity migrations are ready. Runtime startup never provisions or migrates
the database, and does not load `.env` files.

See [development](docs/DEVELOPMENT.md) for explicit local database setup, API
configuration, built PWA inspection and human acceptance. No real OAuth application
registration or credential setup is performed by installation or routine checks.

## Validation

```bash
pnpm check
pnpm test:integration
```

`check` covers formatting, lint, types, module boundaries, behavioral regressions,
OpenAPI parity, documentation links and builds without Docker or provider credentials.
`test:integration` explicitly requires local Docker and uses isolated, identity-verified
fixtures; it does not migrate the development database or call a model.

Human QA and live GitHub/PWA acceptance remain separate. Temporary acceptance scripts
and captures stay in ignored local scratch; meaningful unit and integration regressions
remain versioned.

## Documentation

- [Context](docs/CONTEXT.md)
- [Product](docs/PRODUCT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Design](docs/DESIGN.md)
- [Development](docs/DEVELOPMENT.md)
- [Reuse and attribution](docs/REUSE.md)

Derived from WebApp Template under the [MIT license](LICENSE), with selected mechanisms
from its Rust variant. Better Auth, password/mail flows and monitoring services are
excluded.
