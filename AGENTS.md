# Factory agent instructions

## Working agreement

- Inspect the approved task and current diff before changing code.
- Keep instructions, source, documentation and commit messages in English.
- Preserve unrelated work, existing skills and credentials. Never read or change real
  `.env` files, provider credentials or global agent configuration for routine checks.
- Planning documents under `docs/plans/` remain local and ignored unless publication
  is explicitly authorized. Never infer approval from files or elapsed time.
- Do not commit, push, deploy, migrate data or publish outside the user's exact authority.
- Use Conventional Commits, for example `feat(desktop): add local foundation`.

## Architecture

- Keep `packages/core` pure and provider-neutral. Application use cases depend only on core.
- Concrete provider and persistence implementations belong in adapters and database;
  their composition belongs in the desktop runtime host, not in the renderer.
- Renderer code can consume contracts and UI primitives. Never import Electron, Node,
  filesystem, provider, application or database implementations into it.
- Check type-only and aliased imports with `pnpm boundaries:check` and regression tests.
- Introduce real ports and types when their behavior is implemented, not speculative APIs.
- Reuse shadcn primitives only when needed. The template's branding and styling are not
  Factory requirements. Preserve license notices for reused code.

## Documentation

Update the affected documentation when behavior, contracts, shared code, architecture
or developer procedures change. Remove obsolete information instead of appending a
change journal; do not rewrite documents whose facts remain accurate.

- `docs/CONTEXT.md`: project overview, current capabilities, limitations and open decisions.
- `docs/PRODUCT.md`: user capabilities and durable product rules.
- `docs/ARCHITECTURE.md`: module ownership, dependency boundaries and consequential flows.
- `docs/DESIGN.md`: reusable interface and accessibility conventions; styling stays replaceable.
- `docs/REUSE.md`: stable reusable contracts and mechanisms, their owners and when to use them.
  Exclude test inventories, one-off internals and implementation logs.
- `docs/DEVELOPMENT.md` and `README.md`: commands, prerequisites and onboarding procedures.

Prefer links to the owning source over duplicated details. Update `REUSE.md` when adding,
changing or removing a shared contract; introduce an entry only when it has a plausible
consumer beyond its initial feature.

## Validation

Use pnpm and the pinned Node major. Run `pnpm check` and inspect the final diff.
Boundary checks have meaningful positive/negative fixtures in `scripts/test/`.
Keep routine checks independent of providers, secrets and external services unless
the task explicitly requires a guarded integration check.
Keep the [documented validation commands](docs/DEVELOPMENT.md) accurate as the project evolves.
Report actual checks and blockers; do not present planned runtime behavior as verified.
Human QA and acceptance journeys are not versioned executable test suites. Keep any
agent-authored acceptance/browser check scripts and their artifacts in ignored local
scratch. Retain meaningful unit and integration regression tests in the repository.
