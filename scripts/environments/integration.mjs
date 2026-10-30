import { mkdtemp, unlink, rmdir } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { prepare } from "./prepare.mjs"
import { runCargo, rustProcess } from "../rust/run.mjs"
import { docker } from "./docker.mjs"
import { readFileSync } from "node:fs"

// Only this explicit command builds an image and enables owned ephemeral Docker tests.
const directory = await mkdtemp(
  join(tmpdir(), "factory-environment-integration-")
)
const images = new Map()
const failures = []
let installationId
function trackedDocker(args, options) {
  if (!installationId) {
    installationId = JSON.parse(
      readFileSync(join(directory, "installation.json"), "utf8")
    ).installationId
    console.log(
      `Environment integration fixture: ${installationId} (${directory})`
    )
  }
  const output = docker(args, options)
  if (
    args[0] === "image" &&
    args[1] === "inspect" &&
    [
      `factory-environment:${installationId}`,
      `factory-egress:${installationId}`,
    ].includes(args[2])
  ) {
    const [image] = JSON.parse(output)
    if (!/^sha256:[0-9a-f]{64}$/.test(image.Id))
      throw new Error("Invalid fixture image ID")
    images.set(args[2], image.Id)
  }
  return output
}
try {
  const prepared = await prepare(directory, trackedDocker)
  const available = rustProcess().env
  const env = {
    FACTORY_ENV_TEST_DIRECTORY: directory,
    FACTORY_PROCESS_TEST_IMAGE: prepared.imageId,
  }
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
      "--lib",
      "--",
      "--ignored",
      "--skip",
      "egress::integration::",
      "--test-threads=1",
    ],
    { env, timeout: 600_000 }
  )
  if (result.error) throw result.error
  if (result.status !== 0)
    throw new Error("Environment Docker integration failed")
} catch (error) {
  failures.push(error)
}
for (const [tag, id] of images) {
  try {
    const [image] = JSON.parse(docker(["image", "inspect", tag]))
    if (image.Id !== id)
      throw new Error("Refusing to remove a changed fixture image tag")
    docker(["image", "rm", tag])
  } catch (error) {
    failures.push(error)
  }
}
for (const name of ["prepared.json", "egress.json", "installation.json"]) {
  await unlink(join(directory, name)).catch((error) => {
    if (error.code !== "ENOENT") failures.push(error)
  })
}
await rmdir(directory).catch((error) => failures.push(error))
if (failures.length)
  throw new AggregateError(
    failures,
    "Environment integration or fixture cleanup failed"
  )
