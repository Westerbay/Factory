import { networkInterfaces } from "node:os"
import { isIPv4 } from "node:net"
import { readFile, writeFile, rename, unlink } from "node:fs/promises"
import { randomUUID } from "node:crypto"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { docker } from "../environments/docker.mjs"

const root = fileURLToPath(new URL("../../", import.meta.url))
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

// Production Docker bridges use RFC 1918 ranges already rejected by Squid and nftables.
function privateAddress(address) {
  const [first, second] = address.split(".").map(Number)
  return (
    first === 10 ||
    (first === 172 && second >= 16 && second <= 31) ||
    (first === 192 && second === 168)
  )
}

export function hostAddresses(
  interfaces = networkInterfaces(),
  omitPrivate = false
) {
  const addresses = new Set(["127.0.0.1"])
  for (const entries of Object.values(interfaces)) {
    for (const entry of entries ?? []) {
      if (
        entry.family === "IPv4" &&
        isIPv4(entry.address) &&
        !(omitPrivate && privateAddress(entry.address))
      )
        addresses.add(entry.address)
    }
  }
  return [...addresses].sort()
}

async function requireCompatibleProxy(directory, metadata, runDocker) {
  const name = `factory-egress-${metadata.installationId}`
  const ids = runDocker([
    "container",
    "ls",
    "--all",
    "--filter",
    `name=^/${name}$`,
    "--no-trunc",
    "--format",
    "{{.ID}}",
  ])
    .split(/\r?\n/)
    .filter(Boolean)
  if (ids.length === 0) return
  const refuse = () => {
    throw new Error(
      `Proxy ${name} is incompatible with this preparation; existing metadata was preserved. ` +
        "Follow the verified proxy maintenance procedure in infra/egress/README.md before retrying."
    )
  }
  if (ids.length !== 1 || !/^[0-9a-f]{64}$/.test(ids[0])) refuse()
  let previous
  try {
    const bytes = await readFile(resolve(directory, "egress.json"))
    if (bytes.length > 16384) refuse()
    previous = JSON.parse(bytes.toString("utf8"))
  } catch (error) {
    if (error.code !== "ENOENT" && !(error instanceof SyntaxError)) throw error
    refuse()
  }
  // Never adopt an existing proxy from its mutable configuration. Only the previous
  // prepared contract can establish compatibility, and a changed contract needs maintenance.
  if (
    !previous ||
    typeof previous !== "object" ||
    Array.isArray(previous) ||
    Object.keys(previous).length !== Object.keys(metadata).length ||
    previous.schemaVersion !== metadata.schemaVersion ||
    previous.installationId !== metadata.installationId ||
    previous.imageId !== metadata.imageId ||
    JSON.stringify(previous.hostIpv4) !== JSON.stringify(metadata.hostIpv4)
  )
    refuse()
  const [proxy] = JSON.parse(runDocker(["container", "inspect", ids[0]]))
  const expectedEnv = [
    `FACTORY_HOST_IPV4=${metadata.hostIpv4.join(",")}`,
    "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
  ]
  const expectedLabels = {
    "io.factory.installation": metadata.installationId,
    "io.factory.egress.policy": "1",
    "io.factory.egress.role": "shared",
    "io.factory.egress.image": "1",
    "io.factory.egress.squid": "7.7",
  }
  if (
    proxy?.Id !== ids[0] ||
    proxy.Name !== `/${name}` ||
    proxy.Image !== metadata.imageId ||
    proxy.Config?.Image !== metadata.imageId ||
    JSON.stringify(proxy.Config?.Env) !== JSON.stringify(expectedEnv) ||
    Object.keys(proxy.Config?.Labels ?? {}).length !==
      Object.keys(expectedLabels).length ||
    Object.entries(expectedLabels).some(
      ([key, value]) => proxy.Config.Labels[key] !== value
    )
  )
    refuse()
}

// Runtime creation requires the exclusive Rust supervisor.
export async function prepareEgress(
  directory,
  runDocker = docker,
  reference,
  { checkOnly = false, stageDirectory } = {}
) {
  if (checkOnly && !reference) {
    throw new Error("Deployment checks require a release proxy image")
  }
  const installation = await readFile(
    resolve(directory, "installation.json"),
    "utf8"
  ).then(JSON.parse, (error) => {
    if (!checkOnly || error.code !== "ENOENT") throw error
  })
  if (
    installation !== undefined &&
    (!uuid.test(installation?.installationId) ||
      /^0{8}-0{4}-0{4}-0{4}-0{12}$/.test(installation.installationId))
  ) {
    throw new Error("Invalid egress installation identity")
  }
  const version = JSON.parse(
    runDocker(["version", "--format", "{{json .Server}}"])
  )
  const [major, minor] = String(version.ApiVersion).split(".").map(Number)
  if (
    version.Os !== "linux" ||
    !Number.isInteger(major) ||
    !Number.isInteger(minor) ||
    major < 1 ||
    (major === 1 && minor < 48)
  ) {
    throw new Error("Docker API 1.48+ with isolated bridge support is required")
  }
  const tag = reference ?? `factory-egress:${installation.installationId}`
  if (!reference) {
    runDocker(
      ["build", "--pull", "--tag", tag, resolve(root, "infra/egress")],
      {
        timeout: 1_200_000,
      }
    )
  }
  const [image] = JSON.parse(runDocker(["image", "inspect", tag]))
  if (
    !/^sha256:[0-9a-f]{64}$/.test(image.Id) ||
    image.Os !== "linux" ||
    image.Config?.Labels?.["io.factory.egress.image"] !== "1" ||
    image.Config?.Labels?.["io.factory.egress.squid"] !== "7.7" ||
    image.Config?.User !== "0:0"
  ) {
    throw new Error("Prepared proxy image does not match the egress contract")
  }
  const metadata = {
    schemaVersion: 1,
    installationId: installation?.installationId,
    imageId: image.Id,
    hostIpv4: hostAddresses(undefined, Boolean(reference)),
  }
  if (installation && !stageDirectory) {
    await requireCompatibleProxy(directory, metadata, runDocker)
  }
  if (checkOnly) return
  const output = stageDirectory ?? directory
  const temporary = resolve(output, `egress-${randomUUID()}.tmp`)
  try {
    await writeFile(temporary, JSON.stringify(metadata, null, 2) + "\n", {
      flag: "wx",
      mode: 0o600,
    })
    await rename(temporary, resolve(output, "egress.json"))
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
  if (args.length !== 2 || args[0] !== "--directory" || !args[1]) {
    throw new Error(
      "Usage: node scripts/egress/prepare.mjs --directory <prepared installation directory>"
    )
  }
  await prepareEgress(resolve(args[1]))
  console.log(
    "Egress image prepared; no user container or network has been changed."
  )
}
