import { useSyncExternalStore } from "react"

type Theme = "light" | "dark"
const storageKey = "factory-theme"
const listeners = new Set<() => void>()

function readTheme(): Theme {
  try {
    const saved = localStorage.getItem(storageKey)
    if (saved === "light" || saved === "dark") return saved
  } catch {
    // The preference still works for this tab when storage is unavailable.
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light"
}

let theme = readTheme()
function applyTheme(next: Theme) {
  theme = next
  document.documentElement.dataset.theme = next
  document.documentElement.style.colorScheme = next
  for (const listener of listeners) listener()
}

export function setTheme(next: Theme) {
  try {
    localStorage.setItem(storageKey, next)
  } catch {
    // Keep the current tab usable without storage.
  }
  applyTheme(next)
}

export function subscribeTheme(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useTheme() {
  return useSyncExternalStore(subscribeTheme, () => theme)
}

setTheme(theme)
window.addEventListener("storage", (event) => {
  if (
    event.key === storageKey &&
    (event.newValue === "light" || event.newValue === "dark")
  )
    applyTheme(event.newValue)
})
