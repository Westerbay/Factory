// Adapted from WebApp Template; see docs/REUSE.md and LICENSE.
import { resolve } from "node:path"

export default {
  forbidden: [
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
      name: "renderer-no-system",
      severity: "error",
      from: { path: "^apps/desktop/src/renderer/" },
      to: {
        path: "^(?:apps/desktop/src/(?!renderer/)|packages/(?!ui/|contracts/)|@workspace/(?!ui(?:/|$)|contracts(?:/|$)))|^(?:node:|electron$)|(^|/)node_modules/electron/",
      },
    },
    {
      name: "renderer-no-node-builtins",
      severity: "error",
      from: { path: "^apps/desktop/src/renderer/" },
      to: { dependencyTypes: ["core"] },
    },
    {
      name: "preload-no-implementations",
      severity: "error",
      from: { path: "^apps/desktop/src/preload/" },
      to: {
        path: "^(?:packages/(?!contracts/)|apps/desktop/src/(?!preload/)|@workspace/(?!contracts(?:/|$)))",
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
