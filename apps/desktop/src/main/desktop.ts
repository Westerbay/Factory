import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  session,
  utilityProcess,
} from "electron"
import { join, basename } from "node:path"
import { realpath, stat } from "node:fs/promises"
import { pathToFileURL } from "node:url"
import { randomUUID } from "node:crypto"
import { bridgeChannels, type BridgeEvent } from "@workspace/contracts/desktop"
import { desktopCommandSchema } from "@workspace/contracts"
import { RuntimeClient } from "./runtime-client.js"
import { isTrustedSender, rendererAddress } from "./security.js"
class DesktopShutdownError extends Error {}
export interface DesktopOptions {
  workerPath?: string
  workerArguments?: string[]
  chooseWorkspace?: () => Promise<string | undefined>
  show?: boolean
}
export async function createDesktop(options: DesktopOptions = {}) {
  const window = new BrowserWindow({
    title: "Factory",
    width: 1180,
    height: 850,
    minWidth: 400,
    minHeight: 520,
    show: options.show ?? true,
    webPreferences: {
      preload: join(import.meta.dirname, "../preload/index.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webviewTag: false,
    },
  })
  const url = rendererAddress(
    process.env.ELECTRON_RENDERER_URL,
    app.isPackaged,
    pathToFileURL(join(import.meta.dirname, "../renderer/index.html")).href
  )
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }))
  window.webContents.on("will-navigate", (event) => event.preventDefault())
  window.webContents.on("will-redirect", (event) => event.preventDefault())
  window.webContents.on("will-attach-webview", (event) =>
    event.preventDefault()
  )
  session.defaultSession.setPermissionRequestHandler(
    (_contents, _permission, callback) => callback(false)
  )
  session.defaultSession.setPermissionCheckHandler(() => false)
  // Packaged pages need no network. Development permits only the exact Vite origin.
  session.defaultSession.webRequest.onBeforeRequest((details, callback) => {
    const requested = new URL(details.url)
    const document = new URL(url)
    const sameDevelopmentEndpoint =
      document.protocol === "http:" &&
      requested.hostname === document.hostname &&
      requested.port === document.port &&
      ["http:", "ws:"].includes(requested.protocol)
    const allowed =
      requested.protocol === "file:" ||
      (!app.isPackaged && sameDevelopmentEndpoint)
    callback({ cancel: !allowed })
  })
  const publish = (event: BridgeEvent) => {
    if (!window.isDestroyed())
      window.webContents.send(bridgeChannels.event, event)
  }
  const client = new RuntimeClient(publish)
  const child = utilityProcess.fork(
    options.workerPath ?? join(import.meta.dirname, "worker.js"),
    [client.generation, ...(options.workerArguments ?? [])],
    { stdio: "ignore", serviceName: "Factory execution host" }
  )
  client.attach(child)
  let selectionPending = false
  function requireTrusted(event: import("electron").IpcMainInvokeEvent) {
    if (!isTrustedSender(event, window, url))
      throw new Error("Untrusted Factory IPC source")
  }
  ipcMain.handle(bridgeChannels.command, (event, input: unknown) => {
    requireTrusted(event)
    const parsed = desktopCommandSchema.safeParse(input)
    if (!parsed.success)
      return {
        type: "error",
        code: "invalid-answer",
        message: "Invalid desktop command.",
      }
    return client.command(parsed.data)
  })
  ipcMain.handle(bridgeChannels.status, (event) => {
    requireTrusted(event)
    return client.status()
  })
  ipcMain.handle(bridgeChannels.workspace, async (event) => {
    requireTrusted(event)
    if (selectionPending)
      return { type: "error", message: "A workspace dialog is already open." }
    selectionPending = true
    try {
      let path: string | undefined
      if (options.chooseWorkspace) path = await options.chooseWorkspace()
      else {
        const result = await dialog.showOpenDialog(window, {
          title: "Select agent workspace",
          properties: ["openDirectory"],
        })
        path = result.canceled ? undefined : result.filePaths[0]
      }
      if (!path) return { type: "cancelled" }
      const canonical = await realpath(path)
      if (!(await stat(canonical)).isDirectory())
        return { type: "error", message: "Select an existing directory." }
      const workspace = {
        id: randomUUID(),
        name: basename(canonical) || canonical,
      }
      const result = await client.registerWorkspace(workspace, canonical)
      if (result.type === "error")
        return { type: "error", message: result.message }
      return { type: "workspace", workspace }
    } catch {
      return {
        type: "error",
        message:
          "The selected workspace is unavailable. Choose another directory.",
      }
    } finally {
      selectionPending = false
    }
  })
  let closed = false
  let closePending = false
  async function shutdown() {
    const result = await client.close()
    if (result.type === "error") throw new DesktopShutdownError(result.message)
    closed = true
    child.kill() // Only after provider cleanup and pool close were acknowledged.
    window.destroy()
    for (const channel of [
      bridgeChannels.command,
      bridgeChannels.workspace,
      bridgeChannels.status,
    ])
      ipcMain.removeHandler(channel)
  }
  window.on("close", (event) => {
    if (closed) return
    event.preventDefault()
    if (closePending) return
    closePending = true
    void closeRequested().finally(() => {
      closePending = false
    })
  })
  async function closeRequested() {
    const choice = await dialog.showMessageBox(window, {
      type: "question",
      title: "Close Factory",
      message: "Stop owned agents and close Factory?",
      detail:
        "Closing waits for provider cleanup and the database pool. Pending questions will be invalidated.",
      buttons: ["Keep open", "Stop and close"],
      defaultId: 0,
      cancelId: 0,
      noLink: true,
    })
    if (choice.response !== 1) return
    try {
      await shutdown()
    } catch (error) {
      const detail =
        error instanceof DesktopShutdownError
          ? error.message
          : "The window remains open. Check local provider resources before quitting externally. Persisted work will be recovered as interrupted on the next start."
      await dialog.showMessageBox(window, {
        type: "error",
        title: "Shutdown unconfirmed",
        message: "Factory could not complete confirmed shutdown.",
        detail,
        buttons: ["Keep open"],
      })
    }
  }
  app.on("before-quit", (event) => {
    if (!closed) {
      event.preventDefault()
      window.close()
    }
  })
  window.webContents.on("render-process-gone", () => {
    void shutdown().catch(() =>
      client.fail(
        "Renderer exited and cleanup failed. Owned provider cleanup is unconfirmed."
      )
    )
  })
  await window.loadURL(url)
  return { window, client, shutdown, process: child }
}
