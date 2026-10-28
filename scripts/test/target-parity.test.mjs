import test from "node:test"
import assert from "node:assert/strict"
import { verifyDatabaseTarget } from "../../packages/database/dist/target.js"
import { targetFixtures } from "../database/target-fixtures.mjs"

test("TypeScript ownership proof agrees with the shared Rust acceptance vectors", () => {
  for (const { name, accepted, proof } of targetFixtures) {
    if (accepted) {
      assert.doesNotThrow(() => verifyDatabaseTarget(proof), name)
    } else {
      assert.throws(() => verifyDatabaseTarget(proof), /Refusing/, name)
    }
  }
})
