/** Sanitized identity proof obtained from Docker inspection, never an arbitrary URL. */
export interface DatabaseTarget {
  project: string
  owner: string
  directory: string
  database: string
  username: string
  password: string
  host: string
  port: number
  container: string
  labels: Record<string, string>
  publishedHost: string
  publishedPort: number
}
export function verifyDatabaseTarget(target: DatabaseTarget): DatabaseTarget {
  const validProject =
    target.project === "factory-local" ||
    /^factory-test-[a-z0-9]+$/.test(target.project)
  const expectedDatabase =
    target.project === "factory-local" ? "factory_local" : "factory_test"
  const validIdentity =
    validProject &&
    /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(
      target.owner
    ) &&
    target.directory.startsWith("/") &&
    target.container === `${target.project}-postgres-1`
  const validLabels =
    target.labels["com.docker.compose.project"] === target.project &&
    target.labels["io.factory.owner"] === target.owner &&
    target.labels["io.factory.directory"] === target.directory
  const validConnection =
    target.host === "127.0.0.1" &&
    target.publishedHost === target.host &&
    target.port === target.publishedPort &&
    Number.isInteger(target.port) &&
    target.port > 1024 &&
    target.port < 65536 &&
    target.database === expectedDatabase &&
    target.username === "factory" &&
    target.password === "local-fixture-only"
  if (!validIdentity || !validLabels || !validConnection)
    throw new Error(
      "Refusing database operation: unverified Factory-owned local target"
    )
  return { ...target, labels: { ...target.labels } }
}
