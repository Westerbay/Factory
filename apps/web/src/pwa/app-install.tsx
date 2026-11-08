import * as m from "@workspace/i18n/messages"
import { X } from "lucide-react"
import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react"

import { Button, IconButton } from "../components/ui/button"
import { useLocale } from "../lib/i18n"
import "./install.css"

interface InstallPromptEvent extends Event {
  prompt(): Promise<unknown>
}

const dismissalKey = "factory-install-dismissed"
const InstallContext = createContext<{
  install?: () => void
  offer: boolean
  dismiss: () => void
} | null>(null)

export function AppInstallProvider({ children }: { children: ReactNode }) {
  const prompt = useRef<InstallPromptEvent | null>(null)
  const [available, setAvailable] = useState(false)
  const [dismissed, setDismissed] = useState(() => {
    try {
      return sessionStorage.getItem(dismissalKey) === "1"
    } catch {
      return false
    }
  })

  useEffect(() => {
    if (window.matchMedia("(display-mode: standalone)").matches) return
    let installed = false
    function capture(event: Event) {
      event.preventDefault()
      if (installed) return
      prompt.current = event as InstallPromptEvent
      setAvailable(true)
    }
    function clear() {
      installed = true
      prompt.current = null
      setAvailable(false)
    }
    window.addEventListener("beforeinstallprompt", capture)
    window.addEventListener("appinstalled", clear)
    return () => {
      window.removeEventListener("beforeinstallprompt", capture)
      window.removeEventListener("appinstalled", clear)
    }
  }, [])

  function dismiss() {
    setDismissed(true)
    try {
      sessionStorage.setItem(dismissalKey, "1")
    } catch {
      // The current page still remembers the choice when storage is disabled.
    }
  }

  function install() {
    const event = prompt.current
    if (!event) return
    prompt.current = null
    setAvailable(false)
    dismiss()
    void event.prompt().catch(() => undefined)
  }

  return (
    <InstallContext.Provider
      value={{
        install: available ? install : undefined,
        offer: available && !dismissed,
        dismiss,
      }}
    >
      {children}
    </InstallContext.Provider>
  )
}

export function useAppInstall() {
  return useContext(InstallContext)
}

export function AppInstallOffer() {
  useLocale()
  const app = useAppInstall()
  if (!app?.offer) return null

  return (
    <aside className="app-install" aria-labelledby="install-title">
      <img src="/brand/factory.svg" alt="" width="40" height="40" />
      <div className="app-install-copy">
        <h2 id="install-title">{m.install_factory()}</h2>
        <p>{m.install_home_screen()}</p>
      </div>
      <IconButton label={m.dismiss_install_offer()} onClick={app.dismiss}>
        <X size={18} aria-hidden="true" />
      </IconButton>
      <Button className="app-install-action" onClick={app.install}>
        {m.install_app()}
      </Button>
    </aside>
  )
}
