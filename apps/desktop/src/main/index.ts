import { app } from "electron"
import { createDesktop } from "./desktop.js"
if (!app.requestSingleInstanceLock()) app.quit()
else {
  app
    .whenReady()
    .then(() => createDesktop())
    .catch(() => {
      console.error("Factory desktop startup failed")
      app.exit(1)
    })
  app.on("second-instance", () => {
    const window = requireWindow()
    window?.show()
    window?.focus()
  })
  app.on("window-all-closed", () => app.quit())
}
import { BrowserWindow } from "electron"
function requireWindow() {
  return BrowserWindow.getAllWindows()[0]
}
