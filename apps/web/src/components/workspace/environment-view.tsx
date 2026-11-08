import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
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
  controller: Pick<EnvironmentController, "start" | "retry">
}) {
  useLocale()
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
          stopping ? m.stopping_your_workspace() : m.starting_your_workspace()
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
        {stopped ? m.workspace_stopped() : m.workspace_unavailable()}
      </h1>
      {message && <p role="status">{translateMessage(message)}</p>}
      <Button
        variant="primary"
        disabled={
          state.paused || (state.pending !== null && state.pending !== "read")
        }
        onClick={() => void (stopped ? controller.start() : controller.retry())}
      >
        {stopped ? m.return_to_workspace() : m.try_again()}
      </Button>
      {stopped && <Recovery />}
    </section>
  )
}
