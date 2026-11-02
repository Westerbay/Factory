import type { Model } from "./api"

/** Keep provider identifiers as option values, but show a readable model name. */
function modelName(model: Model): string {
  const name = model.name.replace(/\s*\(recommended\)/gi, "").trim()
  if (!model.id.startsWith("factory:claude:")) return name.replace(" · ", " - ")

  const resolved = model.resolvedModel?.match(
    /^claude-([a-z][a-z0-9]*)-(\d+)(?:-(\d+))?(?:-\d{8})?(\[1m\])?$/i
  )
  if (!resolved) return name.replace(" · ", " - ")

  const [, family, major, minor, context] = resolved
  if (!family || !major) return name.replace(" · ", " - ")
  const version = minor ? `${major}.${minor}` : major
  const provider = name.split(" · ")[0]
  const label = `${family[0]!.toUpperCase()}${family.slice(1)} ${version}`
  return `${provider} - ${label}${context ? " (1M context)" : ""}`
}

/** Omit a redundant default alias only when the CLI advertises an equivalent option. */
export function presentModels(models: Model[]): Model[] {
  return models
    .filter((model) => {
      if (model.id !== "factory:claude:default" || !model.resolvedModel)
        return true
      return !models.some(
        (other) =>
          other.id.startsWith("factory:claude:") &&
          other.id !== model.id &&
          other.resolvedModel === model.resolvedModel &&
          other.defaultEffort === model.defaultEffort &&
          other.effortLevels.length === model.effortLevels.length &&
          model.effortLevels.every((level) =>
            other.effortLevels.includes(level)
          )
      )
    })
    .map((model) => ({ ...model, name: modelName(model) }))
}
