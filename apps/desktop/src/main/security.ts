import type { IpcMainInvokeEvent, BrowserWindow } from "electron"
/** Exact top-frame identity and document URL, never a prefix or sender-controlled origin. */
export function isTrustedSender(
  event: IpcMainInvokeEvent,
  window: BrowserWindow,
  expectedUrl: string
) {
  return (
    !window.isDestroyed() &&
    event.sender === window.webContents &&
    event.senderFrame === window.webContents.mainFrame &&
    event.senderFrame.url === expectedUrl
  )
}
export function rendererAddress(
  developmentUrl: string | undefined,
  packaged: boolean,
  fileUrl: string
): string {
  if (packaged || !developmentUrl) return fileUrl
  const url = new URL(developmentUrl)
  if (
    url.protocol !== "http:" ||
    url.hostname !== "127.0.0.1" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.pathname !== "/"
  )
    throw new Error("The development renderer must use exact loopback HTTP")
  return url.href
}
