import * as m from "@workspace/i18n/messages"
import { useLocale, getLanguage } from "../../lib/i18n"
import { Button } from "../../components/ui/button"
import type { Conversation } from "./api"

export function conversationLabel(conversation: Conversation): string {
  return `${m.chat()} · ${new Date(conversation.createdAt).toLocaleString(
    getLanguage(),
    {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }
  )}`
}

export function ConversationNavigation({
  conversations,
  selected,
  available,
  busy,
  hasOlder,
  loadingMore,
  failed,
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
  onSelect: (id: string) => void
  onLoadOlder: () => void
  onRetry: () => void
}) {
  useLocale()
  return (
    <div className="conversation-navigation">
      {(conversations.length > 0 || hasOlder || failed) && (
        <p className="conversation-list-label">{m.other_chats()}</p>
      )}
      <nav className="conversation-list" aria-label={m.conversations()}>
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
            {loadingMore ? m.loading() : m.older_chats()}
          </Button>
        )}
      </nav>
      {failed && (
        <div className="conversation-list-error" role="status">
          <p>{m.chats_could_not_be_loaded()}</p>
          <Button variant="ghost" disabled={!available} onClick={onRetry}>
            {m.retry()}{" "}
          </Button>
        </div>
      )}
    </div>
  )
}
