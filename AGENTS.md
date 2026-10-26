# Factory agent instructions

## Working agreement

- Inspect the approved task and current diff before changing code. Deliver one code
  task at a time and stop for user feedback after its draft PR.
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

## Validation

Use pnpm and the pinned Node major. Run `pnpm check` and inspect the final diff.
Boundary checks have meaningful positive/negative fixtures in `scripts/test/`.
The current task needs no provider, secrets, database, seed or external service.
Keep [development documentation](docs/DEVELOPMENT.md) accurate as later tasks add them.
Report actual checks and blockers; do not present planned runtime behavior as verified.
