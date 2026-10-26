// Compile-time parity across independent wire/domain boundaries; runtime validation is separate.
import type { Run, RunEvent } from "@workspace/core"
import type { RunProjection, MaterialEvent } from "../src/index.js"
export function domainRun(projection: RunProjection): Run {
  return projection
}
export function wireRun(run: Run): RunProjection {
  return run
}
export function domainEvent(event: MaterialEvent): RunEvent {
  return event
}
export function wireEvent(event: RunEvent): MaterialEvent {
  return event
}
