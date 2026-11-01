// Adapted from WebApp Template; see docs/REUSE.md and LICENSE.
import { resolve } from "node:path"

export default {
  forbidden: [
    {
      name: "web-browser-only",
      severity: "error",
      from: { path: "^apps/web/src/" },
      to: {
        path: "^(?:apps/(?!web/)|crates/|packages/(?!(?:ui|i18n)/)|@workspace/(?!(?:ui|i18n)(?:/|$)))|^(?:node:|electron$)|(^|/)node_modules/electron/",
      },
    },
    {
      name: "web-no-node-builtins",
      severity: "error",
      from: { path: "^apps/web/src/" },
      to: { dependencyTypes: ["core"] },
    },
    {
      name: "i18n-browser-only",
      severity: "error",
      from: { path: "^packages/i18n/src/(?!generated/server\\.js$)" },
      to: { dependencyTypes: ["core"] },
    },
    { name: "no-cycles", severity: "error", from: {}, to: { circular: true } },
    {
      name: "unresolved-source-import",
      severity: "error",
      from: { path: "^(apps/[^/]+|packages/[^/]+)/src/" },
      to: { couldNotResolve: true },
    },
    {
      name: "packages-no-apps",
      severity: "error",
      from: { path: "^packages/[^/]+/src/" },
      to: { path: "^apps/" },
    },
    {
      name: "core-independent",
      severity: "error",
      from: { path: "^packages/core/src/" },
      to: { pathNot: "^packages/core/src/" },
    },
    {
      name: "application-only-core",
      severity: "error",
      from: { path: "^packages/application/src/" },
      to: { pathNot: "^packages/(?:application|core)/src/" },
    },
    {
      name: "contracts-independent",
      severity: "error",
      from: { path: "^packages/contracts/src/" },
      to: { pathNot: "^packages/contracts/src/|(^|/)node_modules/zod/" },
    },
    {
      name: "reference-renderer-no-system",
      severity: "error",
      from: { path: "^packages/execution-reference/src/renderer/" },
      to: {
        path: "^(?:apps/|crates/|packages/(?!execution-reference/src/renderer/|ui/|contracts/)|@workspace/(?!ui(?:/|$)|contracts(?:/|$)))|^(?:node:|electron$)|(^|/)node_modules/electron/",
      },
    },
    {
      name: "reference-renderer-no-node-builtins",
      severity: "error",
      from: { path: "^packages/execution-reference/src/renderer/" },
      to: { dependencyTypes: ["core"] },
    },
    {
      name: "no-product-execution-reference",
      severity: "error",
      from: { path: "^(?:apps/|packages/(?!execution-reference/))" },
      to: {
        path: "^packages/execution-reference/|^@workspace/execution-reference(?:/|$)|(^|/)node_modules/@workspace/execution-reference/",
      },
    },
    {
      name: "execution-reference-only-legacy",
      severity: "error",
      from: { path: "^packages/execution-reference/src/" },
      to: {
        path: "^(?:apps/|crates/|packages/(?!execution-reference/|core/|application/|contracts/|ui/)|@workspace/(?!execution-reference(?:/|$)|core(?:/|$)|application(?:/|$)|contracts(?:/|$)|ui(?:/|$)))",
      },
    },
    {
      name: "execution-reference-no-electron",
      severity: "error",
      from: { path: "^packages/execution-reference/" },
      to: {
        path: "^electron(?:-vite)?$|(^|/)node_modules/electron(?:-vite)?/",
      },
    },
    {
      name: "ui-browser-only",
      severity: "error",
      from: { path: "^packages/ui/src/" },
      to: { dependencyTypes: ["core"] },
    },
    {
      name: "ui-no-electron",
      severity: "error",
      from: { path: "^packages/ui/src/" },
      to: { path: "(^|/)node_modules/electron/|^electron$" },
    },
    {
      name: "ui-no-business-dependencies",
      severity: "error",
      from: { path: "^packages/ui/src/" },
      to: { path: "^(apps/|packages/(?!ui/)|@workspace/(?!ui(?:/|$)))" },
    },
    {
      name: "adapters-independent",
      severity: "error",
      from: { path: "^packages/adapters/src/" },
      to: {
        path: "^packages/(?!adapters/|core/)|^@workspace/(?!adapters(?:/|$)|core(?:/|$))",
      },
    },
    {
      name: "database-independent",
      severity: "error",
      from: { path: "^packages/database/src/" },
      to: {
        path: "^packages/(?!database/|core/)|^@workspace/(?!database(?:/|$)|core(?:/|$))",
      },
    },
  ],
  options: {
    doNotFollow: { path: "(^|/)(?:node_modules|dist|out|test)(?:/|$)" },
    tsPreCompilationDeps: true,
    tsConfig: { fileName: resolve("scripts/tsconfig.boundaries.json") },
    enhancedResolveOptions: {
      exportsFields: ["exports"],
      conditionNames: ["types", "import", "node", "default"],
    },
  },
}
