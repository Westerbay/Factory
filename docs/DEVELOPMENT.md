# Development

## Prerequisites

- Node.js 24.22.1, pinned in `.node-version`, and pnpm 12.11.2.
- Rust 1.90.0 with rustfmt, clippy and a native linker/platform SDK.
- On Windows, MSVC C++ build tools and the Windows SDK, preferably from Developer PowerShell.
- Local Docker Engine API 1.48+ (Engine 28+) with isolated bridge support and Compose
  for database setup, environment preparation and integration;
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

Without identity configuration, owned PostgreSQL and an initialized schema, the
application remains accessible but sign-in is unavailable. `/health/live` means the
process responds; `/health/ready` reflects configured identity/storage readiness.
A successful liveness response does not establish usable authentication or a live
GitHub exchange. Runtime startup does not create infrastructure, initialize the database or
create configuration files. API startup reads `.env` from the working directory
(the repository root for the pnpm commands). After correcting startup configuration,
database ownership or a missing schema, restart the API so it can compose the
identity runtime again. This startup requirement is separate from recovery of an
already composed environment supervisor after a temporary PostgreSQL failure.

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
4. With Docker running, execute `pnpm db:setup`
   from the repository root. These explicit commands prepare only the owned local database.
5. Configure the initial administrator:

   ```bash
   pnpm factory admin setup --provider github --username YOUR_GITHUB_USERNAME
   ```

   The CLI verifies the owned database and initialized schema, resolves the public GitHub
   account, and displays its canonical username, numeric ID and profile URL. Type the
   displayed username to confirm; Enter, EOF or a different answer cancels without writes.
   It stores the stable account ID, not the username or email, and requires no GitHub token.
   An existing pending account can become the administrator. Existing sessions are revoked
   on promotion, and OAuth attempts started before setup cannot grant access.

6. Start or restart the API and frontend with `pnpm dev`. Open
   `http://localhost:3000/sign-in` and sign in with the selected GitHub account.

Administrator setup never initializes the database or creates a browser session. It refuses to replace
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
and required database schema. Do not share the credential values when diagnosing it.

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
pnpm environments:test
pnpm environments:integration
```

`pnpm check` runs formatting, lint, types, behavioral tests, dependency boundaries,
documentation links, OpenAPI parity and builds, including Cargo formatting, Clippy and
tests. It does not require provider credentials, Docker or a database. `web:check`
targets frontend types/tests/build; `rust:build` produces a locked release build.
Node and Cargo lockfiles remain versioned. `pnpm format` applies formatting.
`pnpm test:dependency-security` checks the installed dependency backports and runs
within `pnpm test`. Keep the [patches](../patches/README.md) with the lockfile when
installing the workspace; scanners may still report their original versions.
Web tests live under `apps/web/test/`, grouped by feature. Vitest discovers both
TypeScript and TSX tests there.

GitHub Actions runs TypeScript and Rust checks in parallel. Pull requests limited to
`apps/web/`, `docs/` and `README.md` skip Docker and database integrations; all other
pull requests run them. Pushes to `main`, nightly runs and manual runs execute the full
integrations. A separate workflow builds Rust release binaries on those same events,
without adding a skipped job to pull requests. Branch pushes without a pull request do
not start either workflow. The `check` job requires the applicable foundation jobs to
succeed; the release workflow reports its result separately. The Docker integration builds
the real workspace and egress images and may remain slow on a cold runner. `pnpm check`
locally continues to include the release build.

`environments:test` runs deterministic preparation-tool regressions and is included in
routine checks. `environments:integration` explicitly builds a fixture image and runs
real Docker environment isolation, persistence and resource-limit tests.

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
persistence, SQLx identity/environment persistence, API composition with synthetic
identities, real Docker terminal/account behavior and isolated egress/DNS fixtures.
Fixtures use unique verified projects and isolated volumes, not the development target.
A synthetic provider is test-only; production has no fixture-provider switch.
These regressions do not replace human acceptance or prove live OAuth behavior.

For the PostgreSQL subset, run `pnpm db:build` followed by
`node scripts/database/identity-integration.mjs`. Its disposable database covers
supervisor connection loss and competing ownership, including recovery through the
same HTTP router without restarting the API.

## Local PostgreSQL

Database commands operate only on the verified local Docker target. They do not read or
change `.env` files.

```bash
pnpm db:setup   # start the local PostgreSQL service and initialize an empty schema
pnpm db:status  # inspect database readiness
pnpm db:reset   # delete local database contents and recreate the initial schema
pnpm db:stop    # stop PostgreSQL and retain its data
```

Stop the API before `db:reset`. Reset permanently removes local users, sessions,
conversations and other database records. Existing Docker containers and home volumes are
not removed; they become unassociated with the new database. Configure the administrator
again after reset. No old-database upgrade or data migration is provided.

Development uses project `factory-local`, database `factory_local`, user `factory`,
port `127.0.0.1:55432` and volume `factory-local_postgres_data`. The password
`local-fixture-only` is public synthetic local configuration. The ignored
`.factory/database-owner` holds the ownership marker. Unknown resources are never adopted.
Repeated setup preserves an initialized database; an incompatible database requires an
explicit reset.

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

The single initial schema is [schema.sql](../crates/database/schema.sql). Change it
directly during development and run integration checks against an isolated fixture.

## Local environments

After preparing the owned database, stop the API before explicitly
preparing the shared images so it cannot create resources from an older manifest:

```bash
pnpm environments:prepare
```

This operator command builds the pinned [workspace image](../infra/environments/Dockerfile)
and [egress proxy](../infra/egress/Dockerfile), so initial preparation requires network
access and may take several minutes. The workspace contains zsh with Oh My Zsh, tmux and native Codex
0.160.0; no host Codex installation or credentials are imported. Preparation
stores the stable installation UUID in `.factory/environments/installation.json` and
the immutable workspace image ID/policy in `prepared.json` and the proxy metadata in
`egress.json`. Preserve this ignored directory while
its resources exist; do not regenerate its identity or delete `supervisor.lock` to bypass
a conflict. Repeated preparation retains the installation identity. Existing environments
keep their recorded image and policy; preparation does not replace them.
If an existing proxy differs from the proposed image or host-address configuration,
preparation preserves the previous manifest and refuses publication. Follow the
[explicit proxy maintenance procedure](../infra/egress/README.md) before retrying;
preparation never stops or removes a running proxy on the operator's behalf.
Each Docker CLI invocation uses its own temporary configuration directory. Cleanup
removes that directory and generated Buildx metadata without following symbolic links;
it does not use or remove the operator's Docker configuration.

Restart the API after preparation or configuration changes. API startup, account
activation and status reads do not create user containers. Verified workspace entry
automatically initializes a missing environment or starts a stopped one after checking
for recoverable work. A running environment is reused. Stop retains `/home/factory` and
its `Documents` working directory. There is no user-volume deletion/reset command.
Investigate a missing recorded volume or uncertain provisioning state instead of creating
a substitute.

Factory no longer configures a maximum number of environments or container CPU, memory
or process ceilings. The old `FACTORY_ENV_MAX_RUNNING`, `FACTORY_ENV_IDLE_MINUTES`,
`FACTORY_ENV_MEMORY_MIB`, `FACTORY_ENV_CPUS` and `FACTORY_ENV_PIDS` values are ignored and
can be removed from operator configuration. Host and Docker Desktop limits still apply.
Containers retain a fixed non-root user, read-only root, bounded tmpfs/logs, dropped
capabilities, no-new-privileges, isolated per-user internal networks and no published
ports. They receive neither
host bind mounts, platform credentials nor the Docker socket. Named volumes have no
portable disk quota. Only the approved Codex HTTPS destinations pass through the shared
proxy; direct Internet, host/LAN and other-user access remain blocked. Package downloads,
GitHub cloning and development-server publishing are not enabled in this increment.

A prepared image with a different policy is not applied to existing containers. During
local development, a fresh database creates new environments; existing Docker homes are
not deleted by the database reset.

Missing preparation leaves the workspace unavailable while otherwise-ready
authentication and administration remain usable. Docker failures produce environment
errors and retries. A PostgreSQL timeout or lost supervisor connection closes runtime
admission while the API automatically replaces the failed supervisor. It retains the
installation file lock, lets old reconciliation work settle, then retries acquisition
of a fresh dedicated database connection and advisory lock. Attempts are sequential
and bounded, with delays capped at 30 seconds. Once PostgreSQL responds and the lock is
available, supervision resumes without restarting the API; environment state must be
inspected again before interactive access resumes. Existing browser chat content stays
in memory, while server history and other workspace operations may be unavailable.
Interrupted turns and uncertain model/run reservations keep their existing explicit
recovery requirements.

Client deadlines also cover runtime queries and pool recycling. A nonresponding socket
is discarded so identity, chat and supervision can obtain replacement connections;
timed-out writes are not automatically replayed.

The logs distinguish `api.environments.recovery_pending` from
`api.environments.ownership_reacquired`. `api.environments.recovery_stopped` indicates a
permanent configuration/resource-ownership error or worker failure: investigate it
before restarting. Startup configuration and preparation are not reread by recovery.
Run only one environment supervisor per installation. Do not remove lock files or
change installation metadata to force a second instance; another advisory-lock holder
must keep this instance unavailable.

Browser presence is not a container lease. Closing the page, losing connectivity, signing
out or allowing a login session to expire leaves the environment running. Polling resumes
when an authenticated page returns; status polling itself does not mutate lifecycle state.
Automatic workspace entry is consumed once per open app session, even if the environment
is already running. Focus checks, polling and route remounts cannot reverse a later Stop.
A new tab or browser reload begins a new entry. Failed starts require contextual retry;
outstanding recovery keeps the environment stopped. **Stop environment** in the profile
menu requires confirmation, and **Return to workspace** explicitly resumes after shutdown
and required recovery. Explicit Stop and account suspension persist stopped intent;
suspension also revokes access atomically.
Actual shutdown requires available API/Docker supervision. API shutdown leaves containers
running and restart resumes supervision. Host shutdown, Docker shutdown or a process
failure can still interrupt work. An uncertain conversation run requires an explicit
recovery after a fresh stopped observation and a settled launch; it is never replayed
automatically. An unsettled launch remains fenced even after Stop.

The runtime uses Bollard against the local Linux socket or Windows Docker named pipe,
independently of `DOCKER_HOST` or stored contexts. CI runs real environment integration on
Ubuntu with Docker Engine; local validation also covers Windows Docker Desktop with Linux
containers. The policy uses an OCI annotation (Docker API 1.43+) to disable systemd's
per-container `DefaultTasksMax` fallback when Docker normalizes the unlimited PID setting.
It does not modify host systemd configuration; host and ancestor resource constraints still apply.
WSL behavior remains unverified, and installation/checkout identities must
not be assumed interchangeable. Integration commands create their own labeled fixtures
and verify ownership before cleanup; they never prune or remove user volumes.

## Terminal and Codex account

Wait for automatic workspace preparation and connect Codex. The header's terminal icon
opens or folds the panel; opening loads xterm and attaches to its zsh tmux shell in
`/home/factory/Documents`. Folding preserves the attached display and shell. Drag the
separator to resize, or focus it and use arrow keys; Enter folds the terminal. Narrow
screens switch between chat and terminal. The current image provides a colored prompt
without overwriting user dotfiles. Transport failure offers **Reconnect**. A second tab
must choose **Take over terminal** before it can type. Closing the browser or restarting
the API does not stop the shell; stopping the environment or Docker does. Files survive
Stop/start, but running processes do not. Unsent keystrokes are never replayed.

Use Ctrl+Shift+M to leave terminal input and reach its controls. Screen reader mode is
optional. The mobile toolbar provides Ctrl, Enter, Tab, Ctrl+C, Ctrl+D and Esc. The terminal
header's maximize control fills the app viewport; restore or Escape returns to the
previous layout. The workspace header toggles the chats sidebar. Neither action replaces
the terminal instance. Session checks on focus run in the background while the verified
session remains unexpired. The shell remains independent of this browser lifecycle.

Choose **Codex** on the provider screen to request a device code. Copy it and open the displayed official
OpenAI link in a separate tab, then authorize your personal ChatGPT/Codex account there.
Do not paste a password, API key or credentials into Factory. Keep the code private and
cancel it if it is no longer needed. The connection controller polls pending authorization every two
seconds while visible and shows Connected only after provider confirmation and account
verification. The reported plan does not guarantee access to a particular model.

Only the initiating Factory session can retrieve a pending code. Reloading that page
recovers its current state without starting another login. Expiry, logout or suspension
closes access and attempts to cancel pending authorization. After an API crash, an
abandoned controller may keep the account lock until its watchdog exits, at most ten
minutes; use the contextual **Check again** action after recovery rather than killing
arbitrary container processes.

Codex stores credentials under `/home/factory/.codex` with private permissions. The API
does not read host credentials or store provider tokens in PostgreSQL or browser storage.
In profile **Settings**, **Disconnect Codex** clears the saved sign-in through the account protocol and preserves
the rest of the home. It is blocked for active or uncertain Factory runs. This does not
prove already-running manual Codex requests stopped;
use **Stop environment** when all processes must end. Factory sign-out leaves an already
connected Codex account saved in its owner's home.

The initial proxy allows only `auth.openai.com`, `chatgpt.com` and `api.openai.com` over
HTTPS CONNECT on port 443. Direct egress and private destinations are refused. If the
proxy is down, provider settings report unavailable while local terminal work continues;
local account cancellation/logout does not require the proxy. No HTTP endpoint can
open an arbitrary Docker exec or target another user's container.

After connection, create a conversation, select a model and send a message. Responses
stream into the chat; completed messages and run state are retained. The visible history
is bounded and indicates when older entries are omitted; the conversation list can load
older pages. Questions need an explicit
reply; a submitted answer can remain claimed or uncertain until provider confirmation
and is not automatically resent. Codex runs with full access inside the user's container and does not
request per-action approval: its commands can read, modify or delete the user's home,
including account credentials. A displayed model is not proof of account entitlement.
Provider calls may consume the user's model allowance.

If stopping a run does not confirm cleanup, it remains uncertain and blocks further work.
Use **Stop environment** in the profile, wait for a verified stopped state, then recover
the run from the workspace before choosing **Return to workspace**. Recovery also requires a
conclusively settled launch request. If launch settlement remains unknown, recovery can
fail and capacity remains reserved even after Stop. No verified operator recovery exists
for this case. Recovery releases local
ownership and cannot undo external effects. A reloaded page can list recoverable runs
without exposing their contents while the environment is stopped. A fresh, error-free
stopped observation at a later generation releases a settled uncertain model-probe
reservation automatically, even for a suspended user, without changing environment intent
or session state. An unsettled probe remains reserved after Stop. If model access remains unavailable after retrying, **Restore model access** checks
the same stopped-state proof before restarting.

The integration commands exercise real PTYs, reattachment after adapter restart, a
detached 4-MiB output producer, account-controller cleanup with synthetic cache data,
controlled DNS/private-address refusals. They neither
authorize a real account nor consume model allowance. A successful real login and a
small intentional Codex chat turn remain human acceptance.

## Execution reference and Codex diagnostics

The TypeScript execution packages are maintained as porting references. They are not
wired to member routes or a web terminal. Their deterministic checks remain in
`pnpm check`; package JavaScript exports use built `dist` entries, while TypeScript
consumers resolve source types. Build packages before invoking their JavaScript directly.

The retained TypeScript provider adapter targets **codex-cli 0.160.0**. These diagnostics
do not install or upgrade the host CLI. An intentional diagnostic must select an existing
compatible executable by absolute path. Sign-in for these host-only diagnostics belongs
to Codex's supported UI/CLI; these
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
Keep intercepted HTTP/WebSocket checks in a separate headless browser session and close
that session afterward. Do not hand a mocked or viewport-overridden browser to a person
as the running application.

With separately authorized OAuth configuration and two chosen GitHub accounts, verify:

1. Configure the first administrator with the CLI, then sign in; the second account becomes
   pending and cannot enter the member space.
2. Activation enables a fresh sign-in. The member cannot open user administration or
   invoke its API. Check readable errors, keyboard navigation and narrow layouts.
3. Suspension requires confirmation and revokes the member's sessions. Reactivation
   requires another fresh sign-in. Exercise conflict feedback with two admin tabs.
4. Sign-out invalidates another tab. Transient API loss or offline mode preserves an
   already verified chat in memory with a notice, only until its existing expiry.
   Initial admission still needs the server; no persisted session is restored.
5. Returning to a tab preserves the selected chat, unsent draft, pane layout and manual
   scroll during background verification, without showing access/provider loaders or
   recreating the terminal. An open private dialog stays usable. A different session,
   expiry or authorization refusal immediately removes the private UI and dialog.

After explicit image preparation and database setup, verify each account sees only
its own environment. Check automatic first initialization, automatic stopped-environment
entry, observed Running, profile Stop confirmation and explicit Return to workspace with
retained data using owned fixtures. Confirm focus changes and route remounts do not undo
Stop, and failed or pending recovery blocks automatic start. There is no file editor in
the member UI. Start more than two
environments and verify that closing the page or expiring a login session leaves them
running. Check explicit stop and account suspension. Confirm polling pauses on
hidden/offline pages and stops at logout, and inspect keyboard/narrow-screen
behavior. Record actual platforms and outcomes; synthetic checks do not establish human
acceptance or production behavior.

Using the built preview in a supported Chromium browser, install and reopen the PWA,
check the public offline fallback, then build a visible local change. Verify the update
notice appears after focus/reconnection and only an explicit action reloads the chosen
tab. Check that another tab is not forced to reload and no private API/OAuth content
appears in CacheStorage. Record actual observations and limits locally.

For terminal/account acceptance, use a deliberately chosen disposable directory in the
user's home. Connect the personal account through the code flow, verify the reported
account, then intentionally run a small harmless Codex request in the chat. This may
consume model allowance. Check terminal folding, pointer/keyboard resizing, detached work
and reconnection, cancellation of a second login, saved-account persistence after
Stop/start and explicit account disconnect in Settings.
Check sidebar collapse and terminal maximize/restore, Escape, keyboard focus, retained
split width and terminal input, including reduced motion and narrow screens. Scrollbars
should stay discreet while remaining usable by pointer and keyboard.
Verify Markdown emphasis, links, lists, tables and fenced code in completed and streaming
replies, including narrow screens; embedded HTML and images must stay inert.
Verify model selection, streamed output, questions, cancellation and recovery where
available, and confirm the full-access disclosure. Verify cross-user separation and the
fresh environment on a disposable home before using it for important work. Synthetic
fixtures and UI projections cannot establish live model access or cleanup of manually
launched agents.
