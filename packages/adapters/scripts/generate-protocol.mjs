import { execFileSync } from "node:child_process"
import {
  mkdtempSync,
  readFileSync,
  writeFileSync,
  mkdirSync,
  readdirSync,
  rmSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve, dirname } from "node:path"
import { createHash } from "node:crypto"

const binary = process.argv[2]
if (!binary)
  throw new Error("Pass the absolute path of the installed Codex executable")
const version = execFileSync(binary, ["--version"], {
  encoding: "utf8",
  timeout: 5000,
}).trim()
if (version !== "codex-cli 0.160.0")
  throw new Error(
    "Only codex-cli 0.160.0 is supported; review protocol changes explicitly"
  )
const roots = [
  "InitializeParams.ts",
  "InitializeResponse.ts",
  "v2/ToolRequestUserInputParams.ts",
  "v2/ToolRequestUserInputResponse.ts",
  "v2/ServerRequestResolvedNotification.ts",
  "v2/SkillsListParams.ts",
  "v2/GetAccountParams.ts",
  "v2/TurnInterruptParams.ts",
  "v2/TurnSteerParams.ts",
]
const inspected = [
  "ServerNotification.ts",
  "ServerRequest.ts",
  "v2/ThreadItem.ts",
  "v2/ThreadStartParams.ts",
  "v2/ThreadResumeParams.ts",
  "v2/TurnStartParams.ts",
  "v2/GetAccountResponse.ts",
  "v2/SkillsListResponse.ts",
]
const temp = mkdtempSync(join(tmpdir(), "factory-protocol-"))
const output = resolve("packages/adapters/src/codex/generated")
try {
  execFileSync(
    binary,
    ["app-server", "generate-ts", "--experimental", "--out", temp],
    { timeout: 30000, stdio: "pipe" }
  )
  const seen = new Set()
  const files = new Map()
  const visit = (file) => {
    if (seen.has(file)) return
    seen.add(file)
    const source = readFileSync(join(temp, file), "utf8")
    for (const match of source.matchAll(/from "([^"]+)"/g))
      visit(join(dirname(file), match[1] + ".ts"))
    files.set(file, source.replace(/from "([^"]+)"/g, 'from "$1.js"'))
  }
  roots.forEach(visit)
  const hashes = {}
  for (const file of [...seen, ...inspected].sort())
    hashes[file] = createHash("sha256")
      .update(readFileSync(join(temp, file)))
      .digest("hex")
  files.set(
    "provenance.json",
    JSON.stringify(
      { generator: version, experimental: true, roots, sha256: hashes },
      null,
      2
    ) + "\n"
  )
  if (process.argv.includes("--check")) {
    const existing = []
    const walk = (dir) => {
      for (const item of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, item.name)
        if (item.isDirectory()) walk(path)
        else existing.push(path.slice(output.length + 1))
      }
    }
    walk(output)
    if (existing.length !== files.size)
      throw new Error("Generated protocol file inventory drift")
    for (const [file, source] of files)
      if (readFileSync(join(output, file), "utf8") !== source)
        throw new Error(`Generated protocol drift: ${file}`)
    console.log(
      `Verified ${files.size} generated protocol files against ${version}`
    )
  } else {
    mkdirSync(output, { recursive: true })
    for (const [file, source] of files) {
      mkdirSync(dirname(join(output, file)), { recursive: true })
      writeFileSync(join(output, file), source)
    }
    console.log(`Generated ${files.size} protocol files from ${version}`)
  }
} finally {
  rmSync(temp, { recursive: true, force: true })
}
