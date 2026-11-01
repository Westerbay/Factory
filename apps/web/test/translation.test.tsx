import { renderToStaticMarkup } from "react-dom/server"
import { afterEach, describe, expect, it, vi } from "vitest"

import { setLanguage, translateMessage } from "../src/lib/i18n"
import { SignInView } from "../src/features/auth/sign-in-view"
import { conversationLabel } from "../src/features/conversations/conversation-navigation"

vi.mock("../src/components/ui/brand", () => ({ Brand: () => null }))

afterEach(async () => {
  setLanguage("en")
})

describe("French interface", () => {
  it("looks up complete messages containing punctuation", async () => {
    setLanguage("fr")
    expect(
      translateMessage(
        "Recovery is blocked: shutdown or a model launch is still unconfirmed. Check recovery status or contact your administrator."
      )
    ).toMatch(/^La récupération est bloquée/)
    expect(translateMessage("Task recovered.")).toBe("Tâche récupérée.")
  })

  it("formats conversation dates in the selected language, including English", async () => {
    const conversation = {
      id: "chat",
      createdAt: "2026-10-31T10:00:00Z",
      projectId: null,
    }
    setLanguage("en")
    expect(conversationLabel(conversation)).toMatch(/Chat · Oct 31/)
    setLanguage("fr")
    expect(conversationLabel(conversation)).toMatch(/Conversation · 31 oct\./)
  })

  it("renders the sign-in journey in the selected language", async () => {
    setLanguage("fr")
    const html = renderToStaticMarkup(<SignInView />)
    expect(html).toContain("Se connecter à Factory")
    expect(html).toContain("Continuer avec GitHub")
    setLanguage("en")
    expect(renderToStaticMarkup(<SignInView />)).toContain("Sign in to Factory")
  })
})
