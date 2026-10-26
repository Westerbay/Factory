import { access, readdir, readFile } from "node:fs/promises"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = fileURLToPath(new URL("../", import.meta.url))
const documents = [
  "README.md",
  "AGENTS.md",
  ...(await readdir(resolve(root, "docs")))
    .filter((file) => file.endsWith(".md"))
    .sort()
    .map((file) => `docs/${file}`),
]

const failures = []
for (const document of documents) {
  const absolutePath = resolve(root, document)
  let content
  try {
    content = await readFile(absolutePath, "utf8")
  } catch {
    failures.push(`missing document: ${document}`)
    continue
  }

  const links = content.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)
  for (const match of links) {
    const target = match[1]
    if (!target || /^(?:https?:|mailto:|#)/.test(target)) continue
    const path = decodeURIComponent(target.split("#", 1)[0])
    try {
      await access(resolve(dirname(absolutePath), path))
    } catch {
      failures.push(`broken link in ${document}: ${target}`)
    }
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"))
  process.exitCode = 1
} else {
  console.log(
    `Documentation checked: ${documents.length} files, local links valid`
  )
}
