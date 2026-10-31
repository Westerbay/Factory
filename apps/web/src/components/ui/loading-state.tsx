export function LoadingState({ title }: { title: string }) {
  return (
    <div className="ui-loading" role="status" aria-live="polite">
      <span className="ui-spinner" aria-hidden="true" />
      <p className="ui-loading-title">{title}</p>
    </div>
  )
}
