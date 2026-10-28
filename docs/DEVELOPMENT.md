# Development

## Prerequisites

- Node.js 24.22.1, pinned in `.node-version`, and pnpm 12.11.2.
- Rust 1.90.0 with rustfmt, clippy and a native linker/platform SDK.
- On Windows, MSVC C++ build tools and the Windows SDK, preferably from Developer PowerShell.
- Local Docker Engine and Compose for explicit database setup and integration;
  on Windows, Docker Desktop running Linux containers.
- Network access for initial dependency/toolchain downloads.

```bash
pnpm install --frozen-lockfile
```

The Rust wrapper uses installed Cargo by default. If `.factory/toolchains/cargo/bin`
contains a project-local installation, it selects that installation with matching
Cargo/Rustup homes. This ignored directory is optional, not required on a fresh checkout.
Scripts do not change global defaults. Rustup may download the pinned toolchain and
components into its selected home when missing. Missing tools remain visible failures.

## Web foundation

Run the independent API and frontend in separate terminals from the repository root:

```bash
pnpm dev:api
pnpm dev:web
```

`pnpm dev` starts these two processes together. `pnpm build` followed by `pnpm start`
runs the release API and static preview locally. Neither command starts Electron.

Open `http://localhost:3000`, using `localhost` consistently for the frontend and API.
The API binds to `127.0.0.1:3001` but is addressed by the browser as
`http://localhost:3001`. Do not mix `localhost` and `127.0.0.1`: origin checks and
same-site cookies depend on consistent host configuration. Rust serves HTTP API routes,
not frontend files. The browser app is static; no SSR server is used.

Without identity configuration, owned PostgreSQL and applied identity migrations, the
application remains accessible but sign-in is unavailable. `/health/live` means the
process responds; `/health/ready` reflects configured identity/storage readiness.
A successful liveness response does not establish usable authentication or a live
GitHub exchange. Runtime startup does not create infrastructure, apply migrations or
create configuration files. API startup reads `.env` from the working directory
(the repository root for the pnpm commands). After correcting startup configuration,
database ownership or missing migrations, restart the API so it can compose the
identity runtime again.

### Explicit configuration

Copy [the example](../.env.example) to `.env` in the repository root, then fill in
the two GitHub OAuth values. On PowerShell, use `Copy-Item .env.example .env` only if
`.env` does not already exist. On macOS/Linux, use `cp -n .env.example .env`.
The file is ignored by Git. The API and administrator CLI read this exact file at startup, with existing
process variables taking precedence, including explicitly empty values. It does not
search parent directories or load `.env.local` / mode-specific variants. A missing file
is allowed; malformed or unreadable files fail startup with a sanitized error code.
Parsing does not mutate the process environment. Restart the API after editing the file.

| Variable                       | Default or purpose                                                       |
| ------------------------------ | ------------------------------------------------------------------------ |
| `FACTORY_API_BIND`             | `127.0.0.1:3001`; loopback socket only                                   |
| `FACTORY_WEB_ORIGIN`           | `http://localhost:3000`; exact allowed browser origin                    |
| `FACTORY_API_ORIGIN`           | `http://localhost:3001`; public callback/API origin, matching bind port  |
| `FACTORY_WORKSPACE_ROOT`       | Current working directory; absolute checkout path for database ownership |
| `FACTORY_GITHUB_CLIENT_ID`     | Explicit OAuth application client ID                                     |
| `FACTORY_GITHUB_CLIENT_SECRET` | Server-only OAuth client secret                                          |
| `VITE_API_BASE_URL`            | `http://localhost:3001`; public frontend build-time origin               |

Both origins must use the same local host. Current configuration accepts canonical
loopback HTTP origins without credentials, trailing slash, paths, query or fragment.
It intentionally does not support public deployment origins. Do not put secrets in
`VITE_*` variables: their values can be included in the public bundle. Vite also does
not read `.env` files: this file configures the server only. The default frontend origin
already matches the example. A custom `VITE_API_BASE_URL` is still an explicit public
frontend build-time process variable.

The client ID and client secret must be valid for identity to be enabled. The
operator CLI selects the initial administrator and persists the stable provider ID;
subsequent authorization never depends on an email, handle or first sign-in order. Keep secret values out of source, screenshots, logs and troubleshooting reports.
Routine installation and checks neither read the real `.env` nor register an OAuth app.

### GitHub OAuth setup

The current adapter uses a GitHub **OAuth App** for identity with the `read:user`
scope. Repository access and GitHub App installations are not implemented yet.

1. In GitHub, open **Settings > Developer settings > OAuth Apps > New OAuth App**
   ([registration page](https://github.com/settings/applications/new)).
2. Set **Application name** to `Factory Local`, **Homepage URL** to
   `http://localhost:3000`, and **Authorization callback URL** to
   `http://localhost:3001/api/auth/github/callback`. Leave device flow and callback
   wildcard matching disabled. Token expiration can remain enabled: the current
   adapter uses the token immediately for identity lookup, then discards it.
3. Register the application. Copy its **Client ID** into `FACTORY_GITHUB_CLIENT_ID`.
   Generate a **client secret** and put it only in `FACTORY_GITHUB_CLIENT_SECRET`
   in the local `.env`.
4. With Docker running, execute `pnpm db:setup` and `pnpm db:migrate:identity`
   from the repository root. These explicit commands prepare only the owned local database.
5. Configure the initial administrator:

   ```bash
   pnpm factory admin setup --provider github --username YOUR_GITHUB_USERNAME
   ```

   The CLI verifies the owned database and migrated schema, resolves the public GitHub
   account, and displays its canonical username, numeric ID and profile URL. Type the
   displayed username to confirm; Enter, EOF or a different answer cancels without writes.
   It stores the stable account ID, not the username or email, and requires no GitHub token.
   An existing pending account can become the administrator. Existing sessions are revoked
   on promotion, and OAuth attempts started before setup cannot grant access.

6. Start or restart the API and frontend with `pnpm dev`. Open
   `http://localhost:3000/sign-in` and sign in with the selected GitHub account.

Setup never migrates the database or creates a browser session. It refuses to replace
any existing administrator (`ADMIN_ALREADY_CONFIGURED`), including when the same command
is repeated. Concurrent setup is serialized in the database. GitHub lookup failures,
non-user accounts and absent confirmation do not create an administrator. Only GitHub is
supported currently; other `--provider` values are rejected. Run `pnpm factory --help`
for usage. Keep CLI access limited to trusted server operators.

Existing installations keep their administrator. The old `FACTORY_BOOTSTRAP_GITHUB_ID`
variable is no longer read and can be removed from local configuration. If setup has not
run yet, OAuth sign-in is available but every new account stays pending.

See GitHub's [OAuth application registration guide](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app).
If `/health/ready` returns `identity-unavailable`, inspect the API startup warning
code: it distinguishes missing OAuth configuration, an unavailable owned database
and required identity migrations. Do not share the credential values when diagnosing it.

### Built frontend and PWA

```bash
pnpm --filter @workspace/web build
pnpm preview:web
# In another terminal:
pnpm start:api
```

The frontend output is `apps/web/dist`. Vite preview is local inspection tooling,
not a production server. Development mode does not register the service worker; use
the built preview for installation, offline and update acceptance.

The worker caches public JS/CSS/icons and the public offline page only. API/OAuth
responses and private pages must not appear in CacheStorage. The entry HTML, manifest
and worker must be revalidated when configuring a future static host; hashed assets
may be cached immutably. SPA navigation fallback belongs to that static host and must
not rewrite requests at the API origin. Deployment is not part of this increment.

## Checks and generated contracts

```bash
pnpm check
pnpm web:check
pnpm rust:build
pnpm test:integration
```

`pnpm check` runs formatting, lint, types, behavioral tests, dependency boundaries,
documentation links, OpenAPI parity and builds, including Cargo formatting, Clippy and
tests. It does not require provider credentials, Docker or a database. `web:check`
targets frontend types/tests/build; `rust:build` produces a locked release build.
Node and Cargo lockfiles remain versioned. `pnpm format` applies formatting.

`pnpm execution-reference:test` compiles and exercises the temporary Electron-free
host/model/view reference in `packages/execution-reference`. It has no application
entry point or product exports. These regressions remain until the Rust execution
port supplies equivalent behavior. The former Electron window/sender smoke checks
are obsolete because their runtime has been removed.

Rust handlers and HTTP contracts own generated OpenAPI and browser types. Annotate a
handler with `#[utoipa::path(...)]`, then register it with
`OpenApiRouter::routes(utoipa_axum::routes!(handler))` in its module router. The same
declaration supplies the runtime path/method and the exported operation. Use `ToSchema`
for body types and `IntoParams` for query types; describe response statuses, security
and special headers beside the handler. These semantics cannot be inferred from the
handler's implementation. Do not maintain a separate path/schema list in the exporter.

Regenerate both artifacts after changing handlers or contracts:

```bash
pnpm api:generate
pnpm api:contract
```

Generation and parity checks are inert: they do not listen on a socket, initialize
provider credentials or connect to PostgreSQL. Inspect both generated files after
contract changes; do not hand-edit them.

`pnpm test:integration` explicitly requires Docker and exercises retained execution
persistence, SQLx identity persistence and API composition with synthetic identities.
Fixtures use unique verified projects and isolated volumes, not the development target.
A synthetic provider is test-only; production has no fixture-provider switch.
These regressions do not replace human acceptance or prove live OAuth behavior.

## Local PostgreSQL

Database setup and migrations are explicit operations. Use only the guarded commands;
do not supply a shared database or reuse template resources. No command below reads,
creates or changes a real `.env` file.

```bash
pnpm db:setup             # provision/start owned local PostgreSQL and migrate legacy execution
pnpm db:migrate:identity  # explicitly apply SQLx identity migrations to the verified local target
pnpm db:status            # read-only legacy-store readiness
pnpm db:migrate           # explicitly apply legacy execution migrations
pnpm db:stop              # stop the owned service; retain volume and marker
```

`db:status` checks the retained execution store; use API readiness for the configured
identity runtime. Applying legacy migrations alone does not initialize identity.
The runtime never performs either migration on startup. Identity objects and the SQLx
ledger live in `factory_identity`; execution tables and the Drizzle ledger remain intact.
No legacy run is assigned to a web user.

Development uses project `factory-local`, database `factory_local`, user `factory`,
port `127.0.0.1:55432` and volume `factory-local_postgres_data`. The password
`local-fixture-only` is deliberately public synthetic local configuration, not a
production credential. The ignored `.factory/database-owner` contains a non-secret UUID
ownership marker. Keep it with the checkout while its volume exists.

Setup rejects unknown preexisting resources rather than adopting them. A missing marker
with existing resources, conflicting port, Docker permission error or failed health
check requires resolving that exact condition. Do not reset or prune volumes to bypass
it. Repeated setup retains data and applies pending legacy migrations idempotently.
Keep the marker after stopping; no development reset/delete command is provided.

Every target is checked against Docker project, owner and checkout labels, container,
database identity, credentials, volume and exact loopback port before SQL. Docker is
pinned to `npipe:////./pipe/docker_engine` on Windows and
`unix:///var/run/docker.sock` on Linux, independently of stored contexts or
`DOCKER_HOST`. Compose uses the variable-free
[environment file](../scripts/database/empty-env) and a restricted environment.
On Windows, `SystemRoot`, `USERPROFILE` and `ProgramFiles` are inherited for process
startup and installed Docker CLI plugin discovery. Checkout identity accepts absolute
POSIX paths and Windows drive paths, with exact label matching.

From PowerShell, run setup and status before starting the configured API. If the named
pipe is unavailable, check that Docker Desktop is running with Linux containers.
A successful setup/status on the actual machine is needed to establish database
readiness; deterministic Windows guard checks alone do not establish it. Rust SQLx
connections reject ambient PostgreSQL connection overrides and do not read pgpass.

Integration uses a unique `factory-test-<UUID>` project and ownership token, an
OS-allocated loopback port and a separate test volume. Cleanup rechecks identities,
then removes only that fixture's exact container, volume and network. Development
resources are not deleted. If cleanup is interrupted, retain the reported failure
and inspect that exact project before any manual removal.

For retained execution schema changes, use `pnpm --filter @workspace/database generate`
and inspect SQL/snapshots. Preserve the annotated manual
`DEFERRABLE INITIALLY DEFERRED` root-ownership foreign key that Drizzle's schema DSL
cannot express. Identity migrations live under `crates/database/migrations` and use
the separate SQLx ledger. Run the appropriate integration checks after schema changes;
temporary fixture content never belongs in migrations.

## Execution reference and Codex diagnostics

The TypeScript execution packages are maintained as porting references. They are not
wired to member routes or a web terminal. Their deterministic checks remain in
`pnpm check`; package JavaScript exports use built `dist` entries, while TypeScript
consumers resolve source types. Build packages before invoking their JavaScript directly.

The retained provider adapter targets **codex-cli 0.160.0**. Factory does not install
or upgrade it. An intentional diagnostic must select an existing compatible executable
by absolute path. Sign-in belongs to Codex's supported UI/CLI outside Factory; these
diagnostics never read token files, refresh tokens, change global configuration or
accept an API key.

```bash
pnpm adapters:test
pnpm codex:protocol /absolute/path/to/codex --check
pnpm codex:probe /absolute/path/to/codex
```

`adapters:test` uses sanitized protocol fixtures without real Codex or model calls.
`codex:probe` is explicit and outside routine checks: it verifies the executable,
initializes app-server, requests `account/read` with `refreshToken:false`, prints
safe availability/capability information and closes its owned server. It does not
start a thread or a turn. Capabilities describe pinned mappings, not live delegation
or cleanup proof. Never include raw account output, stderr or credential files in reports.

Intentional regeneration uses `pnpm codex:protocol /absolute/path/to/codex`. Inspect
the generated closure and provenance hashes. A changed executable version is rejected
and requires a separate compatibility change. Generated bindings are preserved apart
from ESM import extensions and excluded from formatting.

The current adapter exposes enabled skill names/paths only, keeps conversation
correlation within one runtime instance, and rejects unsupported actionable requests
visibly. Stop requires terminal confirmation for every observed owned thread, including
children; interrupt acknowledgement or process exit alone is insufficient. Unconfirmed
cleanup blocks further execution. No live inference is authorized by routine diagnostics.

## Human acceptance

QA is accepted by a person. Agent-authored browser/acceptance scripts, screenshots and
observations stay in ignored local scratch, never in the versioned suite. Meaningful
unit/integration regressions protect implementation behavior separately.

With separately authorized OAuth configuration and two chosen GitHub accounts, verify:

1. Configure the first administrator with the CLI, then sign in; the second account becomes
   pending and cannot enter the member space.
2. Activation enables a fresh sign-in. The member cannot open user administration or
   invoke its API. Check readable errors, keyboard navigation and narrow layouts.
3. Suspension requires confirmation and revokes the member's sessions. Reactivation
   requires another fresh sign-in. Exercise conflict feedback with two admin tabs.
4. Sign-out invalidates another tab. API loss or offline mode hides private data and
   recovery rechecks the server instead of displaying a persisted session.

Using the built preview in a supported Chromium browser, install and reopen the PWA,
check the public offline fallback, then build a visible local change. Verify the update
notice appears after focus/reconnection and only an explicit action reloads the chosen
tab. Check that another tab is not forced to reload and no private API/OAuth content
appears in CacheStorage. Record actual observations and limits locally.

Live Codex acceptance belongs to the later execution increment. It may consume model
allowance and requires an explicitly chosen disposable workspace, harmless content,
deliberate questions/permissions and confirmed native cleanup. Synthetic fixtures or a
UI projection cannot establish live agent execution, provider delegation or cleanup.
