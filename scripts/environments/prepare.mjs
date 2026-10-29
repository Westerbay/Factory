import { randomUUID } from "node:crypto"
import { mkdir, readFile, writeFile, rename, unlink } from "node:fs/promises"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { docker } from "./docker.mjs"

const root = fileURLToPath(new URL("../../", import.meta.url))
export const policyVersion = "2"
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
const imageId = /^sha256:[0-9a-f]{64}$/

export async function prepare(
  directory = resolve(root, ".factory/environments"),
  runDocker = docker
) {
  await mkdir(directory, { recursive: true })
  const installationPath = resolve(directory, "installation.json")
  // Exclusive creation prevents simultaneous prepares from assigning different identities.
  try {
    await writeFile(
      installationPath,
      JSON.stringify({ installationId: randomUUID() }),
      { flag: "wx", mode: 0o600 }
    )
  } catch (error) {
    if (error.code !== "EEXIST") throw error
  }
  const installation = JSON.parse(await readFile(installationPath, "utf8"))
  if (
    !uuid.test(installation.installationId) ||
    installation.installationId === "00000000-0000-0000-0000-000000000000"
  ) {
    throw new Error("Invalid local environment installation identity")
  }
  const tag = `factory-environment:${installation.installationId}`
  // Preparation is the only build/pull path, explicitly invoked by the operator.
  runDocker(
    [
      "build",
      "--pull",
      "--force-rm",
      "--tag",
      tag,
      resolve(root, "infra/environments"),
    ],
    { timeout: 600_000 }
  )
  const [image] = JSON.parse(runDocker(["image", "inspect", tag]))
  if (
    !imageId.test(image.Id) ||
    image.Os !== "linux" ||
    image.Config?.User !== "10000:10000" ||
    image.Config?.Labels?.["io.factory.environment.image"] !== policyVersion ||
    image.Config?.Labels?.["io.factory.codex.version"] !== "0.160.0"
  ) {
    throw new Error("Prepared image does not satisfy the environment contract")
  }
  const metadata = {
    schemaVersion: 1,
    installationId: installation.installationId,
    imageId: image.Id,
    policyVersion,
  }
  const temporary = resolve(directory, `prepared-${randomUUID()}.tmp`)
  try {
    await writeFile(temporary, JSON.stringify(metadata, null, 2) + "\n", {
      flag: "wx",
      mode: 0o600,
    })
    await rename(temporary, resolve(directory, "prepared.json"))
  } catch (error) {
    await unlink(temporary).catch((cleanup) => {
      if (cleanup.code !== "ENOENT") throw cleanup
    })
    throw error
  }
  return metadata
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const args = process.argv.slice(2)
  if (
    args.length !== 0 &&
    !(args.length === 2 && args[0] === "--directory" && args[1])
  ) {
    throw new Error(
      "Usage: environments:prepare [--directory <local preparation directory>]"
    )
  }
  await prepare(args[1] ? resolve(args[1]) : undefined)
  console.log(
    "Local environment image prepared. Existing environments retain their recorded image."
  )
}
