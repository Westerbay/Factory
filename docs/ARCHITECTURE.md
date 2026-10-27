# Architecture

## Dependency direction

Factory retains the template's pnpm workspace, Turborepo, strict TypeScript and dependency
checks. Electron owns the host shell; React runs in an isolated renderer.

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
The database adapter implements RunStore; the [Codex adapter](../packages/adapters/src/codex/runtime.ts) implements AgentRuntime. No Electron bridge is
implemented by these schemas.

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
migrations. Host wiring is a later increment. No execution is automatically replayed.

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
operation. `recover()` is called once by the future single-instance host before accepting
commands, after migrations are ready: it atomically appends interruption events and
invalidates outstanding questions/permissions through the core reducer. Opening another
connection does not interrupt live work. `close()` closes the pool; operations after close
fail visibly. Driver details and SQL causes are not exposed as user-visible errors.

[Owned infrastructure helpers](../scripts/database/owned.mjs) verify the Docker project,
checkout directory, ownership token, exact container/database credentials, volume labels
and loopback port before connecting. Compose receives an explicit empty environment file
and a restricted variable set. No caller-supplied URL or real `.env` is consumed.

## Desktop foundation

`apps/desktop/src/main` owns the window. Context isolation and process sandboxing are
enabled; Node integration and webviews are disabled. New windows, navigation, redirects
and permission requests are denied. Preload exposes no bridge in this increment.
The renderer only displays the actual foundation state. A restrictive CSP applies;
loopback WebSocket connections and inline styles support Vite development.

Production preview currently loads the bundled local HTML with `loadFile`. Before
rendering untrusted agent Markdown, introduce a hardened app protocol and safe rendering
as part of the interface/security task. Do not enable remote content or broad privileged APIs.

## Planned runtime

A separate Electron utility process will compose use cases with Codex and database
adapters. PostgreSQL and Drizzle stay in `packages/database`; the renderer never sees
ORM or provider protocol types. Adding a service means implementing a focused adapter;
a new domain capability may also require a new port. No HTTP API is needed in the
current local application. See [development](DEVELOPMENT.md) for the current commands.

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
