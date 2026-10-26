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
Database and provider adapters still have empty entrypoints. No Electron bridge is
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
runs as interrupted before allowing execution; durable recovery is part of the upcoming
database/runtime integration. No execution is automatically replayed.

The dependency-cruiser configuration resolves TypeScript aliases and includes type-only
imports. Its regression harness checks both violations and permitted composition.

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
