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
      [
        "core",
        "application",
        "database",
        "adapters",
        "contracts",
        "ui",
        "i18n",
      ].map((name) => [
        `packages/${name}/src/index.ts`,
        "export interface Entry { id: string }\n",
      ])
    ),
    "node_modules/electron/package.json": JSON.stringify({
      name: "electron",
      main: "index.js",
      types: "index.d.ts",
    }),
    "node_modules/electron/index.js": "exports.app = {}",
    "node_modules/electron/index.d.ts": "export interface App { quit(): void }",
    "packages/execution-reference/src/runtime/system.ts":
      "export interface System { id: string }\n",
    "packages/execution-reference/out/runtime/system.js":
      "export const system = { id: 'reference-only' }\n",
    "apps/web/src/shell.ts": "export {}\n",
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
      'export type { System } from "@workspace/execution-reference/runtime/system"',
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

test("reference renderer rejects type-only, dynamic, relative, aliased and builtin system imports", async (t) => {
  const found = await violations(t, {
    "packages/execution-reference/src/renderer/type.ts":
      'export type { Entry } from "@workspace/database"',
    "packages/execution-reference/src/renderer/dynamic.ts":
      'void import("@workspace/adapters")',
    "packages/execution-reference/src/renderer/relative.ts":
      'export type { System } from "../runtime/system.js"',
    "packages/execution-reference/src/renderer/alias.ts":
      'export type { System } from "@workspace/execution-reference/runtime/system"',
    "packages/execution-reference/src/renderer/electron.ts":
      'export type { App } from "electron"',
    "packages/execution-reference/src/renderer/node.ts":
      'export { readFile } from "fs"',
    "packages/execution-reference/src/renderer/node-prefix.ts":
      'export { readFile } from "node:fs"',
    "packages/execution-reference/src/renderer/core.ts":
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
    hasRule(
      found,
      "reference-renderer-no-system",
      `packages/execution-reference/src/renderer/${file}.ts`
    )
  for (const file of ["node", "node-prefix"])
    hasRule(
      found,
      "reference-renderer-no-node-builtins",
      `packages/execution-reference/src/renderer/${file}.ts`
    )
})

test("unresolved aliases fail closed", async (t) => {
  const found = await violations(t, {
    "apps/web/src/missing.ts": 'void import("@unknown/database")',
  })
  hasRule(found, "unresolved-source-import", "apps/web/src/missing.ts")
})

test("web rejects backend, execution-reference and Node imports including type-only aliases", async (t) => {
  const found = await violations(t, {
    "apps/web/src/provider.ts":
      'export type { Entry } from "@workspace/adapters"',
    "apps/web/src/domain.ts": 'export type { Entry } from "@workspace/core"',
    "apps/web/src/database.ts":
      'void import("../../../packages/database/src/index.js")',
    "apps/web/src/reference.ts":
      'export type { System } from "@workspace/execution-reference/runtime/system"',
    "apps/web/src/node.ts": 'export { readFile } from "fs"',
    "apps/web/src/electron.ts": 'export type { App } from "electron"',
    "apps/web/src/i18n.ts":
      'export type { Entry } from "../../../packages/i18n/src/index.js"',
    "packages/i18n/src/bad.ts": 'export { readFile } from "node:fs"',
    "apps/web/src/ui.ts": 'export type { Entry } from "@workspace/ui"',
  })
  for (const file of [
    "provider",
    "domain",
    "database",
    "reference",
    "electron",
  ])
    hasRule(found, "web-browser-only", `apps/web/src/${file}.ts`)
  hasRule(found, "web-no-node-builtins", "apps/web/src/node.ts")
  hasRule(found, "i18n-browser-only", "packages/i18n/src/bad.ts")
  assert.ok(!found.some((item) => item.from === "apps/web/src/i18n.ts"))
  assert.ok(!found.some((item) => item.from === "apps/web/src/ui.ts"))
})

test("contracts, adapters, database and UI preserve their boundaries", async (t) => {
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
  })
  hasRule(found, "ui-browser-only", "packages/ui/src/node.ts")
  hasRule(found, "ui-no-electron", "packages/ui/src/electron.ts")
  for (const [rule, path] of [
    ["contracts-independent", "packages/contracts"],
    ["ui-no-business-dependencies", "packages/ui"],
    ["adapters-independent", "packages/adapters"],
    ["database-independent", "packages/database"],
  ]) {
    hasRule(found, rule, `${path}/src/bad.ts`)
  }
})

test("allows core use, adapter ports and the test-only execution reference closure", async (t) => {
  assert.deepEqual(
    await violations(t, {
      "packages/application/src/good.ts":
        'export type { Entry } from "@workspace/core"',
      "packages/database/src/good.ts":
        'export type { Entry } from "@workspace/core"',
      "packages/adapters/src/good.ts":
        'export type { Entry } from "@workspace/core"',
      "packages/execution-reference/src/renderer/good.ts":
        'export type { Entry } from "@workspace/contracts"; export type { Entry as Widget } from "@workspace/ui"',
      "packages/execution-reference/src/runtime/good.ts":
        'export type { Entry } from "@workspace/core"; export type { Entry as Contract } from "@workspace/contracts"; export type { Entry as App } from "@workspace/application"',
    }),
    []
  )
})

test("products cannot import reference source or compiled output by alias, type or relative path", async (t) => {
  const found = await violations(t, {
    "apps/service/src/type.ts":
      'export type { System } from "@workspace/execution-reference/runtime/system"',
    "apps/service/src/dynamic.ts":
      'void import("@workspace/execution-reference/runtime/system")',
    "apps/service/src/relative.ts":
      'export type { System } from "../../../packages/execution-reference/src/runtime/system.js"',
    "apps/service/src/compiled.ts":
      'export { system } from "../../../packages/execution-reference/out/runtime/system.js"',
    "packages/adapters/src/reference.ts":
      'export type { System } from "../../execution-reference/src/runtime/system.js"',
  })
  for (const file of ["type", "dynamic", "relative", "compiled"]) {
    hasRule(
      found,
      "no-product-execution-reference",
      `apps/service/src/${file}.ts`
    )
  }
  hasRule(
    found,
    "no-product-execution-reference",
    "packages/adapters/src/reference.ts"
  )
})

test("execution reference cannot gain live providers, persistence or Electron dependencies", async (t) => {
  const found = await violations(t, {
    "packages/execution-reference/src/runtime/provider.ts":
      'export type { Entry } from "@workspace/adapters"',
    "packages/execution-reference/src/runtime/database.ts":
      'export type { Entry } from "@workspace/database"',
    "packages/execution-reference/src/runtime/electron.ts":
      'export type { App } from "electron"',
  })
  for (const file of ["provider", "database"]) {
    hasRule(
      found,
      "execution-reference-only-legacy",
      `packages/execution-reference/src/runtime/${file}.ts`
    )
  }
  hasRule(
    found,
    "execution-reference-no-electron",
    "packages/execution-reference/src/runtime/electron.ts"
  )
})
