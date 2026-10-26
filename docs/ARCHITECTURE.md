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

The shared packages currently contain empty module entrypoints to reserve enforceable
boundaries. They expose no invented business behavior. Runtime composition, IPC ports,
provider contracts and database schemas will arrive with their owning code tasks.

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
