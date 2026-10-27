import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { App } from "./App"
import "./styles.css"
import { operator } from "./model"
operator.connect()

const root = document.getElementById("root")
if (!root) throw new Error("Factory renderer root is missing")
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>
)
