# Architecture

## Dependency direction

Factory retains pnpm, Turborepo and TypeScript boundaries and adds a Cargo workspace.
The new `apps/web` is a browser-only React application; `apps/api` is an Axum HTTP API.
They run independently. Vite serves static assets locally and the Rust process never
serves frontend files. Browser requests use the configured API origin, credentials and
an exact frontend CORS allowlist. CORS is not user authentication or authorization.

The Rust workspace separates `crates/core`, `crates/application`, `crates/adapters`
and `crates/database`. These crates currently reserve module ownership without identity
implementations or speculative ports. Cargo dependency checks enforce core independence,
application-to-core direction and adapter/database-to-core direction. `apps/api` owns
HTTP configuration and health only; no database connection or migration runs at startup.
Future identity DTOs will generate frontend transport types through OpenAPI.

The existing execution implementation remains TypeScript during the port. Electron
still owns its host shell and isolated React renderer until the web replacement is ready.

| Boundary               | Responsibility                    | Allowed internal dependencies            |
| ---------------------- | --------------------------------- | ---------------------------------------- |
| `packages/core`        | Pure domain and outbound ports    | Core only                                |
| `packages/application` | Use cases                         | Application and core                     |
| `packages/contracts`   | Validated desktop messages        | Contracts (Zod when needed)              |
| `packages/adapters`    | Provider implementations          | Adapters and core                        |
| `packages/database`    | PostgreSQL/Drizzle implementation | Database and core                        |
| `packages/ui`          | Reusable presentation primitives  | UI only                                  |
| Desktop renderer       | Presentation                      | Renderer, contracts and UI               |
| Desktop preload        | Narrow context bridge             | Preload and contracts; Electron boundary |
| Desktop runtime host   | Concrete assembly                 | Application, core and concrete adapters  |

Core owns [domain entities and events](../packages/core/src/model.ts), the pure
[projection reducer](../packages/core/src/reducer.ts) and
[outbound ports](../packages/core/src/ports.ts). Application owns
[execution orchestration](../packages/application/src/execution-service.ts) and depends
only on core. Contracts independently owns [Zod wire schemas](../packages/contracts/src/index.ts);
compile-time parity and application-to-schema tests protect the common shapes.
The database adapter implements RunStore; the [Codex adapter](../packages/adapters/src/codex/runtime.ts) implements AgentRuntime. The [desktop bridge schemas](../packages/contracts/src/desktop.ts) also own the narrow
preload API, workspace selections, host status and generation-correlated utility messages.

## Execution and persistence boundary

The host creates one execution service with a runtime, store, ID source and clock.
Mutations are serialized; provider effects are invoked after durable command reservation
and awaited outside the mutation queue so synchronous callbacks cannot deadlock it.
The store must atomically reserve one active root and commit each material event with
its versioned projection. Expected-version conflicts and duplicate IDs must not create
partial history. Reads return detached snapshots; history pages follow commit sequence,
not provider timestamps. Stream fragments are explicitly transient; completed messages
are durable. Repeated identical event IDs are idempotent, conflicting content is rejected.

Question handles transition pending -> submitting -> resolved only after provider
acceptance. Stop, disconnection and originating-agent termination invalidate them.
Provider completion arriving during a response waits for its acceptance. Provider-owned
permission decisions use a separate contract and never acquire consent from defaults.
Continuation on a terminal conversation creates a new run with a provider-neutral prior
run/agent reference; the adapter owns the conversation correlation. Active steering is
explicit and does not infer questions from prose.

Runtime interruption must confirm termination of all owned work including children.
A failed stop stays cancelling and can be explicitly retried. Storage failures latch
an unavailable state, initiate cleanup and retain diagnostics; the application does
not publish the failed projection as durable. A new host must reconcile persisted active
runs as interrupted before allowing execution, using the database adapter’s explicit
`recover()` operation. Connection creation and readiness never reconcile runs or apply
migrations. The utility host performs that explicit startup step. No execution is automatically replayed.

The dependency-cruiser configuration resolves TypeScript aliases and includes type-only
imports. Its regression harness checks both violations and permitted composition.

## PostgreSQL adapter

[Database lifecycle and RunStore](../packages/database/src/client.ts) adapt the template's
Drizzle/postgres-js connection pattern. Each write commits a canonical JSONB Run projection,
its sequence-numbered event and relational ownership rows in one transaction. Commit checks
the locked prior version and the core reducer's expected projection. `runs` carries the
query/index fields; agents, questions, permissions and completed messages additionally
have composite same-run foreign keys. JSONB is canonical for their complete contents;
relational rows enforce ownership, rather than owning a second set of transition rules.

A partial unique index reserves one active root across connections. Stable history uses
(run ID, version), independently of timestamps; duplicate IDs and version races roll back.
A manually added deferred root foreign key allows root creation and ownership checking
inside the same transaction. Integration checks protect this constraint and transactional
recovery rollback. Migrations and snapshots are generated by Drizzle Kit; retain the
annotated deferred constraint when changing the schema.

`ready()` only reports connection/migration state. `migrate()` is an explicit guarded
operation. `recover()` is called once by the single-instance utility host before accepting
commands, after migrations are ready: it atomically appends interruption events and
invalidates outstanding questions/permissions through the core reducer. Opening another
connection does not interrupt live work. `close()` closes the pool; operations after close
fail visibly. Driver details and SQL causes are not exposed as user-visible errors.

[Owned infrastructure helpers](../scripts/database/owned.mjs) verify the Docker project,
checkout directory, ownership token, exact container/database credentials, volume labels
and loopback port before connecting. Compose receives an explicit empty environment file
and a restricted variable set. No caller-supplied URL or real `.env` is consumed.

## Desktop composition and communication

[Main](../apps/desktop/src/main/desktop.ts) owns one sandboxed, context-isolated window,
the native directory picker and one Electron utility process. It denies new windows,
navigation, redirects, webviews and browser permissions. IPC requires the exact owned
webContents, top frame and expected document URL. Renderer commands contain host-issued
workspace/skill IDs; native canonical paths never come from a renderer command. Production
loads bundled local HTML with a strict CSP and no network requests. Development permits
only the exact loopback Vite endpoint; its trusted React preamble receives a fresh nonce.

[Preload](../apps/desktop/src/preload/index.ts) bundles the contracts for Electron's
sandbox restrictions and exposes only command, selectWorkspace, status and subscribe.
It validates outgoing commands and incoming messages and strips Electron event objects.
No generic IPC, filesystem, executable selection, Node, SQL or provider protocol is exposed.

[RuntimeClient](../apps/desktop/src/main/runtime-client.ts) owns a generation and exact
request correlation. Commands run independently: one pending multi-question response must
not prevent the next question from submitting. Count limits bound pending operations.
Ordinary commands have a bounded deadline; human-answer operations have no human deadline
and fail explicitly on host loss. Death or invalid communication rejects all pending work
and latches unavailable without automatically restarting or claiming cleanup.

The [utility worker](../apps/desktop/src/runtime/worker.ts) composes actual ExecutionService,
CodexRuntime and the PostgreSQL RunStore. It imports the original owned infrastructure helper
at its source location so Docker proof and checkout identity retain their owning context.
It accepts no arbitrary database URL or renderer executable path. The fixed local database
marker and exact Docker resources must pass inspection; startup checks readiness, calls
recover once and then accepts work. Migrations remain an explicit developer command.

[DesktopHost](../apps/desktop/src/runtime/host.ts) maps validated wire commands to use cases,
keeps workspace and current skill grants private, observes durable projections and transient
deltas, and surfaces sanitized application diagnostics. The bounded observation outbox
coalesces projections and reports skipped temporary updates. Renderer state removes live
fragments when completed messages arrive and bounds temporary accumulation. Reconnection
uses persisted snapshots/history; browsing never replays execution.

Native close offers keep-open or stop-and-close. The utility confirms service cleanup and
pool close before main disposes that exact utility. A rejected cleanup attempt releases
only its in-flight promise and preserves persistence for an explicit retry, while normal
command admission remains closed. Failure remains visible with cleanup unconfirmed; there is no broad process kill or silent in-memory fallback. Renderer death
attempts the same owned cleanup. A dead utility cannot establish termination of its native
provider descendants; subsequent startup recovery only reconciles persisted state.

The operator interface uses semantic controls and inert text. Renderer imports remain
limited to contracts and UI primitives. Adding another service requires a concrete adapter
for a real capability, composed in this utility host; no HTTP API is needed locally.
See [development](DEVELOPMENT.md) for startup, deterministic checks and explicit smoke tests.

## Codex protocol boundary

[Stdio connection](../packages/adapters/src/codex/process.ts) starts an explicitly selected
Codex 0.160.0 executable with no shell, configuration overrides or credential access.
[Transport](../packages/adapters/src/codex/transport.ts) owns bounded newline framing,
request correlation, timeouts and process lifecycle. Raw stderr, provider errors, account
metadata, reasoning and tool output never become durable events or user diagnostics.

Generated types retain a focused transitive closure from the installed CLI with provenance
hashes for additional inspected mappings. [Validated projections](../packages/adapters/src/codex/protocol.ts)
strip unrelated incoming fields. Binding regeneration is explicit and version checked.
Runtime capability flags describe mappings supported by this pinned protocol; they do not
prove the current account/model will execute native delegation or every experimental tool.

The runtime keeps thread/turn/item and server-request IDs private. Core agent identities
come from observed native spawn relationships. Terminal root success is buffered until all
observed descendants settle. Completed messages are durable; deltas remain transient.
Multiple questions share one provider response and a correlated resolved notification
after submission, guarded by stop/disconnect/terminal invalidation; pending
answers fail on stop, clear-before-submit or disconnect. Permissions allow/deny one action
without changing provider approval or sandbox policy. Unsupported secret-input, file-change approvals without inspectable patches, and richer
permission requests fail closed instead of pretending the current core model supports them.

Active conversation steering requires the exact active turn ID. Terminal continuation
creates a new run on the prior owned conversation. This correlation is session-local;
restart recovery interrupts persisted work, and durable provider resume metadata is not
implemented. Native child execution and resource termination still need the bounded live
acceptance journey; protocol fixtures alone cannot establish those guarantees.

Connection failure latches belong to one transport generation. Shutdown marks the runtime
closing before awaiting an in-flight connector and disposes its result before returning;
readiness cannot reopen a closing host. Execution outcome and native cleanup confirmation
are separate: disconnection can project an interrupted run while unresolved owned work
still blocks restart and makes close fail. Native follow-up turns on an existing child
thread create fresh core execution identities under the current native parent execution;
delayed events from previous turns cannot settle the renewed execution.

If execution cleanup succeeds but pool closure rejects, the host retains that specific
unconfirmed persistence-closure error. The current database adapter marks itself closed
before awaiting its underlying connection shutdown, so a second no-op cannot establish
confirmation. No normal work is readmitted; check local connections before quitting
externally or restarting. This is distinct from retryable execution cleanup failure.
