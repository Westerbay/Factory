import { readFileSync, writeFileSync } from "node:fs"
import { resolve } from "node:path"
import openapiTS, { astToString } from "openapi-typescript"
import { format, resolveConfig } from "prettier"
import { root, runCargo } from "./rust/run.mjs"

const check = process.argv.includes("--check")
const generated = runCargo(
  [
    "run",
    "--quiet",
    "--locked",
    "--package",
    "factory-api",
    "--bin",
    "export-openapi",
  ],
  {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }
)
if (generated.error || generated.status !== 0) {
  throw new Error("Inert OpenAPI export failed.")
}
const schema = JSON.parse(generated.stdout)
const schemaText = `${JSON.stringify(schema, null, 2)}\n`
const typesText = astToString(await openapiTS(schema))
for (const [path, content] of [
  ["apps/api/openapi.json", schemaText],
  ["apps/web/src/lib/api/schema.d.ts", typesText],
]) {
  const target = resolve(root, path)
  const formatted = await format(content, {
    ...(await resolveConfig(target)),
    filepath: target,
  })
  if (check) {
    if (readFileSync(target, "utf8") !== formatted) {
      throw new Error(
        `Generated API contract differs: ${path}. Run pnpm api:generate.`
      )
    }
  } else writeFileSync(target, formatted)
}
console.log(check ? "API contract is current." : "API contract generated.")
