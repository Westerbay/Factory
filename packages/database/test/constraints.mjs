import assert from "node:assert/strict"
import postgres from "postgres"
/** Test-only privileged SQL is restricted to the runner's freshly verified owned fixture. */
export async function verifyRelationalConstraints(proof, runId) {
  const client = postgres({
    host: proof.host,
    port: proof.port,
    database: proof.database,
    username: proof.username,
    password: proof.password,
    max: 1,
    onnotice: () => {},
  })
  try {
    // A real agent in a different execution must not satisfy a same-run relation.
    await client.begin(async (tx) => {
      await tx`insert into runs(id,"rootAgentId",version,status,"createdAt",projection) values ('foreign-run','foreign-agent',1,'completed','2026-10-27',${tx.json({ id: "foreign-run", rootAgentId: "foreign-agent", version: 1, status: "completed" })})`
      await tx`insert into agents("runId",id) values ('foreign-run','foreign-agent')`
    })
    for (const table of ["questions", "permissions", "messages"]) {
      await assert.rejects(
        client`insert into ${client(table)}("runId",id,"agentId") values (${runId},'cross-run','foreign-agent')`,
        (error) => error.code === "23503"
      )
    }
    await assert.rejects(
      client`insert into agents("runId",id,"parentId") values (${runId},'cross-child','foreign-agent')`,
      (error) => error.code === "23503"
    )
    const [rootConstraint] =
      await client`select condeferrable, condeferred from pg_constraint where conname='runs_root_owned'`
    assert.deepEqual(rootConstraint, { condeferrable: true, condeferred: true })
    // Inject a database-side failure after the projection update; the event and recovery
    // projection must both roll back. No hooks exist in the production store for this.
    await client`create function fixture_reject_recovery() returns trigger language plpgsql as $$ begin if NEW.event->>'type' = 'run-interrupted' then raise exception 'synthetic recovery failure'; end if; return NEW; end $$`
    await client`create trigger fixture_reject_recovery before insert on events for each row execute function fixture_reject_recovery()`
    return async () => {
      await client`drop trigger fixture_reject_recovery on events`
      await client`drop function fixture_reject_recovery()`
      await client`delete from runs where id='foreign-run'`
      await client.end()
    }
  } catch (error) {
    await client.end()
    throw error
  }
}
