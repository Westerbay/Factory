import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  stat,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { join } from "node:path"
import test from "node:test"

// The fixture executes stub commands, so use the checkout instead of a potentially noexec /tmp.
const scratch = fileURLToPath(
  new URL("../../node_modules/.cache/deploy-test/", import.meta.url)
)

const docker = `#!/bin/sh
case "$*" in
  *" compose "*" pull "*|*" image inspect "*) exit 0 ;;
  *" container inspect factory-prod-caddy-1") [ "$BLOCK" != bootstrap-retry ] ;;
  *" inspect "*" factory-prod-caddy-1") echo "$INSTALL/previous/web-dist" ;;
  *" api workspace upgrade check")
    echo check >> "$TRACE"
    [ "$BLOCK" != preflight ] ;;
  *" api workspace upgrade apply")
    echo apply >> "$TRACE"
    [ "$BLOCK" != after-stop ] || exit 1
    touch "$INSTALL/state/.factory/environments/upgrade/mutating" ;;
  *" api workspace upgrade rollback") echo rollback >> "$TRACE" ;;
  *" exec -T postgres psql "*"--command "*) cat "$TRACE.version" ;;
  *" exec -T postgres pg_dump "*) echo backup >> "$TRACE"; echo dump ;;
  *" exec -T postgres psql "*"--single-transaction"*)
    cat > "$TRACE.sql"
    echo migrate >> "$TRACE"
    [ "$BLOCK" != migration-failure ] || exit 1
    echo 6 > "$TRACE.version" ;;
  *" stop --timeout 120 api") echo stop >> "$TRACE" ;;
  *" up -d "*) echo up >> "$TRACE"; [ "$BLOCK" != startup ] && [ "$BLOCK" != migrated-startup ] ;;
  *) exit 2 ;;
esac
`

const prepare = `#!/bin/sh
for output do :; done
case "$output" in
  --check) echo check >> "$TRACE"; exit 0 ;;
  ghcr.io/*) echo prepare >> "$TRACE"; exit 0 ;;
esac
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
      {
        block: "schema-upgrade",
        version: 5,
        status: 0,
        trace: ["stage", "check", "stop", "backup", "migrate", "up"],
      },
      {
        block: "migration-failure",
        version: 5,
        status: 1,
        trace: ["stage", "check", "stop", "backup", "migrate", "restore"],
      },
      {
        block: "migrated-startup",
        version: 5,
        status: 1,
        trace: ["stage", "check", "stop", "backup", "migrate", "up"],
      },
      {
        block: "bootstrap-retry",
        status: 0,
        trace: ["check", "prepare", "up"],
      },
    ]
    for (const { block, status, trace, version = 6 } of cases) {
      await t.test(block || "compatible release", async () => {
        await mkdir(scratch, { recursive: true })
        const directory = await realpath(await mkdtemp(join(scratch, "run-")))
        try {
          for (const path of [
            "bin",
            "bootstrap",
            "bundle/infra/deploy",
            "bundle/crates/database/schema-upgrades",
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
          await copyFile(
            new URL(
              "../../crates/database/schema-upgrades/0006-attachments.sql",
              import.meta.url
            ),
            join(
              directory,
              "bundle/crates/database/schema-upgrades/0006-attachments.sql"
            )
          )
          const tracePath = join(directory, "trace")
          await writeFile(tracePath + ".version", String(version))
          const result = spawnSync("bash", [script, directory, "deploy"], {
            env: {
              PATH: `${join(directory, "bin")}:${process.env.PATH}`,
              TRACE: tracePath,
              BLOCK: block,
              INSTALL: directory,
              WORKSPACE_IMAGE:
                block === "compatible" || version === 5 ? "previous" : "next",
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
          if (version === 5) {
            const backups = await readdir(join(directory, "backups"))
            assert.equal(backups.length, 1)
            assert.equal(
              (await stat(join(directory, "backups", backups[0]))).mode & 0o777,
              0o600
            )
            assert.match(
              await readFile(tracePath + ".sql", "utf8"),
              /CREATE TABLE factory_identity.conversation_attachments/
            )
            assert.equal(
              (await readFile(tracePath + ".version", "utf8")).trim(),
              block === "migration-failure" ? "5" : "6"
            )
            if (block === "migrated-startup") {
              assert.match(result.stderr, /Retry the same bundle/)
              assert.ok(
                (await readdir(join(workspaceState, "upgrade"))).includes(
                  "schema-upgrade"
                )
              )
            }
          }
        } finally {
          await rm(directory, { recursive: true })
        }
      })
    }
  }
)
