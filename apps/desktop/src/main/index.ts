import { app, BrowserWindow, session } from "electron"
import { join } from "node:path"

async function createWindow() {
  const window = new BrowserWindow({
    title: "Factory",
    width: 1000,
    height: 720,
    minWidth: 400,
    minHeight: 420,
    webPreferences: {
      preload: join(import.meta.dirname, "../preload/index.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webviewTag: false,
    },
  })
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }))
  window.webContents.on("will-navigate", (event) => event.preventDefault())
  window.webContents.on("will-redirect", (event) => event.preventDefault())
  const rendererUrl = process.env.ELECTRON_RENDERER_URL
  if (!app.isPackaged && rendererUrl) {
    const url = new URL(rendererUrl)
    if (url.protocol !== "http:" || url.hostname !== "127.0.0.1") {
      throw new Error("The development renderer must use loopback HTTP")
    }
    await window.loadURL(url.href)
  } else {
    await window.loadFile(join(import.meta.dirname, "../renderer/index.html"))
  }
}

if (!app.requestSingleInstanceLock()) {
  app.quit()
} else {
  app
    .whenReady()
    .then(async () => {
      session.defaultSession.setPermissionRequestHandler(
        (_contents, _permission, callback) => callback(false)
      )
      session.defaultSession.setPermissionCheckHandler(() => false)
      await createWindow()
      app.on("activate", () => {
        if (BrowserWindow.getAllWindows().length === 0)
          void createWindow().catch(failStartup)
      })
    })
    .catch(failStartup)
  app.on("window-all-closed", () => {
    if (process.platform !== "darwin") app.quit()
  })
}

function failStartup(error: unknown) {
  console.error("Factory could not start", error)
  app.exit(1)
}
