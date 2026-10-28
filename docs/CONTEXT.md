# Context

Factory is moving from Electron to a private web/PWA application with a Rust backend.
React/TypeScript compiles to static assets hosted independently of the API. The target
keeps agent execution outside the HTTP process, with a persistent environment per user,
GitHub-backed projects and global skills. GitHub OAuth, administrator-controlled access
and personal Codex subscription connection are the selected direction; their behavior
is not implemented by the initial web foundation.

The current web increment supplies only the static React shell, an Axum health API,
explicit loopback origins and build/dependency checks. It does not connect to the database
or an agent. API readiness stays unavailable until the next identity/storage work.
The desktop remains runnable until its planned retirement; existing execution semantics,
history and user changes are preserved. Delivery remains local with feedback per code task.

The existing desktop adapts the English WebApp Template foundation. Provider-neutral
execution rules and validated desktop contracts are implemented and exercised with
test-only deterministic adapters. PostgreSQL persistence, guarded local setup, explicit
restart recovery and isolated database integration checks are implemented. The Codex
0.160.0 app-server adapter now maps workspace skills, conversations, native child events,
questions and explicit permissions through the provider-neutral runtime port. A separate Electron utility process now composes those implementations with the execution
service. The operator interface selects native-granted workspaces, starts tasks, observes
agent relationships and output, handles explicit questions/permissions, stops execution
and reads persisted history. The source template stays unchanged. Development is Linux-first;
local PostgreSQL setup and runtime inspection also support Windows Docker Desktop through
its local named pipe and Windows checkout paths. Native Codex acceptance on Windows is
not established by this database compatibility work. Distributing
installers and running a VPS service are outside this increment.

Local execution describes the agent tools and host. A provider may still call a remote
model; offline inference is not promised. Routine checks never access a provider or account. The explicit Codex readiness command
can start the installed app-server and inspect managed sign-in metadata without refreshing
tokens or requesting inference. The built desktop is exercised with an explicitly test-only provider and isolated real
PostgreSQL; native Codex execution and cleanup still require bounded live acceptance.
Human acceptance and its temporary scripts remain local and ignored; unit and integration regressions remain versioned.
The current skill port exposes enabled names and paths only, without descriptions or
disabled catalog entries. Conversation references remain session-local.

See [architecture](ARCHITECTURE.md), [product](PRODUCT.md) and [development](DEVELOPMENT.md).
