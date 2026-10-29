import { execFileSync } from "node:child_process"
import { mkdtempSync, realpathSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { basename, dirname, join } from "node:path"

// No ambient Docker context, endpoint, auth config, build args, or real .env files.
export function docker(args, { timeout = 30_000 } = {}) {
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
    return execFileSync(
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
        stdio: ["ignore", "pipe", "pipe"],
      }
    ).trim()
  } catch {
    throw new Error("Local environment Docker operation failed or timed out")
  } finally {
    // Buildx writes metadata here. Remove only this invocation's private directory;
    // rmSync removes symlinks themselves without traversing their targets.
    rmSync(config, { recursive: true, force: true, maxRetries: 3 })
  }
}
