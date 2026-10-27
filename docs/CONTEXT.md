# Context

Factory is a local Electron application for building and observing agent workflows.
The user wants modular hexagonal boundaries, local execution, existing skills as the
workflow source of truth, and frequent review after each code task.

The workspace adapts the English WebApp Template foundation for desktop. Provider-neutral
execution rules and validated desktop contracts are implemented and exercised with
test-only deterministic adapters. PostgreSQL persistence, guarded local setup, explicit
restart recovery and isolated database integration checks are now implemented. The Electron window remains the foundation screen;
no execution service, real provider or persistence adapter is connected to it yet. The source template stays unchanged. Development is Linux-first; distributing
installers and running a VPS service are outside this increment.

Local execution describes the agent tools and host. A provider may still call a remote
model; offline inference is not promised. No provider or account is accessed by this scaffold.

See [architecture](ARCHITECTURE.md), [product](PRODUCT.md) and [development](DEVELOPMENT.md).
