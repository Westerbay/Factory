import { SquarePen } from "lucide-react"

import { Button } from "../../components/ui/button"
import type { Conversation } from "./api"

export function conversationLabel(conversation: Conversation): string {
  return `Chat · ${new Date(conversation.createdAt).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })}`
}

export function ConversationNavigation({
  conversations,
  selected,
  available,
  busy,
  hasOlder,
  loadingMore,
  failed,
  onCreate,
  onSelect,
  onLoadOlder,
  onRetry,
}: {
  conversations: Conversation[]
  selected: string | undefined
  available: boolean
  busy: boolean
  hasOlder: boolean
  loadingMore: boolean
  failed: boolean
  onCreate: () => void
  onSelect: (id: string) => void
  onLoadOlder: () => void
  onRetry: () => void
}) {
  return (
    <div className="conversation-navigation">
      <Button
        variant="ghost"
        className="conversation-new"
        onClick={onCreate}
        disabled={!available || busy}
      >
        <SquarePen aria-hidden="true" size={16} />
        New chat
      </Button>
      <p className="conversation-list-label">Chats</p>
      <nav className="conversation-list" aria-label="Conversations">
        {conversations.map((conversation) => {
          const label = conversationLabel(conversation)
          return (
            <Button
              variant="ghost"
              className="conversation-row"
              key={conversation.id}
              aria-current={conversation.id === selected ? "page" : undefined}
              title={label}
              disabled={busy || !available}
              onClick={() => onSelect(conversation.id)}
            >
              <span>{label}</span>
            </Button>
          )
        })}
        {hasOlder && (
          <Button
            variant="ghost"
            className="conversation-older"
            disabled={!available || loadingMore}
            onClick={onLoadOlder}
          >
            {loadingMore ? "Loading…" : "Older chats"}
          </Button>
        )}
      </nav>
      {failed && (
        <div className="conversation-list-error" role="status">
          <p>Chats could not be loaded.</p>
          <Button variant="ghost" disabled={!available} onClick={onRetry}>
            Retry
          </Button>
        </div>
      )}
    </div>
  )
}
