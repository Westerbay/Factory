import i18n from "i18next"
import frMessages from "./fr.json"
import { initReactI18next } from "react-i18next"

import { defaultLanguage, resolveLanguage, type Language } from "./language"

export type { Language } from "./language"
const storageKey = "factory.language"

function storedLanguage(): string | null {
  try {
    return localStorage.getItem(storageKey)
  } catch {
    return null
  }
}

export function setLanguage(language: Language) {
  try {
    localStorage.setItem(storageKey, language)
  } catch {
    /* The current session can still change language. */
  }
  void i18n.changeLanguage(language)
}

function browserLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return []
  return navigator.languages?.length
    ? navigator.languages
    : [navigator.language]
}

void i18n.use(initReactI18next).init({
  lng: resolveLanguage(storedLanguage(), browserLanguages()),
  fallbackLng: defaultLanguage,
  keySeparator: false,
  nsSeparator: false,
  interpolation: { escapeValue: false },
  resources: {
    en: { translation: {} },
    fr: {
      translation: frMessages,
    },
  },
})

function updateDocumentLanguage(language: string) {
  if (typeof document !== "undefined") document.documentElement.lang = language
}
updateDocumentLanguage(i18n.language ?? defaultLanguage)
i18n.on("languageChanged", updateDocumentLanguage)

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    try {
      if (event.storageArea !== localStorage) return
    } catch {
      return
    }
    if (event.key === storageKey || event.key === null)
      void i18n.changeLanguage(
        resolveLanguage(event.newValue, browserLanguages())
      )
  })
}

export default i18n
