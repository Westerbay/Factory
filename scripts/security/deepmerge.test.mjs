import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { test } from "node:test"

// Exercise the patched dependency used by the web build, without scanning node_modules.
// Remove these regressions with the local patch once an upstream fix replaces it.
const webRequire = createRequire(
  new URL("../../apps/web/package.json", import.meta.url)
)
const pwaRequire = createRequire(webRequire.resolve("vite-plugin-pwa"))
const workboxRequire = createRequire(pwaRequire.resolve("workbox-build"))
const rollupRequire = createRequire(
  workboxRequire.resolve("@rollup/plugin-node-resolve")
)
const merge = rollupRequire("deepmerge")

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
  assert.deepEqual(merge([1], [2], { arrayMerge: (_, source) => source }), [2])
})
