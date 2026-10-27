import { CodexRuntime, createStdioConnector } from "@workspace/adapters"
import { desktopEventSchema } from "@workspace/contracts"
import { DesktopHost } from "./host.js"
import { openOwnedDatabase } from "./database.js"
import { ObservationRelay } from "./observation-relay.js"
import { serveWorker } from "./worker-loop.js"
const generation = process.argv[2]
const port = process.parentPort
if (!generation || !port)
  throw new Error("Factory runtime requires an owned utility process")
serveWorker(port, generation, async (send) => {
  const database = await openOwnedDatabase()
  const runtime = new CodexRuntime({ connect: createStdioConnector("codex") })
  const relay = new ObservationRelay((event) =>
    send({ type: "event", generation, event: desktopEventSchema.parse(event) })
  )
  return new DesktopHost(database, runtime, relay.publish, () => relay.close())
})
