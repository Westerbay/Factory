# Context

Factory is a private web/PWA application with a Rust backend. React/TypeScript compiles
to static files hosted independently of the API; there is no frontend SSR process or
Rust static-file server. The current increment runs on loopback only.

GitHub OAuth identifies users by a stable provider identity. The operator CLI resolves a GitHub
username and stores the selected account as the first administrator. Other verified users remain pending
until that administrator activates them. The backoffice lists pending, active and
suspended users and changes member access with version checks. Sessions persist in
PostgreSQL until fixed expiry or revocation; suspension revokes all of a member's
sessions. Missing configuration or storage leaves access unavailable.

The PWA caches public static assets and a public offline page. It does not offer offline
access to private screens or cache API/OAuth responses. Updates are detected in the
background, with an explicit reload action for each open tab.

Active users can start, inspect and stop their own local Docker environment from the
member page. The first start creates a private persistent home volume; creating or
activating a user does not create a container. The operator explicitly prepares a common
image first. Environments keep running after the browser closes or login sessions expire.
Only an explicit stop or account suspension requests a stop, while the API and Docker
are available. Factory imposes no count, CPU, memory or process quota; host and Docker
Desktop limits still apply. Named volumes have no portable disk quota.

Members can open a persistent xterm terminal through the Rust API and Docker exec.
tmux owns the shell independently of browser and API connections. Explicit takeover
allows only one attached writer. Codex device authorization connects the member's
personal ChatGPT/Codex account inside that same home; Factory exposes account status,
not an agent conversation or execution history. GitHub projects and global skills remain
future work. GitLab, Azure and Claude remain future adapters; GitHub identity currently
grants no repository access.

Policy-3 environments use a dedicated isolated internal network per user and one
installation-owned HTTPS CONNECT proxy. Only the initial Codex destinations are allowed;
direct Internet, host/LAN and other-user access remain blocked. A stopped proxy makes
the account service unavailable while the local shell remains usable. Existing containers
require an explicit stopped-container upgrade that preserves their home; Start never
silently replaces them.

The TypeScript core, application, contracts, Codex adapter and PostgreSQL execution store
remain tested references for the later Rust execution port. Their existing tables and
Drizzle migration ledger are preserved. Historical runs have no web-user ownership and
must not be assigned or exposed to users by inference. The separate SQLx identity schema
does not rewrite that history. Its additive environment migration is manual; an existing
unmigrated database remains Pending and cannot serve identity until the operator applies
it. Missing image preparation affects only environment availability.

Local PostgreSQL ownership checks support Linux Docker Engine and Windows Docker Desktop
using exact checkout paths and the local socket or named pipe. Windows database support
alone does not establish live Codex execution. The workspace image pins native Codex
0.160.0. The web runtime uses short-lived app-server account operations and never issues
agent turns; a member may intentionally run Codex in the terminal. Routine checks use
synthetic credentials and do not call a model.
The new environment adapter uses Rust/Bollard directly against that local Docker
transport. CI exercises environment integration on Ubuntu with Docker Engine, alongside
local Windows Docker Desktop validation with Linux containers. WSL remains unverified.

This increment does not deploy to a VPS, register an OAuth application or configure real
credentials. Unit and integration regressions use synthetic principals and owned local
fixtures. Live GitHub sign-in with two accounts and installed-PWA behavior require
separate human acceptance; temporary QA scripts and observations remain local and ignored.
Performance under concurrent agents and production hosting requirements remain unverified.

See [architecture](ARCHITECTURE.md), [product](PRODUCT.md) and [development](DEVELOPMENT.md).
