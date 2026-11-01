export const supportedLanguages = ["en", "fr"] as const
export type Language = (typeof supportedLanguages)[number]
export const defaultLanguage: Language = "en"

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
