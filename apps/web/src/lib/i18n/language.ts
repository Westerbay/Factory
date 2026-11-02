import { baseLocale, locales, type Locale } from "@workspace/i18n/runtime"

export const supportedLanguages = locales
export type Language = Locale
export const defaultLanguage: Language = baseLocale

export function resolveLanguage(
  preference: string | null,
  browserLanguages: readonly string[]
): Language {
  if (supportedLanguages.some((language) => language === preference))
    return preference as Language
  for (const candidate of browserLanguages) {
    const language = candidate.toLowerCase().split("-")[0]
    if (supportedLanguages.some((supported) => supported === language))
      return language as Language
  }
  return defaultLanguage
}
