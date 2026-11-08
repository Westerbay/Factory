import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const docker = `#!/bin/sh
case "$*" in
  *" compose "*" pull"|*" image pull "*) exit 0 ;;
  *" ps --status running --quiet api") echo existing-api ;;
  *" stop api") echo stop >> "$TRACE" ;;
  *" start api") echo start >> "$TRACE" ;;
  *" up -d "*) echo up >> "$TRACE" ;;
  *) exit 2 ;;
esac
`

const prepare = `#!/bin/sh
case "$*" in
  *" --check")
    echo check >> "$TRACE"
    if [ "$BLOCK" = preflight ] || { [ "$BLOCK" = after-stop ] && grep -qx stop "$TRACE"; }; then
      exit 1
    fi
    ;;
  *) echo prepare >> "$TRACE" ;;
esac
`

test(
  "deployment checks run before shutdown and a late refusal restores the API",
  { skip: process.platform === "win32" },
  async (t) => {
    const cases = [
      { block: "preflight", status: 1, trace: ["check"] },
      {
        block: "after-stop",
        status: 1,
        trace: ["check", "stop", "check", "start"],
      },
      {
        block: "",
        status: 0,
        trace: ["check", "stop", "check", "prepare", "up"],
      },
    ]
    for (const { block, status, trace } of cases) {
      await t.test(block || "compatible release", async () => {
        const directory = await mkdtemp(join(tmpdir(), "factory-deploy-test-"))
        try {
          for (const path of [
            "bin",
            "bootstrap",
            "bundle/infra/deploy",
            "bundle/web-dist",
          ]) {
            await mkdir(join(directory, path), { recursive: true })
          }
          const scripts = {
            docker,
            node: prepare,
            flock: "#!/bin/sh\nexit 0\n",
            uname:
              '#!/bin/sh\ncase "$1" in -s) echo Linux ;; -m) echo x86_64 ;; *) exit 2 ;; esac\n',
          }
          for (const [name, source] of Object.entries(scripts)) {
            await writeFile(join(directory, "bin", name), source, {
              mode: 0o700,
            })
          }
          for (const asset of ["index.html", "sw.js", "manifest.webmanifest"]) {
            await writeFile(
              join(directory, "bundle/web-dist", asset),
              "fixture"
            )
          }
          const images = ["API", "WORKSPACE", "EGRESS"].map(
            (name) =>
              `FACTORY_${name}_IMAGE=ghcr.io/example/factory-${name.toLowerCase()}@sha256:${"a".repeat(64)}\n`
          )
          await writeFile(
            join(directory, "bundle/release.env"),
            images.join("")
          )
          const script = join(directory, "bundle/infra/deploy/server.sh")
          await copyFile(
            new URL("../../infra/deploy/server.sh", import.meta.url),
            script
          )
          const tracePath = join(directory, "trace")
          const result = spawnSync("bash", [script, directory, "deploy"], {
            env: {
              PATH: `${join(directory, "bin")}:${process.env.PATH}`,
              TRACE: tracePath,
              BLOCK: block,
            },
            encoding: "utf8",
            timeout: 10_000,
          })
          assert.equal(result.status, status, result.stderr)
          assert.deepEqual(
            (await readFile(tracePath, "utf8")).trim().split("\n"),
            trace
          )
        } finally {
          await rm(directory, { recursive: true })
        }
      })
    }
  }
)
