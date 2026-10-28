import { createRoute } from "@tanstack/react-router"
import { useQuery } from "@tanstack/react-query"

import { readLiveness } from "../lib/api/client"
import { rootRoute } from "./__root"

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: FoundationPage,
})

function FoundationPage() {
  const health = useQuery({
    queryKey: ["health", "live"],
    queryFn: ({ signal }) => readLiveness(signal),
  })
  let connection = "Checking the service…"
  if (health.isError) connection = "Service unavailable"
  else if (health.isSuccess) connection = "Service reachable"

  return (
    <section className="foundation" aria-labelledby="page-title">
      <p className="eyebrow">Welcome to Factory</p>
      <h1 id="page-title">Your workspace is being prepared.</h1>
      <p className="introduction">
        Sign-in and workspaces are not available in this preview. No account or
        project data is loaded.
      </p>
      <div className="connection-panel">
        <div>
          <h2>Connection</h2>
          <p role="status" aria-live="polite">
            {connection}
          </p>
        </div>
        <button
          type="button"
          disabled={health.isFetching}
          onClick={() => void health.refetch()}
        >
          {health.isFetching ? "Checking…" : "Check again"}
        </button>
      </div>
      {health.isError && (
        <p className="connection-help">
          The service could not be reached. Check that it is running, then try
          again.
        </p>
      )}
    </section>
  )
}
