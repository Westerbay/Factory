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

The PWA caches public static assets and a public offline page, never API/OAuth responses.
It cannot restore a private workspace offline. An already verified tab keeps its current
chat in memory through a temporary connection loss, only until its fixed session expiry.
Routine focus checks run in the background without replacing the workspace. Updates are
detected in the background, with an explicit reload action for each open tab.

An active member entering the workspace automatically initializes a missing environment
or starts a stopped one after checking for recoverable work. Creating or activating a
user alone creates no container. The first initialization creates a private persistent
home volume; the operator explicitly prepares a common image first. Entry makes one
automatic start attempt, with explicit retry or recovery after failure. Stop in the
profile menu requires confirmation and suppresses automatic restart for that open app
session. Environments keep running after the browser closes or login sessions expire.
Explicit stop or account suspension records stop intent; runtime shutdown follows while
the API and Docker are available. Factory imposes no count, CPU, memory or process quota;
host and Docker Desktop limits still apply. Named volumes have no portable disk quota.

Members can open a persistent xterm terminal through the Rust API and Docker exec.
tmux owns the shell independently of browser and API connections. Explicit takeover
allows only one attached writer. Codex device authorization connects the member's
personal ChatGPT/Codex account inside that same home. Once connected, members can use
a web conversation with Markdown-rendered streamed Codex output, model selection, questions and a terminal
side panel. The graphite interface uses self-hosted Space Mono, a conversation sidebar
and a bottom-left GitHub profile menu. The chats sidebar can collapse. A terminal icon
toggles the panel; its separator resizes it by pointer or keyboard and its header can
maximize it across the app without reattaching. Setup and account details appear only when needed,
with provider settings available from the profile. Conversations and completed messages
are private to their owner. Codex runs
in full-access mode inside the user's container; commands can access that user's home
and credentials without per-action approval. Repository selection and global skills
remain future work. Claude and multiple accounts per provider are not implemented;
GitHub identity currently grants no repository access.

Environments use an isolated internal network per user and an installation-owned HTTPS
CONNECT proxy. Only approved Codex destinations are allowed. Existing containers are not
silently replaced when the image changes. A local database reset removes database records
but leaves Docker homes untouched and unassociated.

The TypeScript execution packages remain tested porting references. Fresh installations
use one initial database schema; no previous database upgrade is supported.

Local PostgreSQL ownership checks support Linux Docker Engine and Windows Docker Desktop
using exact checkout paths and the local socket or named pipe. Windows database support
alone does not establish live Codex execution. The workspace image pins native Codex
0.160.0. The web runtime uses app-server for account operations and conversation turns.
Routine checks use synthetic credentials and do not call a model; live provider access
and a complete conversation still require human acceptance.
The new environment adapter uses Rust/Bollard directly against that local Docker
transport. CI exercises environment integration on Ubuntu with Docker Engine, alongside
local Windows Docker Desktop validation with Linux containers. WSL remains unverified.

This increment does not deploy to a VPS, register an OAuth application or configure real
credentials. Unit and integration regressions use synthetic principals and owned local
fixtures. Live GitHub sign-in with two accounts and installed-PWA behavior require
separate human acceptance; temporary QA scripts and observations remain local and ignored.
Performance under concurrent agents and production hosting requirements remain unverified.

See [architecture](ARCHITECTURE.md), [product](PRODUCT.md) and [development](DEVELOPMENT.md).
