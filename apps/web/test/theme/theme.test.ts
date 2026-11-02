import { afterEach, expect, it, vi } from "vitest"

afterEach(() => {
  vi.unstubAllGlobals()
  vi.resetModules()
})

it.each([
  [null, true, "dark"],
  [null, false, "light"],
  ["light", true, "light"],
  ["invalid", false, "light"],
] as const)(
  "selects the saved theme or the initial browser preference",
  async (saved, prefersDark, expected) => {
    const setItem = vi.fn()
    const root = { dataset: { theme: "" }, style: { colorScheme: "" } }
    vi.stubGlobal("localStorage", { getItem: () => saved, setItem })
    vi.stubGlobal("document", { documentElement: root })
    vi.stubGlobal("window", {
      matchMedia: () => ({ matches: prefersDark }),
      addEventListener: vi.fn(),
    })
    const { setTheme } = await import("../../src/lib/theme/theme")
    expect(root.dataset.theme).toBe(expected)
    expect(setItem).toHaveBeenCalledWith("factory-theme", expected)
    setTheme("dark")
    expect(root.style.colorScheme).toBe("dark")
    expect(setItem).toHaveBeenLastCalledWith("factory-theme", "dark")
  }
)
