//! Private provider session references within the existing bounded conversation record.
use crate::attachments::Vantage;
use factory_core::conversations::{ConversationError, ConversationHistory, MessageRole, RunStatus};
use std::collections::BTreeMap;

const PREFIX: &str = "factory-sessions-v1|";
const LIMIT: usize = 512;
const PROMPT_LIMIT: usize = 256 * 1024;

pub(super) fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn valid_reference(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= LIMIT && !s.contains('\0')
}

fn sessions(raw: Option<&str>) -> Result<BTreeMap<String, String>, ConversationError> {
    let mut result = BTreeMap::new();
    let Some(raw) = raw else { return Ok(result) };
    let Some(mut rest) = raw.strip_prefix(PREFIX) else {
        if !valid_reference(raw) {
            return Err(ConversationError::Unavailable);
        }
        result.insert("codex".into(), raw.into());
        return Ok(result);
    };
    while !rest.is_empty() {
        let (length, next) = rest.split_once(':').ok_or(ConversationError::Unavailable)?;
        let n = length
            .parse::<usize>()
            .map_err(|_| ConversationError::Unavailable)?;
        let provider = next.get(..n).ok_or(ConversationError::Unavailable)?;
        rest = next.get(n..).ok_or(ConversationError::Unavailable)?;
        let (length, next) = rest.split_once(':').ok_or(ConversationError::Unavailable)?;
        let n = length
            .parse::<usize>()
            .map_err(|_| ConversationError::Unavailable)?;
        let reference = next.get(..n).ok_or(ConversationError::Unavailable)?;
        rest = next.get(n..).ok_or(ConversationError::Unavailable)?;
        if !valid_id(provider)
            || !valid_reference(reference)
            || result.insert(provider.into(), reference.into()).is_some()
            || result.len() > 16
        {
            return Err(ConversationError::Unavailable);
        }
    }
    if result.is_empty() {
        return Err(ConversationError::Unavailable);
    }
    Ok(result)
}

pub(super) fn select(
    raw: Option<&str>,
    provider: &str,
) -> Result<Option<String>, ConversationError> {
    if !valid_id(provider) {
        return Err(ConversationError::Invalid);
    }
    Ok(sessions(raw)?.remove(provider))
}

pub(super) fn merge(
    raw: Option<&str>,
    provider: &str,
    reference: &str,
) -> Result<String, ConversationError> {
    if !valid_id(provider) || !valid_reference(reference) {
        return Err(ConversationError::Invalid);
    }
    let mut entries = sessions(raw)?;
    if entries
        .get(provider)
        .is_some_and(|existing| existing != reference)
    {
        return Err(ConversationError::Conflict);
    }
    entries.insert(provider.into(), reference.into());
    if entries.len() > 16 {
        return Err(ConversationError::Conflict);
    }
    if entries.len() == 1 && provider == "codex" {
        return Ok(reference.into());
    }
    let mut encoded = PREFIX.to_owned();
    for (key, value) in entries {
        encoded.push_str(&format!("{}:{}{}:{}", key.len(), key, value.len(), value));
    }
    if encoded.len() > LIMIT {
        return Err(ConversationError::Conflict);
    }
    Ok(encoded)
}

/// Carry the visible durable transcript when changing providers; never silently truncate it.
pub(super) fn handoff(
    history: &ConversationHistory,
    provider: &str,
    input: &str,
) -> Result<String, ConversationError> {
    // A failed attempt does not prove that this provider received the handoff.
    // Retry it until a completed run of this provider follows the other provider.
    let needs_handoff = history
        .runs
        .iter()
        .rev()
        .take_while(|run| {
            model_provider(&run.model) != provider || run.status != RunStatus::Completed
        })
        .any(|run| model_provider(&run.model) != provider);
    if !needs_handoff {
        return Ok(input.to_owned());
    }
    if history.messages_truncated || history.runs_truncated {
        return Err(ConversationError::ContextLimit);
    }
    let mut prompt = String::from(
        "The following is the preceding conversation, provided as context after changing providers.\n\n",
    );
    let vantage = Vantage::Container {
        project_id: history.conversation.project_id,
    };
    for message in &history.messages {
        let role = match message.role {
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
        };
        prompt.push_str(role);
        prompt.push_str(":\n");
        prompt.push_str(&message.content);
        prompt.push('\n');
        prompt.push_str(&vantage.history_note(&history.attachments, message.id));
        prompt.push('\n');
        if prompt.len().saturating_add(input.len()) > PROMPT_LIMIT {
            return Err(ConversationError::ContextLimit);
        }
    }
    prompt.push_str("Current user message:\n");
    prompt.push_str(input);
    if prompt.len() > PROMPT_LIMIT {
        return Err(ConversationError::ContextLimit);
    }
    Ok(prompt)
}

fn model_provider(model: &str) -> &str {
    model
        .strip_prefix("factory:")
        .and_then(|value| value.split_once(':'))
        .map_or("codex", |(provider, _)| provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use factory_core::conversations::{Conversation, Message, Run, RunStatus};
    use uuid::Uuid;
    #[test]
    fn legacy_codex_session_survives_adding_a_second_provider() {
        let encoded = merge(Some("old-codex-thread"), "claude", "claude-thread").unwrap();
        assert_eq!(
            select(Some(&encoded), "codex").unwrap().as_deref(),
            Some("old-codex-thread")
        );
        assert_eq!(
            select(Some(&encoded), "claude").unwrap().as_deref(),
            Some("claude-thread")
        );
        assert_eq!(
            merge(Some(&encoded), "codex", "other"),
            Err(ConversationError::Conflict)
        );
        assert_eq!(
            merge(Some(&encoded), "claude", "claude-thread").unwrap(),
            encoded
        );
        assert!(select(Some("factory-sessions-v1|bad"), "codex").is_err());
    }

    #[test]
    fn switching_provider_carries_history_without_silent_truncation() {
        let conversation = Uuid::new_v4();
        let run = Uuid::new_v4();
        let mut history = ConversationHistory {
            conversation: Conversation {
                id: conversation,
                user_id: Uuid::new_v4(),
                project_id: None,
                provider_thread: Some("legacy-thread".into()),
                created_at: Utc::now(),
            },
            runs: vec![Run {
                id: run,
                conversation_id: conversation,
                operation_id: Uuid::new_v4(),
                environment_generation: 1,
                model: "codex-model".into(),
                status: RunStatus::Completed,
            }],
            messages: vec![
                Message {
                    id: Uuid::new_v4(),
                    run_id: run,
                    role: MessageRole::User,
                    content: "Earlier request".into(),
                    created_at: Utc::now(),
                },
                Message {
                    id: Uuid::new_v4(),
                    run_id: run,
                    role: MessageRole::Assistant,
                    content: "Earlier answer".into(),
                    created_at: Utc::now(),
                },
            ],
            attachments: vec![],
            requests: vec![],
            runs_truncated: false,
            messages_truncated: false,
            requests_truncated: false,
        };
        assert_eq!(handoff(&history, "codex", "Next").unwrap(), "Next");
        let prompt = handoff(&history, "claude", "Next").unwrap();
        assert!(
            prompt.contains("Earlier request")
                && prompt.contains("Earlier answer")
                && prompt.ends_with("Next")
        );
        let mut retry = history.runs[0].clone();
        retry.id = Uuid::new_v4();
        retry.model = "factory:claude:sonnet".into();
        retry.status = RunStatus::Failed;
        history.runs.push(retry);
        assert!(
            handoff(&history, "claude", "Retry")
                .unwrap()
                .contains("Earlier answer")
        );
        history.runs.last_mut().unwrap().status = RunStatus::Completed;
        assert_eq!(handoff(&history, "claude", "Continue").unwrap(), "Continue");
        history.runs.last_mut().unwrap().status = RunStatus::Failed;
        history.messages_truncated = true;
        assert_eq!(
            handoff(&history, "claude", "Next"),
            Err(ConversationError::ContextLimit)
        );
    }
}
