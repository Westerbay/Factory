import { mkdtemp, writeFile, unlink, rmdir } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { randomUUID } from "node:crypto"
import { docker } from "../environments/docker.mjs"
import { prepareEgress } from "./prepare.mjs"
import { runCargo, rustProcess } from "../rust/run.mjs"

const root = fileURLToPath(new URL("../../", import.meta.url))
const installationId = randomUUID()
const directory = await mkdtemp(join(tmpdir(), "factory-egress-integration-"))
const tags = [
  `factory-egress:${installationId}`,
  `factory-egress-fixture:${installationId}`,
]
const images = new Map()
const failures = []
console.log(`Egress integration fixture: ${installationId} (${directory})`)
function trackedDocker(args, options) {
  const output = docker(args, options)
  if (args[0] === "image" && args[1] === "inspect" && tags.includes(args[2])) {
    const [image] = JSON.parse(output)
    if (!/^sha256:[0-9a-f]{64}$/.test(image.Id))
      throw new Error("Invalid fixture image ID")
    images.set(args[2], image.Id)
  }
  return output
}
try {
  await writeFile(
    join(directory, "installation.json"),
    JSON.stringify({ installationId }),
    { flag: "wx", mode: 0o600 }
  )
  const metadata = await prepareEgress(directory, trackedDocker)
  docker(
    [
      "build",
      "--target",
      "fixture",
      "--tag",
      tags[1],
      resolve(root, "infra/egress"),
    ],
    { timeout: 1_200_000 }
  )
  const [fixture] = JSON.parse(trackedDocker(["image", "inspect", tags[1]]))
  if (!/^sha256:[0-9a-f]{64}$/.test(fixture.Id))
    throw new Error("Invalid fixture image ID")
  const env = {
    FACTORY_EGRESS_TEST_IMAGE_ID: metadata.imageId,
    FACTORY_EGRESS_DNS_IMAGE_ID: fixture.Id,
  }
  const available = rustProcess().env
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
      "egress::integration::",
      "--",
      "--ignored",
      "--test-threads=1",
    ],
    { env }
  )
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error("Egress Docker integration failed")
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
// Individual files in this invocation's mkdtemp directory; never a recursive cleanup.
for (const name of ["egress.json", "installation.json"]) {
  await unlink(join(directory, name)).catch((error) => {
    if (error.code !== "ENOENT") failures.push(error)
  })
}
await rmdir(directory).catch((error) => failures.push(error))
if (failures.length)
  throw new AggregateError(
    failures,
    "Egress integration or fixture cleanup failed"
  )
