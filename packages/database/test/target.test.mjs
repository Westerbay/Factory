import assert from "node:assert/strict"
import test from "node:test"
import { verifyDatabaseTarget } from "../dist/target.js"
const proof = {
  project: "factory-test-abc",
  owner: "b52c14b8-a06e-48ee-ab1d-2194cda60568",
  directory: "/tmp/factory-fixture",
  database: "factory_test",
  username: "factory",
  password: "local-fixture-only",
  host: "127.0.0.1",
  port: 54321,
  container: "factory-test-abc-postgres-1",
  labels: {
    "com.docker.compose.project": "factory-test-abc",
    "io.factory.owner": "b52c14b8-a06e-48ee-ab1d-2194cda60568",
    "io.factory.directory": "/tmp/factory-fixture",
  },
  publishedHost: "127.0.0.1",
  publishedPort: 54321,
}
test("reject remote, shared, unknown and mismatched ownership before connecting", () => {
  assert.equal(verifyDatabaseTarget(proof).database, "factory_test")
  for (const change of [
    { host: "example.com" },
    { host: "localhost" },
    { username: "other" },
    { password: "other" },
    { directory: "relative" },
    { publishedHost: "0.0.0.0" },
    { container: "shared-postgres" },
    { port: 80, publishedPort: 80 },
    { project: "shared" },
    { database: "template" },
    { owner: "unknown" },
    { publishedPort: 123 },
    { labels: { ...proof.labels, "io.factory.owner": "other" } },
  ])
    assert.throws(
      () => verifyDatabaseTarget({ ...proof, ...change }),
      /Refusing/
    )
})
