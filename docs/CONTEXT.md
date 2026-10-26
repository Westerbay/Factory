# Context

Factory is a local Electron application for building and observing agent workflows.
The user wants modular hexagonal boundaries, local execution, existing skills as the
workflow source of truth, and frequent review after each code task.

The first increment adapts the English WebApp Template foundation into a desktop
workspace. The source template stays unchanged. Development is Linux-first; distributing
installers and running a VPS service are outside this increment.

Local execution describes the agent tools and host. A provider may still call a remote
model; offline inference is not promised. No provider or account is accessed by this scaffold.

See [architecture](ARCHITECTURE.md), [product](PRODUCT.md) and [development](DEVELOPMENT.md).
