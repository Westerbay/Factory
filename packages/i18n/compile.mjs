import { compile } from "@inlang/paraglide-js"

await compile({
  project: "./project.inlang",
  outdir: "./src/generated",
  strategy: ["globalVariable", "baseLocale"],
  emitTsDeclarations: true,
  watch: process.argv.includes("--watch"),
})
