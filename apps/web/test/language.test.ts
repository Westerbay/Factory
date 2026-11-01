import { describe, expect, it } from "vitest"
import { resolveLanguage } from "../src/lib/i18n/language"

describe("language selection", () => {
  it("uses a saved preference before browser languages", () => {
    expect(resolveLanguage("en", ["fr-FR"])).toBe("en")
    expect(resolveLanguage("fr", ["en-US"])).toBe("fr")
  })
  it("matches the first supported browser language, including regional variants", () => {
    expect(resolveLanguage(null, ["de-DE", "fr-CA", "en-US"])).toBe("fr")
    expect(resolveLanguage(null, ["EN-gb", "fr"])).toBe("en")
  })
  it("falls back to English for unsupported or missing preferences", () => {
    expect(resolveLanguage("es", ["de-DE"])).toBe("en")
    expect(resolveLanguage(null, [])).toBe("en")
  })
})
