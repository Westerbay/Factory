import { afterEach, describe, expect, it, vi } from "vitest"

afterEach(() => {
  vi.unstubAllGlobals()
  vi.resetModules()
})

describe("language preference across tabs", () => {
  it("updates the current tab and returns to browser preference when cleared", async () => {
    const otherTab = new EventTarget()
    vi.stubGlobal("window", otherTab)
    vi.stubGlobal("navigator", { language: "fr-FR", languages: ["fr-FR"] })
    const storage = { getItem: () => "en", setItem: vi.fn() }
    vi.stubGlobal("localStorage", storage)
    const { getLanguage } = await import("../src/lib/i18n")
    expect(getLanguage()).toBe("en")

    function notify(
      key: string | null,
      newValue: string | null,
      storageArea: object = storage
    ) {
      const event = new Event("storage")
      Object.defineProperties(event, {
        key: { value: key },
        newValue: { value: newValue },
        storageArea: { value: storageArea },
      })
      otherTab.dispatchEvent(event)
    }
    notify("factory.language", "fr")
    await vi.waitFor(() => expect(getLanguage()).toBe("fr"))
    notify("another-key", "en")
    notify("factory.language", "en", {})
    expect(getLanguage()).toBe("fr")
    notify("factory.language", "en")
    await vi.waitFor(() => expect(getLanguage()).toBe("en"))
    notify(null, null)
    await vi.waitFor(() => expect(getLanguage()).toBe("fr"))
  })
})
