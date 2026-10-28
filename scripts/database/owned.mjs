import { execFileSync } from "node:child_process"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { verifyDatabaseTarget } from "../../packages/database/dist/index.js"
export const root = resolve(fileURLToPath(new URL("../..", import.meta.url)))
export function docker(args, environment = {}) {
  // Never let Compose load a real .env or caller-controlled Docker/Compose settings.
  const windows = process.platform === "win32"
  const host = windows
    ? "npipe:////./pipe/docker_engine"
    : "unix:///var/run/docker.sock"
  const env = { PATH: process.env.PATH, ...environment }
  if (windows) {
    // Windows process startup and Docker's installed CLI plugin discovery need these.
    for (const name of ["SystemRoot", "USERPROFILE", "ProgramFiles"]) {
      if (process.env[name]) env[name] = process.env[name]
    }
  }
  try {
    return execFileSync("docker", ["--host", host, ...args], {
      encoding: "utf8",
      env,
      stdio: ["ignore", "pipe", "pipe"],
    }).trim()
  } catch (error) {
    const detail = String(error.stderr ?? error.message)
      .replace(/postgres(?:ql)?:\/\/[^\s]+/gi, "[redacted database URL]")
      .replace(/(?:POSTGRES_PASSWORD|PASSWORD)=[^\s]+/gi, "PASSWORD=[redacted]")
      .replaceAll("local-fixture-only", "[redacted]")
      .slice(0, 1800)
    throw new Error(`Factory Docker operation failed: ${detail}`)
  }
}
export function compose(target, args) {
  verifyDatabaseTarget({
    ...target,
    labels: {
      "com.docker.compose.project": target.project,
      "io.factory.owner": target.owner,
      "io.factory.directory": target.directory,
    },
    publishedHost: target.host,
    publishedPort: target.port,
  })
  return docker(
    [
      "compose",
      "--env-file",
      resolve(root, "scripts/database/empty-env"),
      "-f",
      resolve(root, "compose.yml"),
      "-p",
      target.project,
      ...args,
    ],
    {
      FACTORY_DB_NAME: target.database,
      FACTORY_DB_PORT: String(target.port),
      FACTORY_DB_OWNER: target.owner,
      FACTORY_DB_DIRECTORY: target.directory,
    }
  )
}
export function inspectTarget(target, { allowStopped = false } = {}) {
  const [container] = JSON.parse(docker(["inspect", target.container]))
  const configured = container.HostConfig?.PortBindings?.["5432/tcp"]
  if (
    !Array.isArray(configured) ||
    configured.length !== 1 ||
    configured[0].HostIp !== "127.0.0.1" ||
    Number(configured[0].HostPort) !== target.port
  )
    throw new Error("Refusing database operation: configured port differs")
  const binding = allowStopped
    ? configured
    : container.NetworkSettings?.Ports?.["5432/tcp"]
  if (!Array.isArray(binding) || binding.length !== 1)
    throw new Error("Refusing database operation: ambiguous published port")
  const env = container.Config?.Env ?? []
  if (
    !env.includes(`POSTGRES_DB=${target.database}`) ||
    !env.includes(`POSTGRES_USER=${target.username}`) ||
    !env.includes(`POSTGRES_PASSWORD=${target.password}`)
  )
    throw new Error(
      "Refusing database operation: container database identity differs"
    )
  const volume = ownedVolume(target)
  const dataMounts =
    container.Mounts?.filter(
      (m) => m.Destination === "/var/lib/postgresql/data"
    ) ?? []
  if (
    dataMounts.length !== 1 ||
    dataMounts[0].Type !== "volume" ||
    dataMounts[0].Name !== volume
  )
    throw new Error(
      "Refusing database operation: container data volume differs"
    )
  return verifyDatabaseTarget({
    ...target,
    labels: container.Config?.Labels ?? {},
    publishedHost: binding[0].HostIp,
    publishedPort: Number(binding[0].HostPort),
  })
}
export function ownedVolume(target) {
  const name = `${target.project}_postgres_data`
  const [volume] = JSON.parse(docker(["volume", "inspect", name]))
  if (
    volume.Labels?.["io.factory.owner"] !== target.owner ||
    volume.Labels?.["io.factory.directory"] !== target.directory ||
    volume.Labels?.["com.docker.compose.project"] !== target.project
  )
    throw new Error("Refusing cleanup: volume identity differs")
  return name
}
export function ownedNetwork(target) {
  const name = `${target.project}_default`
  const [proof] = JSON.parse(docker(["network", "inspect", name]))
  if (
    proof.Labels?.["com.docker.compose.project"] !== target.project ||
    proof.Labels?.["io.factory.owner"] !== target.owner ||
    proof.Labels?.["io.factory.directory"] !== target.directory
  )
    throw new Error("Refusing operation: network identity differs")
  return name
}
export function cleanupFixture(target) {
  if (!/^factory-test-[a-z0-9]+$/.test(target.project))
    throw new Error("Refusing cleanup: only ephemeral fixtures may be deleted")
  const containerExists = docker([
    "ps",
    "-a",
    "--filter",
    `name=^/${target.container}$`,
    "--format",
    "{{.ID}}",
  ])
  if (containerExists) inspectTarget(target, { allowStopped: true })
  const volume = `${target.project}_postgres_data`
  const volumeExists = docker([
    "volume",
    "ls",
    "--filter",
    `name=^${volume}$`,
    "--format",
    "{{.Name}}",
  ])
  if (volumeExists) ownedVolume(target)
  const network = `${target.project}_default`
  const networkExists = docker([
    "network",
    "ls",
    "--filter",
    `name=^${network}$`,
    "--format",
    "{{.ID}}",
  ])
  if (networkExists) ownedNetwork(target)
  // Verify every existing resource before removing any exact owned resource.
  if (containerExists) docker(["rm", "-f", target.container])
  if (volumeExists) docker(["volume", "rm", volume])
  if (networkExists) docker(["network", "rm", network])
}
