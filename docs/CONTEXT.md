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
image first. A visible member page renews the idle lease; expiration and suspension
request a stop while the API and Docker are available. There is no independent watchdog
when the API is stopped, and named volumes have no portable disk quota.

The intended next stages connect GitHub projects and a personal ChatGPT/Codex
subscription, and provide a terminal with global skills. These capabilities are not
connected to the web application yet. GitLab, Azure and Claude remain future adapters;
GitHub identity currently grants no repository access.

The TypeScript core, application, contracts, Codex adapter and PostgreSQL execution store
remain tested references for the later Rust execution port. Their existing tables and
Drizzle migration ledger are preserved. Historical runs have no web-user ownership and
must not be assigned or exposed to users by inference. The separate SQLx identity schema
does not rewrite that history. Its additive environment migration is manual; an existing
unmigrated database remains Pending and cannot serve identity until the operator applies
it. Missing image preparation affects only environment availability.

Local PostgreSQL ownership checks support Linux Docker Engine and Windows Docker Desktop
using exact checkout paths and the local socket or named pipe. Windows database support
does not establish live Codex execution or native cleanup. The Codex reference adapter
targets CLI 0.160.0, but no agent is started by the web runtime or routine checks.
The new environment adapter uses Rust/Bollard directly against that local Docker
transport. CI exercises environment integration on Ubuntu with Docker Engine, alongside
local Windows Docker Desktop validation with Linux containers. WSL remains unverified.

This increment does not deploy to a VPS, register an OAuth application or configure real
credentials. Unit and integration regressions use synthetic principals and owned local
fixtures. Live GitHub sign-in with two accounts and installed-PWA behavior require
separate human acceptance; temporary QA scripts and observations remain local and ignored.
Performance under concurrent agents and production hosting requirements remain unverified.

See [architecture](ARCHITECTURE.md), [product](PRODUCT.md) and [development](DEVELOPMENT.md).
