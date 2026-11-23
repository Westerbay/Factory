import { renderToStaticMarkup } from "react-dom/server"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { expect, it, vi } from "vitest"

vi.mock("../../src/lib/theme/theme", () => ({ useTheme: () => "dark" }))

// The application is client-rendered. Read its external stores synchronously in
// this server-rendered harness while retaining the real refs and memo callbacks.
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) =>
    snapshot(),
}))

vi.mock("../../src/lib/auth/session-context", () => ({
  useSession: () => ({
    state: {
      status: "active",
      session: {
        csrfToken: "fixture-csrf",
        user: { id: "fixture-member", role: "member" },
      },
    },
    controller: {},
  }),
}))

import { Chat } from "../../src/features/conversations/chat"
import { useProjects } from "../../src/features/projects/use-projects"

function SelectedChat() {
  const projects = useProjects(true)
  return (
    <Chat
      available
      projectWorkspace={projects}
      connectedProviders={[]}
      workflow={{
        conversation: {
          id: "714ab526-5719-4fb9-a094-feb256b57d21",
          projectId: null,
          createdAt: "2026-11-18T00:00:00Z",
        },
        version: "fixture-version",
        modelLabel: "Fixture model",
        canSend: true,
        start: async () => {},
        onRefresh: () => {},
      }}
      renderLayout={({ conversation }) => conversation}
    />
  )
}

it("renders the attachment composer when a workflow conversation is already selected", () => {
  const client = new QueryClient()
  try {
    const html = renderToStaticMarkup(
      <QueryClientProvider client={client}>
        <SelectedChat />
      </QueryClientProvider>
    )
    expect(html).toContain("Assistant conversation")
    expect(html).toContain('type="file"')
  } finally {
    client.clear()
  }
})
