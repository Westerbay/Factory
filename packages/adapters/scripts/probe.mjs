import { CodexRuntime, createStdioConnector } from "../dist/index.js"
const executable = process.argv[2]
if (!executable)
  throw new Error(
    "Pass the absolute path of the pinned installed Codex executable"
  )
const runtime = new CodexRuntime({ connect: createStdioConnector(executable) })
try {
  const result = await runtime.probe()
  console.log(JSON.stringify(result, null, 2))
  if (!result.available) process.exitCode = 1
} finally {
  await runtime.close()
}
