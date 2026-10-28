import test from "node:test"
import assert from "node:assert/strict"
import { boundaryViolations } from "../rust/boundaries.mjs"

function metadata(name, dependencies) {
  return {
    workspace_members: [name],
    packages: [{ id: name, name, dependencies }],
  }
}
const local = (name, extra = {}) => ({
  name,
  path: `/fixture/${name}`,
  ...extra,
})

test("application may use core while provider/database composition stays in API", () => {
  assert.deepEqual(
    boundaryViolations(
      metadata("factory-application", [local("factory-core")])
    ),
    []
  )
  assert.deepEqual(
    boundaryViolations(
      metadata("factory-api", [
        local("factory-database"),
        local("factory-adapters"),
      ])
    ),
    []
  )
})

test("aliased and platform-specific workspace dependencies cannot reverse boundaries", () => {
  for (const extra of [
    { rename: "innocent" },
    { target: "cfg(windows)" },
    { kind: "dev" },
  ]) {
    assert.equal(
      boundaryViolations(
        metadata("factory-core", [local("factory-api", extra)])
      ).length,
      1
    )
    assert.equal(
      boundaryViolations(
        metadata("factory-application", [local("factory-database", extra)])
      ).length,
      1
    )
  }
})

test("domain cannot import infrastructure and unknown local crates fail closed", () => {
  assert.equal(
    boundaryViolations(
      metadata("factory-core", [{ name: "sqlx", rename: "storage" }])
    ).length,
    1
  )
  assert.equal(
    boundaryViolations(
      metadata("factory-application", [{ name: "reqwest", kind: "build" }])
    ).length,
    1
  )
  assert.equal(
    boundaryViolations(
      metadata("factory-adapters", [local("factory-database")])
    ).length,
    1
  )
  assert.equal(
    boundaryViolations(metadata("factory-unclassified", [])).length,
    1
  )
})

test("async application tests may use Tokio without granting a production dependency", () => {
  assert.deepEqual(
    boundaryViolations(
      metadata("factory-application", [{ name: "tokio", kind: "dev" }])
    ),
    []
  )
  assert.equal(
    boundaryViolations(
      metadata("factory-application", [{ name: "tokio", kind: null }])
    ).length,
    1
  )
  assert.equal(
    boundaryViolations(
      metadata("factory-core", [{ name: "tokio", kind: "dev" }])
    ).length,
    1
  )
})
