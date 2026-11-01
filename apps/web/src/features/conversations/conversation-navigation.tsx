import { useTranslation } from "react-i18next"
import i18n from "../../lib/i18n"
import { Button } from "../../components/ui/button"
import type { Conversation } from "./api"

export function conversationLabel(conversation: Conversation): string {
  return `${i18n.t("Chat")} · ${new Date(conversation.createdAt).toLocaleString(
    i18n.language,
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
  const { t: translate } = useTranslation()
  return (
    <div className="conversation-navigation">
      {(conversations.length > 0 || hasOlder || failed) && (
        <p className="conversation-list-label">{translate("Other chats")}</p>
      )}
      <nav
        className="conversation-list"
        aria-label={translate("Conversations")}
      >
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
            {loadingMore ? translate("Loading…") : translate("Older chats")}
          </Button>
        )}
      </nav>
      {failed && (
        <div className="conversation-list-error" role="status">
          <p>{translate("Chats could not be loaded.")}</p>
          <Button variant="ghost" disabled={!available} onClick={onRetry}>
            {translate("Retry")}{" "}
          </Button>
        </div>
      )}
    </div>
  )
}
