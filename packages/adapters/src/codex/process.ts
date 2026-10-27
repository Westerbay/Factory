import { execFile, spawn } from "node:child_process"
import { promisify } from "node:util"
import { ExecutionError } from "@workspace/core"
import { JsonRpcTransport } from "./transport.js"
import { PINNED_CODEX_VERSION } from "./protocol.js"

export interface CodexConnection {
  transport: JsonRpcTransport
  version: string
}
export type CodexConnector = () => Promise<CodexConnection>
/** No shell, config overrides, login flow, or token file access. */
export function createStdioConnector(executable: string): CodexConnector {
  return async () => {
    let version: string
    try {
      const result = await promisify(execFile)(executable, ["--version"], {
        timeout: 5_000,
        maxBuffer: 4_096,
        encoding: "utf8",
      })
      version = result.stdout.trim().replace(/^codex-cli /, "")
    } catch {
      throw new ExecutionError("unavailable", "Codex executable unavailable")
    }
    if (version !== PINNED_CODEX_VERSION)
      throw new ExecutionError(
        "unsupported",
        `Codex ${PINNED_CODEX_VERSION} is required; regenerate and review bindings before upgrading`
      )
    const child = spawn(executable, ["app-server", "--listen", "stdio://"], {
      shell: false,
      stdio: ["pipe", "pipe", "pipe"],
    })
    return { transport: new JsonRpcTransport(child), version }
  }
}
