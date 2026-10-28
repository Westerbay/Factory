import { mkdtemp, unlink, rmdir } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { prepare } from "./prepare.mjs"
import { runCargo, rustProcess } from "../rust/run.mjs"

// Only this explicit command builds an image and enables owned ephemeral Docker tests.
const directory = await mkdtemp(
  join(tmpdir(), "factory-environment-integration-")
)
try {
  await prepare(directory)
  const available = rustProcess().env
  const env = { FACTORY_ENV_TEST_DIRECTORY: directory }
  for (const key of [
    "PATH",
    "SystemRoot",
    "ProgramFiles",
    "USERPROFILE",
    "HOME",
    "TEMP",
    "TMP",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "RUSTUP_HOME",
    "LIB",
    "INCLUDE",
    "LIBPATH",
  ]) {
    if (available[key] !== undefined) env[key] = available[key]
  }
  const result = runCargo(
    [
      "test",
      "--locked",
      "-p",
      "factory-adapters",
      "real_docker_persistence_isolation_and_limits",
      "--",
      "--ignored",
    ],
    { env }
  )
  if (result.error) throw result.error
  if (result.status !== 0)
    throw new Error("Environment Docker integration failed")
} finally {
  for (const name of ["prepared.json", "installation.json"]) {
    await unlink(join(directory, name)).catch((error) => {
      if (error.code !== "ENOENT") throw error
    })
  }
  await rmdir(directory)
}
