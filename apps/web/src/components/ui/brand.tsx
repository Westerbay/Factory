import { Link } from "@tanstack/react-router"

export function Brand() {
  return (
    <Link to="/app" className="ui-brand" aria-label="Factory workspace">
      <svg viewBox="0 0 80 80" aria-hidden="true" focusable="false">
        <path
          d="M12 17 34 39 12 61M40 62h28"
          fill="none"
          stroke="currentColor"
          strokeWidth="8"
          strokeLinecap="square"
          strokeLinejoin="miter"
        />
      </svg>
      <span>factory</span>
    </Link>
  )
}
