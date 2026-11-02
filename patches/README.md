# Dependency security fixes

Reviewed against official releases on 2026-11-02. Prefer a compatible published
release over a local patch; keep the lockfile as the source of installed versions.

- `glob@14.0.3` replaces the glob backports. The official fix for
  [CVE-2026-118773](https://github.com/isaacs/node-glob/security/advisories/GHSA-3g3m-2fjr-6pw4)
  is available from 14.0.2. Older glob consumers outside the affected override range
  retain their existing major and callback API.
- `minimatch@11.0.4` replaces the backports for compatible consumers. The official
  fix for [CVE-2026-118764](https://github.com/isaacs/minimatch/security/advisories/GHSA-g3hv-75wg-m249)
  is available from 11.0.1. Overrides exclude legacy major versions.

## Remaining local patches

- `deepmerge@4.3.1`: local guard rejecting `__proto__` keys on both merge inputs,
  based on [CVE-2026-93753](https://github.com/advisories/GHSA-gcqx-w48m-w2h4).
  The advisory lists no patched version and 4.3.1 remains the latest published release.
  This is a project-maintained mitigation, not an official upstream fix. Remove it
  after a compatible release passes the prototype-pollution and normal-merge tests.

## Validation and removal

Run the root `test:dependency-security` script after a full frozen installation.
Its three tests exercise the patched `deepmerge` resolved through the web build's
dependency chain: `vite-plugin-pwa` → `workbox-build` → `@rollup/plugin-node-resolve`.
They check both merge inputs and normal merging without scanning `node_modules` or
pinning expected version numbers. Remove these tests and their command when the
local patch is replaced by an upstream fix or the dependency is no longer used.

One-off checks of upstream glob/minimatch fixes remain outside the repository.
Vulnerability monitoring checks dependency versions; the existing application tests,
lint, typecheck and builds check compatibility. A passing patch regression alone does
not prove that the application was exposed to the vulnerability.

Keep the remaining patch files in installation and Docker build contexts. Local
patches retain the original package version, so version-based advisories remain
visible; a proposed PR does not establish integration or deployment. Remove obsolete
patch files and manifest entries together with a package-manager-generated lockfile.

Vitest 5.0.5 also includes the 5.0.3 mocker fixes for CVE-2026-112949 and
CVE-2026-112950. These concern dev-server mocker plugins, not production build
output. This does not establish that Factory exposed those plugins.
