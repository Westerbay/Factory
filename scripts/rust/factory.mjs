import { runCargo } from "./run.mjs"

const result = runCargo([
  "run",
  "--locked",
  "--package",
  "factory-api",
  "--bin",
  "factory",
  "--",
  ...process.argv.slice(2),
])
if (result.error) console.error("Unable to start the Factory CLI.")
process.exitCode = result.status ?? 1
