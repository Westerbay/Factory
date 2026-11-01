import { useTranslation } from "react-i18next"
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
  const { t: translate } = useTranslation()
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
          stopping
            ? translate("Stopping your workspace…")
            : translate("Starting your workspace…")
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
        {stopped
          ? translate("Workspace stopped")
          : translate("Workspace unavailable")}
      </h1>
      {message && <p role="status">{translate(message)}</p>}
      <Button
        variant="primary"
        disabled={state.pending !== null || state.paused}
        onClick={() => void (stopped ? controller.start() : controller.retry())}
      >
        {stopped ? translate("Return to workspace") : translate("Try again")}
      </Button>
      {stopped && <Recovery />}
    </section>
  )
}
