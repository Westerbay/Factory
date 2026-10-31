import { Recovery } from "../../features/conversations/recovery"
import { environmentProblem } from "../../features/environments/api"
import type {
  EnvironmentController,
  EnvironmentState,
} from "../../features/environments/environment-controller"
import { Button } from "../ui/button"
import { LoadingState } from "../ui/loading-state"

export function EnvironmentView({
  state,
  controller,
}: {
  state: EnvironmentState
  controller: EnvironmentController
}) {
  const environment = state.environment
  const message = state.message ?? environmentProblem(environment?.errorCode)
  const stopping =
    state.pending === "stop" || environment?.status === "stopping"
  const starting =
    state.pending === "initialize" ||
    state.pending === "start" ||
    environment?.status === "starting"
  if (stopping || starting || (!environment && !message))
    return (
      <LoadingState
        title={
          stopping ? "Stopping your workspace…" : "Starting your workspace…"
        }
      />
    )
  const stopped = environment?.status === "stopped"
  return (
    <section
      className="environment-state"
      aria-labelledby="environment-state-title"
    >
      <h1 id="environment-state-title">
        {stopped ? "Workspace stopped" : "Workspace unavailable"}
      </h1>
      {message && <p role="status">{message}</p>}
      <Button
        variant="primary"
        disabled={state.pending !== null || state.paused}
        onClick={() => void (stopped ? controller.start() : controller.retry())}
      >
        {stopped ? "Return to workspace" : "Try again"}
      </Button>
      {stopped && <Recovery />}
    </section>
  )
}
