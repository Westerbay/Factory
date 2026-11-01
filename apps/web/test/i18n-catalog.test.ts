import { readdirSync, readFileSync } from "node:fs"
import { join } from "node:path"
import ts from "typescript"
import { describe, expect, it } from "vitest"

import french from "../src/lib/i18n/fr.json"

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name)
    return entry.isDirectory()
      ? sourceFiles(path)
      : /\.tsx?$/.test(entry.name) && !entry.name.endsWith(".d.ts")
        ? [path]
        : []
  })
}

describe("French message catalog", () => {
  it("contains every literal message requested by the interface", () => {
    const missing: string[] = []
    const catalog: Record<string, string> = french
    for (const path of sourceFiles(join(process.cwd(), "src"))) {
      const source = ts.createSourceFile(
        path,
        readFileSync(path, "utf8"),
        ts.ScriptTarget.Latest,
        true,
        path.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS
      )
      function visit(node: ts.Node) {
        if (
          ts.isCallExpression(node) &&
          ((ts.isIdentifier(node.expression) &&
            ["t", "translate"].includes(node.expression.text)) ||
            (ts.isPropertyAccessExpression(node.expression) &&
              ts.isIdentifier(node.expression.expression) &&
              node.expression.expression.text === "i18n" &&
              node.expression.name.text === "t")) &&
          node.arguments[0] &&
          ts.isStringLiteral(node.arguments[0])
        ) {
          const key = node.arguments[0].text
          if (!(key in catalog) && !(key + "_one" in catalog))
            missing.push(`${path}: ${key}`)
        }
        ts.forEachChild(node, visit)
      }
      visit(source)
    }
    expect(missing).toEqual([])
  })
})
