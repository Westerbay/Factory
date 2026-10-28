const DEFAULT_API_ORIGIN = "http://localhost:3001"

/** This local increment permits only a loopback API with no URL credentials. */
export function parseApiOrigin(value = DEFAULT_API_ORIGIN): string {
  let url: URL
  try {
    url = new URL(value)
  } catch {
    throw new Error("VITE_API_BASE_URL must be a local HTTP origin")
  }
  if (
    url.protocol !== "http:" ||
    !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname) ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  ) {
    throw new Error("VITE_API_BASE_URL must be a local HTTP origin")
  }
  return url.origin
}
