import { execFileSync } from "node:child_process"
import { mkdtempSync, rmdirSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

// No ambient Docker context, endpoint, auth config, build args, or real .env files.
export function docker(args, { timeout = 30_000 } = {}) {
  const config = mkdtempSync(join(tmpdir(), "factory-environment-docker-"))
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
    // Do not recursively delete an unexpected file created by a CLI plugin.
    rmdirSync(config)
  }
}
