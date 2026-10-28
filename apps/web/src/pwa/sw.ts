/// <reference lib="webworker" />
import {
  cleanupOutdatedCaches,
  matchPrecache,
  precacheAndRoute,
  type PrecacheEntry,
} from "workbox-precaching"
import { registerRoute } from "workbox-routing"

declare const self: ServiceWorkerGlobalScope & {
  __WB_MANIFEST: PrecacheEntry[]
}

// Navigations must never restore a previously authenticated document offline.
registerRoute(
  ({ request, url }) =>
    request.mode === "navigate" &&
    url.origin === self.location.origin &&
    !/^\/(?:api|health)(?:\/|$)/.test(url.pathname),
  async ({ request }) => {
    try {
      return await fetch(request, { cache: "no-store" })
    } catch {
      return (await matchPrecache("/offline.html")) ?? Response.error()
    }
  }
)

// Vite PWA adds the web manifest independently of globPatterns; it must revalidate.
precacheAndRoute(
  self.__WB_MANIFEST.filter(
    (entry) =>
      (typeof entry === "string" ? entry : entry.url) !== "manifest.webmanifest"
  ),
  { ignoreURLParametersMatching: [] }
)
cleanupOutdatedCaches()

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim())
})

self.addEventListener("message", (event) => {
  if (event.data?.type === "SKIP_WAITING") {
    event.waitUntil(self.skipWaiting())
  }
})
