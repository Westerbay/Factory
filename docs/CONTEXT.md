# Context

Factory is a private web/PWA application with a Rust backend. React/TypeScript compiles
to static files and Rust serves the API. Development uses loopback HTTP. The
[production deployment](DEPLOYMENT.md) uses a dedicated Linux server with Caddy,
PostgreSQL and release images from GHCR. Caddy can serve the bundled static
frontend alongside the API, or the frontend can use a separate static host.

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
After a temporary PostgreSQL supervision failure, the API reacquires exclusive ownership
and replaces the failed runtime without a process restart. Old operations remain fenced;
fresh environment inspection is required, and interrupted turns are never replayed.

Members can open a persistent xterm terminal through the Rust API and Docker exec.
tmux owns the shell independently of browser and API connections. Explicit takeover
allows only one attached writer. Members connect a personal Codex account with device
authorization or a personal Claude account through Claude Code's browser sign-in. A member
can connect both providers and use web conversations with Markdown-rendered streamed output,
model and reasoning-effort selection, questions and a terminal
side panel. The interface offers light and graphite themes, uses self-hosted Space Mono, a conversation sidebar
and a bottom-left GitHub profile menu. The chats sidebar can collapse. A terminal icon
toggles the panel; its separator resizes it by pointer or keyboard and its header can
maximize it across the app without reattaching. Setup and account details appear only when needed,
with provider settings available from the profile. Conversations and completed messages
are private to their owner. In free chat, both providers run
in full-access mode inside the user's container; commands can access that user's home
and credentials without per-action approval. GitHub sign-in also authorizes repository access,
allowing members to import accessible repositories once and start project-bound chats.
A dedicated SSH key permits Git operations with the member’s own permissions. The workspace
also offers a Features journey from product discussion and UML approval to implementation,
review and an explicitly requested draft PR. Features use a versioned workflow with task
profiles for either connected provider. Multiple accounts per provider remain future work.

Environments use an isolated internal network per user and an installation-owned HTTPS
CONNECT proxy. Approved OpenAI and Anthropic destinations and GitHub SSH on port 443 are
allowed. Existing containers are not silently replaced when the image changes. A local
database reset removes database records but leaves Docker homes untouched and unassociated.

The TypeScript execution packages remain tested porting references. Fresh installations
use the current database schema. An explicit local upgrade from schema version 4 to
version 5 preserves existing records and adds feature state.

Local PostgreSQL ownership checks support Linux Docker Engine and Windows Docker Desktop
using exact checkout paths and the local socket or named pipe. Windows database support
alone does not establish live Codex execution. The workspace image pins native Codex
0.160.0 and Claude Code 2.1.284. The web runtime uses Codex app-server and Claude Code's
streaming process protocol in the member's environment.
Routine checks use synthetic credentials and do not call a model; live provider access
and a complete conversation still require human acceptance.
The new environment adapter uses Rust/Bollard directly against that local Docker
transport. CI exercises environment integration on Ubuntu with Docker Engine, alongside
local Windows Docker Desktop validation with Linux containers. WSL remains unverified.

Unit and integration regressions use synthetic principals and owned local fixtures.
Live GitHub sign-in with two accounts, installed-PWA behavior and concurrent agent
performance require acceptance on the target deployment. Temporary QA scripts and
observations remain local and ignored.

See [architecture](ARCHITECTURE.md), [product](PRODUCT.md) and [development](DEVELOPMENT.md).

The web interface has English and French language selection in profile settings. Browser
language preferences determine the initial selection, falling back to English; an explicit
choice is stored in that browser and synchronized across open tabs. Provider responses
and terminal output are not translated.
