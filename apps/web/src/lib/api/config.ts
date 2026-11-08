const DEFAULT_API_ORIGIN = "http://localhost:3001"

/** A slash targets the web page's origin. Explicit API origins use HTTPS or loopback HTTP. */
export function parseApiOrigin(value = DEFAULT_API_ORIGIN): string {
  if (value === "/") return value
  let url: URL
  try {
    url = new URL(value)
  } catch {
    throw new Error(
      "VITE_API_BASE_URL must be /, an HTTPS origin, or a loopback HTTP origin"
    )
  }
  const localHttp =
    url.protocol === "http:" &&
    ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)
  if (
    (!localHttp && url.protocol !== "https:") ||
    url.port === "0" ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  ) {
    throw new Error(
      "VITE_API_BASE_URL must be /, an HTTPS origin, or a loopback HTTP origin"
    )
  }
  return url.origin
}
