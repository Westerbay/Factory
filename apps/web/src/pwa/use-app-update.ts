import { useEffect, useRef, useState } from "react"

const UPDATE_INTERVAL_MS = 60 * 60 * 1000

export function useAppUpdate() {
  const registration = useRef<ServiceWorkerRegistration | undefined>(undefined)
  const reloadApproved = useRef(false)
  const [available, setAvailable] = useState(false)
  const [error, setError] = useState(false)

  useEffect(() => {
    if (!import.meta.env.PROD || !("serviceWorker" in navigator)) return
    let disposed = false
    let checking = false
    let interval: ReturnType<typeof setInterval> | undefined
    let installing: ServiceWorker | null = null
    let hadController = Boolean(navigator.serviceWorker.controller)

    function onControllerChange() {
      if (reloadApproved.current) window.location.reload()
      else if (hadController) setAvailable(true)
      hadController = true
    }

    function onInstalled() {
      if (
        installing?.state === "installed" &&
        registration.current?.waiting &&
        registration.current.active
      ) {
        setAvailable(true)
      }
    }

    function onUpdateFound() {
      installing?.removeEventListener("statechange", onInstalled)
      installing = registration.current?.installing ?? null
      installing?.addEventListener("statechange", onInstalled)
    }

    async function checkForUpdate() {
      if (!navigator.onLine || checking || registration.current?.installing)
        return
      checking = true
      try {
        await registration.current?.update()
        if (!disposed) setError(false)
      } catch {
        if (!disposed) setError(true)
      } finally {
        checking = false
      }
    }

    navigator.serviceWorker.addEventListener(
      "controllerchange",
      onControllerChange
    )
    navigator.serviceWorker
      .register("/sw.js", { scope: "/", updateViaCache: "none" })
      .then((value) => {
        if (disposed) return
        registration.current = value
        setAvailable(Boolean(value.waiting && value.active))
        value.addEventListener("updatefound", onUpdateFound)
        onUpdateFound()
        window.addEventListener("focus", checkForUpdate)
        window.addEventListener("online", checkForUpdate)
        interval = setInterval(checkForUpdate, UPDATE_INTERVAL_MS)
      })
      .catch(() => {
        if (!disposed) setError(true)
      })

    return () => {
      disposed = true
      clearInterval(interval)
      window.removeEventListener("focus", checkForUpdate)
      window.removeEventListener("online", checkForUpdate)
      navigator.serviceWorker.removeEventListener(
        "controllerchange",
        onControllerChange
      )
      registration.current?.removeEventListener("updatefound", onUpdateFound)
      installing?.removeEventListener("statechange", onInstalled)
    }
  }, [])

  function applyUpdate() {
    reloadApproved.current = true
    const waiting = registration.current?.waiting
    if (waiting) waiting.postMessage({ type: "SKIP_WAITING" })
    else window.location.reload()
  }

  return { available, error, applyUpdate }
}
