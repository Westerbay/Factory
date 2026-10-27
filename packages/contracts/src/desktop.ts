import { z } from "zod"
import {
  desktopCommandSchema,
  desktopEventSchema,
  desktopResultSchema,
} from "./index.js"
const id = z.string().regex(/^[A-Za-z0-9_-]{1,128}$/)
export const workspaceSelectionSchema = z.strictObject({
  id,
  name: z.string().min(1).max(4096),
})
export const hostStatusSchema = z.strictObject({
  state: z.enum(["starting", "ready", "unavailable", "closing", "closed"]),
  message: z.string().min(1).max(4096),
})
export const bridgeEventSchema = z.union([
  desktopEventSchema,
  z.strictObject({ type: z.literal("host-status"), status: hostStatusSchema }),
])
export const workspaceResultSchema = z.union([
  z.strictObject({
    type: z.literal("workspace"),
    workspace: workspaceSelectionSchema,
  }),
  z.strictObject({ type: z.literal("cancelled") }),
  z.strictObject({
    type: z.literal("error"),
    message: z.string().min(1).max(4096),
  }),
])
export interface DesktopBridge {
  command(
    command: import("./index.js").DesktopCommand
  ): Promise<import("./index.js").DesktopResult>
  selectWorkspace(): Promise<WorkspaceResult>
  status(): Promise<HostStatus>
  subscribe(listener: (event: BridgeEvent) => void): () => void
}
export const workerRequestSchema = z.discriminatedUnion("type", [
  z.strictObject({
    type: z.literal("command"),
    generation: id,
    requestId: id,
    command: desktopCommandSchema,
  }),
  z.strictObject({
    type: z.literal("workspace"),
    generation: id,
    requestId: id,
    workspace: workspaceSelectionSchema,
    path: z.string().min(1).max(4096),
  }),
  z.strictObject({ type: z.literal("close"), generation: id, requestId: id }),
])
export const workerResponseSchema = z.discriminatedUnion("type", [
  z.strictObject({
    type: z.literal("result"),
    generation: id,
    requestId: id,
    result: desktopResultSchema,
  }),
  z.strictObject({
    type: z.literal("event"),
    generation: id,
    event: desktopEventSchema,
  }),
  z.strictObject({
    type: z.literal("status"),
    generation: id,
    status: hostStatusSchema,
  }),
])
export const bridgeChannels = {
  command: "factory:command",
  workspace: "factory:workspace",
  status: "factory:status",
  event: "factory:event",
} as const
export type WorkspaceSelection = z.infer<typeof workspaceSelectionSchema>
export type WorkspaceResult = z.infer<typeof workspaceResultSchema>
export type HostStatus = z.infer<typeof hostStatusSchema>
export type BridgeEvent = z.infer<typeof bridgeEventSchema>
export type WorkerRequest = z.infer<typeof workerRequestSchema>
export type WorkerResponse = z.infer<typeof workerResponseSchema>
