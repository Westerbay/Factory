import { useSyncExternalStore } from "react"
import { getLocale, setLocale } from "@workspace/i18n/runtime"

import { defaultLanguage, resolveLanguage, type Language } from "./language"

export type { Language } from "./language"
export { translateMessage } from "./dynamic-messages"
const storageKey = "factory.language"
const listeners = new Set<() => void>()

function storedLanguage(): string | null {
  try {
    return localStorage.getItem(storageKey)
  } catch {
    return null
  }
}

function browserLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return []
  return navigator.languages?.length
    ? navigator.languages
    : [navigator.language]
}

function applyLanguage(language: Language) {
  void setLocale(language, { reload: false })
  if (typeof document !== "undefined") document.documentElement.lang = language
  for (const listener of listeners) listener()
}

export function setLanguage(language: Language) {
  try {
    localStorage.setItem(storageKey, language)
  } catch {
    // Keep the current tab usable without storage.
  }
  applyLanguage(language)
}

export function getLanguage(): Language {
  return getLocale()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useLocale(): Language {
  return useSyncExternalStore(subscribe, getLanguage, () => defaultLanguage)
}

applyLanguage(resolveLanguage(storedLanguage(), browserLanguages()))

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    try {
      if (event.storageArea !== localStorage) return
    } catch {
      return
    }
    if (event.key === storageKey || event.key === null)
      applyLanguage(resolveLanguage(event.newValue, browserLanguages()))
  })
}
