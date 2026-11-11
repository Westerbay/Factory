import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const docker = `#!/bin/sh
case "$*" in
  *" compose "*" pull "*|*" image inspect "*) exit 0 ;;
  *" inspect "*" factory-prod-caddy-1") echo "$INSTALL/previous/web-dist" ;;
  *" api workspace upgrade check")
    echo check >> "$TRACE"
    [ "$BLOCK" != preflight ] ;;
  *" api workspace upgrade apply")
    echo apply >> "$TRACE"
    [ "$BLOCK" != after-stop ] || exit 1
    touch "$INSTALL/state/.factory/environments/upgrade/mutating" ;;
  *" api workspace upgrade rollback") echo rollback >> "$TRACE" ;;
  *" stop --timeout 120 api") echo stop >> "$TRACE" ;;
  *" up -d "*) echo up >> "$TRACE"; [ "$BLOCK" != startup ] ;;
  *) exit 2 ;;
esac
`

const prepare = `#!/bin/sh
for output do :; done
printf '%s\\n' "$WORKSPACE_IMAGE" > "$output/prepared.json"
printf '%s\\n' 'proxy' > "$output/egress.json"
echo stage >> "$TRACE"
`

test(
  "deployment preserves running workspaces or transitions and restores the previous release on failure",
  { skip: process.platform === "win32" },
  async (t) => {
    const cases = [
      { block: "preflight", status: 1, trace: ["stage", "check"] },
      {
        block: "after-stop",
        status: 1,
        trace: ["stage", "check", "stop", "apply", "restore"],
      },
      {
        block: "startup",
        status: 1,
        trace: [
          "stage",
          "check",
          "stop",
          "apply",
          "up",
          "stop",
          "rollback",
          "restore",
        ],
      },
      {
        block: "changed",
        status: 0,
        trace: ["stage", "check", "stop", "apply", "up"],
      },
      {
        block: "compatible",
        status: 0,
        trace: ["stage", "check", "up"],
      },
    ]
    for (const { block, status, trace } of cases) {
      await t.test(block || "compatible release", async () => {
        const directory = await realpath(
          await mkdtemp(join(tmpdir(), "factory-deploy-test-"))
        )
        try {
          for (const path of [
            "bin",
            "bootstrap",
            "bundle/infra/deploy",
            "bundle/web-dist",
            "previous/infra/deploy",
            "state/.factory/environments",
          ]) {
            await mkdir(join(directory, path), { recursive: true })
          }
          const scripts = {
            docker,
            node: prepare,
            flock: "#!/bin/sh\nexit 0\n",
            mv: '#!/bin/sh\n[ "$1" != -T ] || shift\n[ "$1" != -- ] || shift\nexec /bin/mv "$@"\n',
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
          await writeFile(
            join(directory, "previous/release.env"),
            images.join("")
          )
          await writeFile(
            join(directory, "previous/infra/deploy/server.sh"),
            '#!/bin/sh\necho restore >> "$TRACE"\n',
            { mode: 0o700 }
          )
          const workspaceState = join(directory, "state/.factory/environments")
          await writeFile(join(workspaceState, "prepared.json"), "previous\n")
          await writeFile(join(workspaceState, "egress.json"), "proxy\n")
          const script = join(directory, "bundle/infra/deploy/server.sh")
          for (const name of ["server.sh", "upgrade.sh"]) {
            await copyFile(
              new URL(`../../infra/deploy/${name}`, import.meta.url),
              join(directory, "bundle/infra/deploy", name)
            )
          }
          const tracePath = join(directory, "trace")
          const result = spawnSync("bash", [script, directory, "deploy"], {
            env: {
              PATH: `${join(directory, "bin")}:${process.env.PATH}`,
              TRACE: tracePath,
              BLOCK: block,
              INSTALL: directory,
              WORKSPACE_IMAGE: block === "compatible" ? "previous" : "next",
            },
            encoding: "utf8",
            timeout: 10_000,
          })
          assert.equal(result.status, status, result.stderr)
          assert.deepEqual(
            (await readFile(tracePath, "utf8")).trim().split("\n"),
            trace
          )
          assert.equal(
            await readFile(join(workspaceState, "prepared.json"), "utf8"),
            block === "changed" ? "next\n" : "previous\n"
          )
        } finally {
          await rm(directory, { recursive: true })
        }
      })
    }
  }
)
