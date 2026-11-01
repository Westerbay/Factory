# Dependency security backports

The lockfile applies these patches during installation. Keep this directory in any
build context that installs the workspace. Package versions remain unchanged, so
version-based scanners may continue reporting the advisories.

- `minimatch@10.2.6`: bound `**/..` preprocessing and skip quadratic deduplication
  above 1,000 patterns, following upstream commit `77a0f5c4b80bdc641c3c930a1d9edf5ad2db57f8`
  ([CVE-2026-118764](https://github.com/isaacs/minimatch/security/advisories/GHSA-g3hv-75wg-m249)).
- `glob@11.1.0`: stop filesystem traversal after cancellation, following upstream
  commit `4682e8c9babee1b385202ede36389634e12076d7`
  ([CVE-2026-118773](https://github.com/isaacs/node-glob/security/advisories/GHSA-3g3m-2fjr-6pw4)).
- `deepmerge@4.3.1`: reject `__proto__` keys on both source and target objects
  ([CVE-2026-93753](https://github.com/advisories/GHSA-gcqx-w48m-w2h4)).
  This local backport covers the published CommonJS and UMD distributions.

`pnpm test:dependency-security` exercises the installed patched modules with
synthetic inputs, including normal behavior and abort during directory traversal.
The command is included in `pnpm test`, so CI checks patch activation. Remove a
patch only after adopting a compatible upstream fix and passing these regressions.

Vitest is updated to 5.0.5, including the 5.0.3 mocker fixes for CVE-2026-112949 and
CVE-2026-112950. These notices concern dev-server mocker plugins, not production
build output; this change does not establish that Factory exposed those plugins.
