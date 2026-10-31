import { Link } from "@tanstack/react-router"

export function Brand() {
  return (
    <Link to="/app" className="ui-brand" aria-label="Factory workspace">
      factory
    </Link>
  )
}
