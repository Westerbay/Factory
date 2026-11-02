use factory_core::conversations::{
    Conversation, ConversationHistory, MessageRole, RequestKind, RequestState, RunStatus,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationView {
    pub id: Uuid,
    pub created_at: String,
    pub project_id: Option<Uuid>,
}
impl From<Conversation> for ConversationView {
    fn from(c: Conversation) -> Self {
        Self {
            id: c.id,
            created_at: c.created_at.to_rfc3339(),
            project_id: c.project_id,
        }
    }
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateConversationBody {
    pub project_id: Option<Uuid>,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPageView {
    pub items: Vec<ConversationView>,
    pub next_cursor: Option<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationListQuery {
    pub before: Option<Uuid>,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub operation_id: Uuid,
    pub model: String,
    pub effort: Option<String>,
    pub status: String,
}
impl From<factory_core::conversations::Run> for RunView {
    fn from(r: factory_core::conversations::Run) -> Self {
        let (model, effort) = factory_core::conversations::execution::decode_selection(&r.model);
        Self {
            id: r.id,
            conversation_id: r.conversation_id,
            operation_id: r.operation_id,
            model: model.to_owned(),
            effort: effort.map(str::to_owned),
            status: match r.status {
                RunStatus::Active => "active",
                RunStatus::Completed => "completed",
                RunStatus::Failed => "failed",
                RunStatus::Interrupted => "interrupted",
                RunStatus::Uncertain => "uncertain",
            }
            .into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub id: Uuid,
    pub run_id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum RequestStateView {
    Pending,
    Claimed,
    Uncertain,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestView {
    pub id: Uuid,
    pub run_id: Uuid,
    pub kind: String,
    pub state: RequestStateView,
    pub questions: Vec<QuestionView>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionOptionView {
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionView {
    pub id: Uuid,
    pub header: String,
    pub question: String,
    pub allow_free_text: bool,
    pub options: Vec<QuestionOptionView>,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HistoryView {
    pub conversation: ConversationView,
    pub runs: Vec<RunView>,
    pub messages: Vec<MessageView>,
    pub requests: Vec<RequestView>,
    pub runs_truncated: bool,
    pub messages_truncated: bool,
    pub requests_truncated: bool,
}
impl TryFrom<ConversationHistory> for HistoryView {
    type Error = ();
    fn try_from(h: ConversationHistory) -> Result<Self, ()> {
        Ok(Self {
            conversation: h.conversation.into(),
            runs_truncated: h.runs_truncated,
            messages_truncated: h.messages_truncated,
            requests_truncated: h.requests_truncated,
            runs: h.runs.into_iter().map(Into::into).collect(),
            messages: h
                .messages
                .into_iter()
                .map(|m| MessageView {
                    id: m.id,
                    run_id: m.run_id,
                    role: match m.role {
                        MessageRole::User => "user",
                        MessageRole::Assistant => "assistant",
                    }
                    .into(),
                    content: m.content,
                    created_at: m.created_at.to_rfc3339(),
                })
                .collect(),
            requests: h
                .requests
                .into_iter()
                .map(|q| {
                    if q.kind != RequestKind::Question {
                        return Err(());
                    }
                    Ok(RequestView {
                        id: q.id,
                        run_id: q.run_id,
                        kind: "question".into(),
                        state: match q.state {
                            RequestState::Pending => RequestStateView::Pending,
                            RequestState::Claimed => RequestStateView::Claimed,
                            RequestState::Uncertain => RequestStateView::Uncertain,
                        },
                        questions: serde_json::from_str(&q.details).map_err(|_| ())?,
                    })
                })
                .collect::<Result<Vec<_>, ()>>()?,
        })
    }
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub resolved_model: Option<String>,
    pub effort_levels: Vec<String>,
    pub default_effort: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartBody {
    pub operation_id: Uuid,
    pub model: String,
    pub effort: Option<String>,
    pub input: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnswerBody {
    pub operation_id: Uuid,
    pub answers: Vec<AnswerView>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerView {
    pub id: Uuid,
    pub value: String,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventView {
    pub kind: String,
    pub run_id: Uuid,
    pub text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projections_never_expose_provider_threads_or_user_identity() {
        let view = ConversationView::from(Conversation {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            project_id: None,
            provider_thread: Some("secret-native-thread".into()),
            created_at: chrono::Utc::now(),
        });
        let text = serde_json::to_string(&view).unwrap();
        assert!(!text.contains("secret-native-thread"));
        assert!(!text.contains("user"));
        assert!(text.contains("createdAt"));
    }
    #[test]
    fn malformed_question_records_fail_closed() {
        let history = ConversationHistory {
            conversation: Conversation {
                id: Uuid::new_v4(),
                user_id: Uuid::new_v4(),
                project_id: None,
                provider_thread: None,
                created_at: chrono::Utc::now(),
            },
            runs: vec![],
            messages: vec![],
            runs_truncated: false,
            messages_truncated: false,
            requests_truncated: false,
            requests: vec![factory_core::conversations::PendingRequest {
                id: Uuid::new_v4(),
                run_id: Uuid::new_v4(),
                kind: RequestKind::Question,
                state: RequestState::Pending,
                details: "not-json".into(),
            }],
        };
        assert!(HistoryView::try_from(history).is_err());
    }
}

/// Minimal stopped-state recovery identifier; never includes conversation contents.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableRunView {
    pub conversation_id: Uuid,
    pub run_id: Uuid,
    pub launch_settled: bool,
}
