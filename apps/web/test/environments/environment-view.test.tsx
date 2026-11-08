import { renderToStaticMarkup } from "react-dom/server"
import { expect, it, vi } from "vitest"
import { EnvironmentView } from "../../src/components/workspace/environment-view"

it("keeps retry enabled while polling an unavailable workspace", () => {
  const controller = { start: vi.fn(), retry: vi.fn() }
  const html = renderToStaticMarkup(
    <EnvironmentView
      state={{
        environment: { status: "unavailable", errorCode: "policy_mismatch" },
        pending: "read",
        message: null,
        paused: false,
      }}
      controller={controller}
    />
  )
  expect(html).toContain("Workspace unavailable")
  expect(html).toContain(
    "The environment configuration needs administrator attention."
  )
  expect(html).toMatch(/<button(?![^>]*\bdisabled\b)[^>]*>Try again<\/button>/)
})
