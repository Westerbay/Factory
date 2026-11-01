import assert from "node:assert/strict"
import fs from "node:fs"
import path from "node:path"
import os from "node:os"
import { spawnSync } from "node:child_process"
import { test } from "node:test"
import { createRequire } from "node:module"
import { pathToFileURL } from "node:url"
const require = createRequire(import.meta.url)

const expected = {
  minimatch: ["10.2.6"],
  glob: ["11.1.0"],
  deepmerge: ["4.3.1"],
}
// Follow reachable dependency links; obsolete virtual-store entries are not installed dependencies.
function installedPackages() {
  const pending = [path.resolve("node_modules")]
  for (const root of ["apps", "packages", "infra"]) {
    if (fs.existsSync(root))
      for (const entry of fs.readdirSync(root))
        pending.push(path.resolve(root, entry, "node_modules"))
  }
  const seen = new Set()
  const found = new Map()
  while (pending.length) {
    const directory = pending.pop()
    if (!fs.existsSync(directory)) continue
    const real = fs.realpathSync(directory)
    if (seen.has(real)) continue
    seen.add(real)
    for (const entry of fs.readdirSync(directory)) {
      const location = path.join(directory, entry)
      if (entry.startsWith("@")) {
        pending.push(location)
      } else if (!entry.startsWith(".")) {
        pending.push(path.join(location, "node_modules"))
        if (fs.existsSync(location)) {
          let parent = path.dirname(fs.realpathSync(location))
          if (path.basename(parent).startsWith("@"))
            parent = path.dirname(parent)
          if (path.basename(parent) === "node_modules") pending.push(parent)
        }
        if (
          Object.hasOwn(expected, entry) &&
          fs.existsSync(path.join(location, "package.json"))
        ) {
          const manifest = JSON.parse(
            fs.readFileSync(path.join(location, "package.json"), "utf8")
          )
          found.set(fs.realpathSync(location), {
            location: fs.realpathSync(location),
            name: manifest.name,
            version: manifest.version,
          })
        }
      }
    }
  }
  return [...found.values()]
}
const packages = installedPackages()
test("every security-patched dependency version is installed", () => {
  for (const [name, versions] of Object.entries(expected))
    for (const version of versions) {
      assert.ok(
        packages.some((p) => p.name === name && p.version === version),
        `missing ${name}@${version}; run a full frozen install`
      )
    }
})
for (const pkg of packages) {
  if (!expected[pkg.name].includes(pkg.version)) continue
  const label = `${pkg.name}@${pkg.version}`
  if (pkg.name === "minimatch")
    for (const format of ["commonjs", "esm"])
      test(`${label} ${format}: bounded preprocessing preserves ordinary matches`, () => {
        const entry = path.join(pkg.location, "dist", format, "index.js")
        const result = spawnSync(
          process.execPath,
          [
            "-e",
            `
      const assert = require('node:assert/strict');
      const { Minimatch } = require(${JSON.stringify(entry)});
      const options = { optimizationLevel: 2, braceExpandMax: 64 };
      const bounded = new Minimatch(Array(20).fill('**/../a/b').join('/'), options);
      assert.equal(bounded.set.length, 64);
      const ordinary = new Minimatch('**/../a/b/**/../c/d', options);
      assert.equal(ordinary.match('../a/b/../c/d'), true);
      assert.equal(ordinary.match('x/a/b/y/c/d'), true);
      const large = new Minimatch('{x,*}/{0..999}', { optimizationLevel: 2 });
      assert.equal(large.set.length, 2000);
      assert.equal(large.match('y/999'), true);
      assert.equal(large.match('.y/5'), false);
    `,
          ],
          { timeout: 5000 }
        )
        assert.ifError(result.error)
        assert.equal(result.status, 0, result.stderr.toString())
      })
  if (pkg.name === "glob")
    for (const format of ["commonjs", "esm"])
      test(`${label} ${format}: cancellation stops directory reads`, async (t) => {
        const manifest = JSON.parse(
          fs.readFileSync(path.join(pkg.location, "package.json"), "utf8")
        )
        const api =
          format === "commonjs"
            ? require(pkg.location)
            : await import(
                pathToFileURL(path.join(pkg.location, manifest.module)).href
              )
        const { glob, globSync, globStreamSync } = api
        const cwd = fs.mkdtempSync(path.join(os.tmpdir(), "glob-security-"))
        t.after(() => fs.rmSync(cwd, { recursive: true, force: true }))
        for (let i = 0; i < 8; i++) {
          fs.mkdirSync(path.join(cwd, `a${i}`, "b"), { recursive: true })
          fs.writeFileSync(
            path.join(cwd, `a${i}`, "b", "file.txt"),
            "synthetic"
          )
        }
        assert.equal(globSync("**/*.txt", { cwd }).length, 8)
        for (const mode of ["async", "sync-stream"]) {
          const controller = new AbortController()
          const reason = new Error("synthetic cancellation")
          let calls = 0
          const count = () => {
            if (++calls === 1) controller.abort(reason)
          }
          const counted = {
            ...fs,
            readdir(...args) {
              count()
              return fs.readdir(...args)
            },
            readdirSync(...args) {
              count()
              return fs.readdirSync(...args)
            },
          }
          if (mode === "async")
            await assert.rejects(
              glob("**", { cwd, signal: controller.signal, fs: counted }),
              (error) => error === reason
            )
          else
            await new Promise((resolve, reject) => {
              const stream = globStreamSync("**", {
                cwd,
                signal: controller.signal,
                fs: counted,
              })
              stream.on("error", (error) => {
                try {
                  assert.equal(error, reason)
                  resolve()
                } catch (failure) {
                  reject(failure)
                }
              })
              stream.on("end", () => reject(new Error("expected cancellation")))
              stream.resume()
            })
          await new Promise((resolve) => setTimeout(resolve, 100))
          assert.equal(calls, 1, `${mode}: no traversal after abort`)
        }
      })
}

for (const pkg of packages.filter((p) => p.name === "deepmerge")) {
  const merge = require(pkg.location)
  test("untrusted merge keys cannot change the returned prototype", () => {
    const source = JSON.parse('{"__proto__":{"isAdmin":true},"name":"safe"}')
    for (const target of [
      Object.create(null),
      { nested: null },
      JSON.parse('{"__proto__":null}'),
    ]) {
      const result = merge(target, source)
      assert.equal(Object.getPrototypeOf(result), Object.prototype)
      assert.equal(result.isAdmin, undefined)
      assert.equal(result.name, "safe")
    }
    const nested = merge({ nested: null }, { nested: source })
    assert.equal(nested.nested.isAdmin, undefined)
    assert.equal({}.isAdmin, undefined)
  })

  test("poisoned target keys cannot change the returned prototype", () => {
    const result = merge(
      JSON.parse('{"__proto__":{"isAdmin":true},"enabled":true}'),
      {}
    )
    assert.equal(Object.getPrototypeOf(result), Object.prototype)
    assert.equal(result.isAdmin, undefined)
    assert.equal(result.enabled, true)
  })

  test("ordinary merging, symbols, arrays and custom array merging are preserved", () => {
    const key = Symbol("setting")
    const target = { nested: { a: 1 }, values: [1], [key]: "before" }
    const source = { nested: { b: 2 }, values: [2], [key]: "after" }
    assert.deepEqual(merge(target, source), {
      nested: { a: 1, b: 2 },
      values: [1, 2],
      [key]: "after",
    })
    assert.deepEqual(target.values, [1])
    assert.deepEqual(merge.all([{ a: 1 }, { b: 2 }]), { a: 1, b: 2 })
    assert.deepEqual(
      merge([1], [2], { arrayMerge: (_, source) => source }),
      [2]
    )
  })
}
