import js from "@eslint/js"
import globals from "globals"
import tseslint from "typescript-eslint"

export default tseslint.config(
  {
    ignores: [
      "**/dist/**",
      "**/out/**",
      "**/node_modules/**",
      "docs/plans/**",
      ".factory/**",
      "target/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  { languageOptions: { globals: globals.node } },
  {
    files: [
      "apps/desktop/src/renderer/**/*.{ts,tsx}",
      "apps/web/src/**/*.{ts,tsx}",
    ],
    languageOptions: { globals: globals.browser },
  }
)
