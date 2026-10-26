// Regression harness adapted from WebApp Template.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"
import config from "../../packages/config/dependency-cruiser.mjs"
import tsConfig from "../tsconfig.boundaries.json" with { type: "json" }

const cli = fileURLToPath(
  new URL(
    "../../node_modules/dependency-cruiser/bin/dependency-cruiser.mjs",
    import.meta.url
  )
)

async function violations(t, files) {
  const directory = await mkdtemp(join(tmpdir(), "factory-boundaries-"))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const sources = {
    "package.json": JSON.stringify({ type: "module" }),
    "scripts/tsconfig.boundaries.json": JSON.stringify(tsConfig),
    "boundaries.json": JSON.stringify({
      ...config,
      options: {
        ...config.options,
        tsConfig: {
          fileName: join(directory, "scripts/tsconfig.boundaries.json"),
        },
      },
    }),
    ...Object.fromEntries(
      ["core", "application", "database", "adapters", "contracts", "ui"].map(
        (name) => [
          `packages/${name}/src/index.ts`,
          "export interface Entry { id: string }\n",
        ]
      )
    ),
    "node_modules/electron/package.json": JSON.stringify({
      name: "electron",
      main: "index.js",
      types: "index.d.ts",
    }),
    "node_modules/electron/index.js": "exports.app = {}",
    "node_modules/electron/index.d.ts": "export interface App { quit(): void }",
    "apps/desktop/src/main/system.ts":
      "export interface System { id: string }\n",
    ...files,
  }
  for (const [path, content] of Object.entries(sources)) {
    const target = join(directory, path)
    await mkdir(dirname(target), { recursive: true })
    await writeFile(target, content)
  }
  const run = (output) =>
    spawnSync(
      process.execPath,
      [
        cli,
        "--config",
        "boundaries.json",
        "--output-type",
        output,
        "apps",
        "packages",
      ],
      { cwd: directory, encoding: "utf8" }
    )
  const report = run("json")
  if (report.error) throw report.error
  assert.equal(report.status, 0, report.stderr)
  const summary = JSON.parse(report.stdout).summary
  const validation = run("err")
  if (validation.error) throw validation.error
  assert.equal(validation.signal, null, validation.stderr)
  assert.equal(validation.status === 0, summary.error === 0, validation.stderr)
  return summary.violations
}

function hasRule(found, name, from) {
  assert.ok(
    found.some((item) => item.rule.name === name && item.from === from),
    `${name}: ${from}`
  )
}

test("core and application reject infrastructure, framework and type-only alias dependencies", async (t) => {
  const found = await violations(t, {
    "packages/core/src/database.ts":
      'export type { Entry } from "@workspace/database"',
    "packages/core/src/electron.ts": 'export type { App } from "electron"',
    "packages/application/src/electron.ts":
      'export type { App } from "electron"',
    "packages/core/src/files.ts": 'export { readFile } from "node:fs/promises"',
    "packages/application/src/provider.ts":
      'void import("@workspace/adapters")',
    "packages/application/src/database.ts":
      'export type { Entry } from "../../database/src/index.js"',
    "packages/application/src/framework.ts":
      'export type { System } from "@system/system"',
  })
  hasRule(found, "core-independent", "packages/core/src/database.ts")
  hasRule(found, "core-independent", "packages/core/src/files.ts")
  hasRule(found, "core-independent", "packages/core/src/electron.ts")
  hasRule(
    found,
    "application-only-core",
    "packages/application/src/electron.ts"
  )
  for (const file of ["provider", "database", "framework"])
    hasRule(
      found,
      "application-only-core",
      `packages/application/src/${file}.ts`
    )
})

test("renderer rejects type-only, dynamic, relative, aliased and builtin system imports", async (t) => {
  const found = await violations(t, {
    "apps/desktop/src/renderer/type.ts":
      'export type { Entry } from "@workspace/database"',
    "apps/desktop/src/renderer/dynamic.ts":
      'void import("@workspace/adapters")',
    "apps/desktop/src/renderer/relative.ts":
      'export type { System } from "../main/system.js"',
    "apps/desktop/src/renderer/alias.ts":
      'export type { System } from "@system/system"',
    "apps/desktop/src/renderer/electron.ts":
      'export type { App } from "electron"',
    "apps/desktop/src/renderer/node.ts": 'export { readFile } from "fs"',
    "apps/desktop/src/renderer/node-prefix.ts":
      'export { readFile } from "node:fs"',
    "apps/desktop/src/renderer/core.ts":
      'export type { Entry } from "@workspace/core"',
  })
  for (const file of [
    "type",
    "dynamic",
    "relative",
    "alias",
    "core",
    "electron",
  ])
    hasRule(found, "renderer-no-system", `apps/desktop/src/renderer/${file}.ts`)
  for (const file of ["node", "node-prefix"])
    hasRule(
      found,
      "renderer-no-node-builtins",
      `apps/desktop/src/renderer/${file}.ts`
    )
})

test("unresolved aliases fail closed", async (t) => {
  const found = await violations(t, {
    "apps/desktop/src/renderer/missing.ts": 'void import("@unknown/database")',
  })
  hasRule(
    found,
    "unresolved-source-import",
    "apps/desktop/src/renderer/missing.ts"
  )
})

test("contracts, adapters, database, UI and preload preserve their boundaries", async (t) => {
  const found = await violations(t, {
    "packages/contracts/src/bad.ts":
      'export type { Entry } from "@workspace/database"',
    "packages/ui/src/electron.ts": 'export type { App } from "electron"',
    "packages/ui/src/node.ts": 'export { readFile } from "node:fs"',
    "packages/ui/src/bad.ts": 'export type { Entry } from "@workspace/core"',
    "packages/adapters/src/bad.ts":
      'export type { Entry } from "@workspace/database"',
    "packages/database/src/bad.ts":
      'export type { Entry } from "@workspace/application"',
    "apps/desktop/src/preload/bad.ts":
      'export type { Entry } from "@workspace/adapters"',
  })
  hasRule(found, "ui-browser-only", "packages/ui/src/node.ts")
  hasRule(found, "ui-no-electron", "packages/ui/src/electron.ts")
  for (const [rule, path] of [
    ["contracts-independent", "packages/contracts"],
    ["ui-no-business-dependencies", "packages/ui"],
    ["adapters-independent", "packages/adapters"],
    ["database-independent", "packages/database"],
    ["preload-no-implementations", "apps/desktop/src/preload"],
  ]) {
    const file = path.startsWith("packages")
      ? `${path}/src/bad.ts`
      : `${path}/bad.ts`
    hasRule(found, rule, file)
  }
})

test("allows core use, adapter ports, presentation contracts and runtime composition", async (t) => {
  assert.deepEqual(
    await violations(t, {
      "packages/application/src/good.ts":
        'export type { Entry } from "@workspace/core"',
      "packages/database/src/good.ts":
        'export type { Entry } from "@workspace/core"',
      "packages/adapters/src/good.ts":
        'export type { Entry } from "@workspace/core"',
      "apps/desktop/src/renderer/good.ts":
        'export type { Entry } from "@workspace/contracts"; export type { Entry as Widget } from "@workspace/ui"',
      "apps/desktop/src/preload/good.ts":
        'export type { Entry } from "@workspace/contracts"',
      "apps/desktop/src/runtime/good.ts":
        'export type { Entry } from "@workspace/database"; export type { Entry as Provider } from "@workspace/adapters"; export type { Entry as App } from "@workspace/application"',
    }),
    []
  )
})
