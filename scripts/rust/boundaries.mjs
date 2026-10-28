import { resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { runCargo } from "./run.mjs"

const workspaceEdges = {
  "factory-core": [],
  "factory-application": ["factory-core"],
  "factory-adapters": ["factory-core"],
  "factory-database": ["factory-core"],
  "factory-api": [
    "factory-core",
    "factory-application",
    "factory-adapters",
    "factory-database",
  ],
}
const pureDependencies = new Set([
  "serde",
  "uuid",
  "chrono",
  "async-trait",
  "thiserror",
])

/** Cargo resolves inherited, renamed and platform-specific manifest dependencies. */
export function boundaryViolations(metadata) {
  const members = new Set(metadata.workspace_members)
  const packages = metadata.packages.filter((pkg) => members.has(pkg.id))
  const violations = []
  for (const pkg of packages) {
    const allowed = workspaceEdges[pkg.name]
    if (!allowed) {
      violations.push(`Unclassified workspace crate: ${pkg.name}`)
      continue
    }
    for (const dep of pkg.dependencies) {
      const local = dep.path !== undefined && dep.path !== null
      if (local && !allowed.includes(dep.name)) {
        violations.push(`${pkg.name} cannot depend on local crate ${dep.name}`)
      }
      if (
        !local &&
        ["factory-core", "factory-application"].includes(pkg.name) &&
        !(
          pkg.name === "factory-application" &&
          dep.name === "tokio" &&
          dep.kind === "dev"
        ) &&
        !pureDependencies.has(dep.name)
      ) {
        violations.push(
          `${pkg.name} cannot import infrastructure dependency ${dep.name}`
        )
      }
    }
  }
  return violations
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const result = runCargo(
    ["metadata", "--format-version", "1", "--no-deps", "--locked", "--offline"],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
    }
  )
  if (result.error || result.status !== 0) {
    console.error("Cargo metadata failed; Rust boundaries were not verified.")
    process.exitCode = 1
  } else {
    const violations = boundaryViolations(JSON.parse(result.stdout))
    for (const violation of violations) console.error(violation)
    if (violations.length) process.exitCode = 1
    else console.log("Rust workspace boundaries passed.")
  }
}
