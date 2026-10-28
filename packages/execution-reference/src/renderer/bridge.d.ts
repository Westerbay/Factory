import type { DesktopBridge } from "@workspace/contracts/desktop"
declare global {
  interface Window {
    factory: DesktopBridge
  }
}
export {}
