import { contextBridge, ipcRenderer } from "electron"
import { desktopCommandSchema, desktopResultSchema } from "@workspace/contracts"
import {
  bridgeChannels,
  bridgeEventSchema,
  workspaceResultSchema,
  hostStatusSchema,
  type DesktopBridge,
} from "@workspace/contracts/desktop"
const bridge: DesktopBridge = {
  command: async (input) => {
    const command = desktopCommandSchema.parse(input)
    return desktopResultSchema.parse(
      await ipcRenderer.invoke(bridgeChannels.command, command)
    )
  },
  selectWorkspace: async () =>
    workspaceResultSchema.parse(
      await ipcRenderer.invoke(bridgeChannels.workspace)
    ),
  status: async () =>
    hostStatusSchema.parse(await ipcRenderer.invoke(bridgeChannels.status)),
  subscribe: (listener) => {
    if (typeof listener !== "function")
      throw new Error("A desktop event listener is required")
    const receive = (_event: Electron.IpcRendererEvent, input: unknown) => {
      const parsed = bridgeEventSchema.safeParse(input)
      if (parsed.success) listener(parsed.data)
    }
    ipcRenderer.on(bridgeChannels.event, receive)
    return () => ipcRenderer.removeListener(bridgeChannels.event, receive)
  },
}
contextBridge.exposeInMainWorld("factory", bridge)
