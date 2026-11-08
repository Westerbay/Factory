import { execFileSync } from "node:child_process"
import { mkdtempSync, realpathSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { basename, dirname, join } from "node:path"

// No ambient Docker context, endpoint, auth config, build args, or real .env files.
export function docker(args, { timeout = 30_000, logBuild = false } = {}) {
  const showBuildOutput = logBuild && args[0] === "build"
  const temporaryRoot = realpathSync(tmpdir())
  const prefix = "factory-environment-docker-"
  const config = mkdtempSync(join(temporaryRoot, prefix))
  if (
    dirname(config) !== temporaryRoot ||
    !basename(config).startsWith(prefix)
  ) {
    throw new Error("Refusing to use an unowned Docker configuration directory")
  }
  const env = {}
  for (const key of ["PATH", "SystemRoot", "ProgramFiles", "TEMP", "TMP"]) {
    if (process.env[key]) env[key] = process.env[key]
  }
  try {
    const output = execFileSync(
      "docker",
      [
        "--config",
        config,
        "--host",
        process.platform === "win32"
          ? "npipe:////./pipe/docker_engine"
          : "unix:///var/run/docker.sock",
        ...args,
      ],
      {
        env,
        encoding: "utf8",
        timeout,
        maxBuffer: 4 * 1024 * 1024,
        stdio: showBuildOutput
          ? ["ignore", "inherit", "inherit"]
          : ["ignore", "pipe", "pipe"],
      }
    )
    return output?.trim() ?? ""
  } catch (error) {
    // Regular calls expose only machine status. The integration fixture can
    // stream builds of the repository Dockerfiles for CI diagnostics.
    const details = []
    if (typeof error.code === "string" && /^E[A-Z0-9_]{1,40}$/.test(error.code))
      details.push(`code=${error.code}`)
    if (Number.isInteger(error.status)) details.push(`status=${error.status}`)
    if (
      typeof error.signal === "string" &&
      /^SIG[A-Z0-9]{1,20}$/.test(error.signal)
    )
      details.push(`signal=${error.signal}`)
    throw new Error(
      "Local environment Docker operation failed or timed out" +
        (details.length ? ` (${details.join(", ")})` : "")
    )
  } finally {
    // Buildx writes metadata here. Remove only this invocation's private directory;
    // rmSync removes symlinks themselves without traversing their targets.
    rmSync(config, { recursive: true, force: true, maxRetries: 3 })
  }
}
