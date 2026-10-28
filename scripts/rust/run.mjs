import { existsSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..")

/** Use an optional repository-local toolchain without altering global settings. */
export function rustProcess() {
  const toolchains = resolve(root, ".factory/toolchains")
  const cargoHome = resolve(toolchains, "cargo")
  const cargo = resolve(
    cargoHome,
    "bin",
    process.platform === "win32" ? "cargo.exe" : "cargo"
  )
  if (!existsSync(cargo)) return { command: "cargo", env: { ...process.env } }
  return {
    command: cargo,
    env: {
      ...process.env,
      CARGO_HOME: cargoHome,
      RUSTUP_HOME: resolve(toolchains, "rustup"),
    },
  }
}

export function runCargo(args, options = {}) {
  const { command, env } = rustProcess()
  return spawnSync(command, args, {
    cwd: root,
    env,
    stdio: "inherit",
    ...options,
  })
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const result = runCargo(process.argv.slice(2))
  if (result.error) {
    console.error(
      "Unable to start Cargo. Install the pinned Rust toolchain and native build tools."
    )
  }
  process.exitCode = result.status ?? 1
}
