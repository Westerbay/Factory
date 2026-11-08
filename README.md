# Factory

Factory is a self-hosted React/TypeScript web application with a Rust API. Its current
foundation provides GitHub sign-in, administrator-controlled admission, revocable
server sessions, a user-access backoffice and private local Docker environments,
with Codex and Claude Code conversations, a persistent web terminal and personal account connections.
The frontend builds to static assets
hosted independently of Rust and is installable as a PWA.

The member workspace provides Codex and Claude Code conversations and an xterm terminal backed by Docker
exec and tmux. Both agents run with full access inside the user container, including its home
and credentials. Members can authorize GitHub access, import projects and start chats and
independent persistent terminals in each repository. Global skills remain future work.
The existing TypeScript execution packages remain porting references;
their history is not assigned or exposed to web users.

## Install and run locally

Install Node.js 24.22.1, pnpm 12.11.2, Rust 1.90.0 with rustfmt, clippy and a native
linker, and Docker Engine 28 or newer with Compose and isolated bridge support.
On Windows, use Docker Desktop with Linux containers, the MSVC C++ tools and the
Windows SDK.

Clone the repository and install dependencies.

```sh
git clone https://github.com/Westerbay/Factory.git
cd Factory
pnpm install --frozen-lockfile
```

Register a GitHub OAuth App with homepage `http://localhost:3000` and callback
`http://localhost:3001/api/auth/github/callback`. Copy [.env.example](.env.example)
to `.env` at the repository root, then set `FACTORY_GITHUB_CLIENT_ID` and
`FACTORY_GITHUB_CLIENT_SECRET`. See the [local OAuth setup](docs/DEVELOPMENT.md#github-oauth-setup)
for details.

With Docker running, prepare the database, confirm the initial administrator and
build the workspace images.

```sh
pnpm db:setup
pnpm factory admin setup --provider github --username YOUR_GITHUB_USERNAME
pnpm environments:prepare
pnpm dev
```

Open `http://localhost:3000` and sign in as the configured administrator. Factory
prepares your persistent workspace on entry. Connect a personal Codex or Claude
Code account to run an agent. Both providers run with full access to the member's
container files and credentials. See the [development guide](docs/DEVELOPMENT.md)
for troubleshooting, environment lifecycle and provider setup.

## Deploy to production

Run the API, PostgreSQL and agent runtime on a dedicated Linux host with Docker
Compose. A published GitHub Release builds the server images and deployment
bundle. The bundle includes the static frontend, which Caddy serves alongside
the API at the configured HTTPS address. The
[production runbook](docs/DEPLOYMENT.md) covers server prerequisites, local secret
configuration, first sign-in, separate frontend hosting and recovery.

## Validation

```bash
pnpm check
pnpm test:integration
```

`check` covers formatting, lint, types, module boundaries, behavioral regressions,
OpenAPI parity, documentation links and builds without Docker or provider credentials.
`test:integration` explicitly requires local Docker and uses isolated, identity-verified
fixtures; it does not modify the development database or call a model.

Human QA and live GitHub/Codex/PWA acceptance remain separate. Temporary acceptance scripts
and captures stay in ignored local scratch; meaningful unit and integration regressions
remain versioned.

## Documentation

- [Context](docs/CONTEXT.md)
- [Product](docs/PRODUCT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Design](docs/DESIGN.md)
- [Development](docs/DEVELOPMENT.md)
- [Deployment](docs/DEPLOYMENT.md)
- [Reuse and attribution](docs/REUSE.md)

Derived from WebApp Template under the [MIT license](LICENSE), with selected mechanisms
from its Rust variant. Better Auth, password/mail flows and monitoring services are
excluded.
