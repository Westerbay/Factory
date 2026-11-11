import { randomUUID } from "node:crypto"
import { mkdir, readFile, writeFile, rename, unlink } from "node:fs/promises"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { docker } from "./docker.mjs"
import { prepareEgress } from "../egress/prepare.mjs"

const root = fileURLToPath(new URL("../../", import.meta.url))
export const policyVersion = "5"
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
const imageId = /^sha256:[0-9a-f]{64}$/

export async function prepare(
  directory = resolve(root, ".factory/environments"),
  runDocker = docker,
  images = {},
  { checkOnly = false, stageDirectory } = {}
) {
  const installationPath = resolve(directory, "installation.json")
  if ((checkOnly || stageDirectory) && (!images.workspace || !images.egress)) {
    throw new Error("Deployment checks require both release images")
  }
  if (!checkOnly && !stageDirectory) {
    await mkdir(directory, { recursive: true })
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
  }
  const installation = await readFile(installationPath, "utf8").then(
    JSON.parse,
    (error) => {
      if (!checkOnly || error.code !== "ENOENT") throw error
    }
  )
  if (
    installation !== undefined &&
    (!uuid.test(installation?.installationId) ||
      installation.installationId === "00000000-0000-0000-0000-000000000000")
  ) {
    throw new Error("Invalid local environment installation identity")
  }
  if (checkOnly && installation) {
    const states = runDocker([
      "container",
      "ls",
      "--all",
      "--filter",
      `label=io.factory.installation=${installation.installationId}`,
      "--filter",
      "label=io.factory.environment",
      "--format",
      "{{.State}}",
    ])
      .split(/\r?\n/)
      .filter(Boolean)
    if (states.some((state) => state !== "exited" && state !== "dead")) {
      throw new Error(
        "Factory workspaces are active. Finish agent work and stop the workspaces in Factory before updating."
      )
    }
  }
  const tag =
    images.workspace ?? `factory-environment:${installation.installationId}`
  if (!images.workspace) {
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
  }
  const [image] = JSON.parse(runDocker(["image", "inspect", tag]))
  if (
    !imageId.test(image.Id) ||
    image.Os !== "linux" ||
    image.Config?.User !== "10000:10000" ||
    image.Config?.Labels?.["io.factory.environment.image"] !== policyVersion ||
    image.Config?.Labels?.["io.factory.codex.version"] !== "0.160.0" ||
    image.Config?.Labels?.["io.factory.claude.version"] !== "2.1.284"
  ) {
    throw new Error("Prepared image does not satisfy the environment contract")
  }
  const metadata = {
    schemaVersion: 1,
    installationId: installation?.installationId,
    imageId: image.Id,
    policyVersion,
  }
  if (checkOnly) {
    const previous = await readFile(
      resolve(directory, "prepared.json"),
      "utf8"
    ).then(JSON.parse, (error) => {
      if (error.code !== "ENOENT") throw error
    })
    // Existing workspace records are pinned to the prepared image and policy.
    if (
      previous !== undefined &&
      (previous?.schemaVersion !== metadata.schemaVersion ||
        previous.installationId !== metadata.installationId ||
        previous.imageId !== metadata.imageId ||
        previous.policyVersion !== metadata.policyVersion)
    ) {
      throw new Error(
        "The release changes the prepared workspace image or policy. Existing workspaces require a compatible release or a workspace migration before updating."
      )
    }
  }
  // Never publish the user manifest before proxy image and engine validation succeeds.
  await prepareEgress(directory, runDocker, images.egress, {
    checkOnly,
    stageDirectory,
  })
  if (checkOnly) return
  const output = stageDirectory ?? directory
  const temporary = resolve(output, `prepared-${randomUUID()}.tmp`)
  try {
    await writeFile(temporary, JSON.stringify(metadata, null, 2) + "\n", {
      flag: "wx",
      mode: 0o600,
    })
    await rename(temporary, resolve(output, "prepared.json"))
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
  const checkOnly = args.at(-1) === "--check"
  if (checkOnly) args.pop()
  const stageDirectory = args.at(-2) === "--stage" ? args.pop() : undefined
  if (stageDirectory) args.pop()
  const [directoryFlag, directory, imagesFlag, workspace, egress] = args
  const digestReference = /^ghcr\.io\/[a-z0-9_./-]+@sha256:[0-9a-f]{64}$/
  const validDirectory = directoryFlag === "--directory" && directory
  const validImages =
    imagesFlag === "--images" &&
    digestReference.test(workspace ?? "") &&
    digestReference.test(egress ?? "")
  if (
    (checkOnly && stageDirectory) ||
    !(
      (args.length === 5 && validDirectory && validImages) ||
      (!checkOnly &&
        !stageDirectory &&
        (args.length === 0 || (args.length === 2 && validDirectory)))
    )
  ) {
    throw new Error(
      "Usage: environments:prepare [--directory <state directory> [--images <workspace digest ref> <egress digest ref> [--check | --stage <directory>]]]"
    )
  }
  await prepare(
    directory ? resolve(directory) : undefined,
    docker,
    { workspace, egress },
    { checkOnly, stageDirectory }
  )
  if (!checkOnly && !stageDirectory) {
    console.log(
      "Environment images prepared. Existing environments retain their recorded image."
    )
  }
}
