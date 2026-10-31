# Factory

Factory is a private React/TypeScript web application with a Rust API. Its current
foundation provides GitHub sign-in, administrator-controlled admission, revocable
server sessions, a user-access backoffice and private local Docker environments,
with Codex conversations, a persistent web terminal and personal account connection.
The frontend builds to static assets
hosted independently of Rust and is installable as a PWA.

The member workspace provides Codex conversations and an xterm terminal backed by Docker
exec and tmux. Codex runs with full access inside the user container, including its home
and credentials. Repository selection, Claude and global skills remain future work.
The existing TypeScript execution packages remain porting references;
their history is not assigned or exposed to web users. Hosting and deployment are
outside the current local increment.

## Get started

Use Node.js 24.22.1, pnpm 12.11.2 and Rust 1.90.0 with rustfmt, clippy and a native
linker. Windows Rust builds need the MSVC C++ tools and Windows SDK. After
`pnpm install --frozen-lockfile`, copy [.env.example](.env.example) to `.env` at the
repository root and fill in the two GitHub OAuth values. See the
[GitHub OAuth setup](docs/DEVELOPMENT.md#github-oauth-setup) for registration and the
administrator CLI setup. Existing terminal variables override `.env` values.

With Docker running, initialize the owned local database explicitly:

```bash
pnpm db:setup
pnpm factory admin setup --provider github --username YOUR_GITHUB_USERNAME
pnpm environments:prepare
```

The CLI displays the selected profile and asks you to type its username to confirm.
It stores the stable provider ID and refuses to replace an existing administrator.
Environment preparation builds pinned workspace and egress-proxy images and records
their immutable identities in `.factory/environments`; it does not create user containers.
Workspace entry automatically initializes the user's container and persistent home or
starts an existing stopped environment after checking recovery. Profile **Stop environment**
requires confirmation, preserves saved files and ends running processes; it stays stopped
until **Return to workspace** or a new browser entry. Keep the preparation directory with
its Docker resources. Docker Engine API 1.48+ and isolated bridge support are required;
the runtime refuses unsupported isolation instead of falling back.
Then run these in separate terminals:

```bash
pnpm dev:api
pnpm dev:web
```

`pnpm dev` starts both processes together. After `pnpm build`, `pnpm start` runs
the release API and local static preview together; it is not a deployment command.

Open `http://localhost:3000`. The browser calls the separate API at
`http://localhost:3001`. The application can start without provider credentials;
sign-in remains unavailable until OAuth configuration, the verified owned PostgreSQL
target and initial database schema are ready. Runtime startup does not change the database.
For a clean local database, stop the API and run `pnpm db:reset`. This deletes all local
users, sessions and conversations. Existing Docker homes and containers are not removed;
they are no longer associated with the new database. Set up the administrator again.
Missing environment preparation leaves only environments unavailable when identity is otherwise ready.

After workspace preparation, choose **Codex** and authorize your personal account. A saved
confirmed account opens chat directly. The terminal icon folds or opens the side panel;
drag its separator to resize, or maximize it from its header. The chats sidebar can
collapse independently. Returning to the tab checks the session in the background.
Full-access Codex
can read and modify the container user's files and credentials without per-action
approval. Closing Factory leaves the shell running; reconnecting never replays unsent input. See the
[terminal and account instructions](docs/DEVELOPMENT.md#terminal-and-codex-account).

See [development](docs/DEVELOPMENT.md) for explicit local database setup, API
configuration, [persistent environment lifecycle](docs/DEVELOPMENT.md#local-environments),
built PWA inspection and human acceptance. No real OAuth application
registration or credential setup is performed by installation or routine checks.

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
- [Reuse and attribution](docs/REUSE.md)

Derived from WebApp Template under the [MIT license](LICENSE), with selected mechanisms
from its Rust variant. Better Auth, password/mail flows and monitoring services are
excluded.
