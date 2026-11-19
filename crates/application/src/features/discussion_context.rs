//! Budget transient prompts without changing the durable conversation or proposals.
use crate::attachments::Vantage;
use factory_core::{
    conversations::{ConversationHistory, Message, MessageRole},
    features::FeatureError,
};

pub(super) fn compose(
    mut prefix: String,
    history: &ConversationHistory,
    current: String,
    limit: usize,
) -> Result<String, FeatureError> {
    if history.messages_truncated {
        return Err(FeatureError::Invalid);
    }
    let pinned_bytes = prefix.len().saturating_add(current.len());
    if pinned_bytes > limit {
        return Err(FeatureError::Invalid);
    }
    // Keep every message and attachment note of a run together. Only a contiguous
    // recent suffix is eligible; an oversized recent turn cannot expose older ones.
    let turns: Vec<_> = history
        .messages
        .chunk_by(|a, b| a.run_id == b.run_id)
        .collect();
    let sizes: Vec<usize> = turns
        .iter()
        .map(|turn| {
            turn.iter()
                .map(|message| {
                    role_label(message)
                        .len()
                        .saturating_add(message.content.len())
                        .saturating_add(
                            Vantage::Worker
                                .history_note(&history.attachments, message.id)
                                .len(),
                        )
                        .saturating_add(4) // ":\n", content newline, attachment-note newline
                })
                .fold(0, usize::saturating_add)
        })
        .collect();
    let mut remaining = sizes.iter().copied().fold(0, usize::saturating_add);
    let mut first = 0;
    let mut omitted_messages = 0;
    let notice = loop {
        let notice = if first == 0 {
            String::new()
        } else {
            format!(
                "[Context window: {first} older turns ({omitted_messages} messages) omitted from this prompt to fit the byte budget. The full transcript remains stored. Current proposals and the current user message are included in full. Do not assume omitted details; ask the member if needed.]\n\n"
            )
        };
        if pinned_bytes
            .saturating_add(remaining)
            .saturating_add(notice.len())
            <= limit
        {
            break notice;
        }
        if first == turns.len() {
            return Err(FeatureError::Invalid);
        }
        remaining = remaining.saturating_sub(sizes[first]);
        omitted_messages += turns[first].len();
        first += 1;
    };
    prefix.push_str(&notice);
    for turn in &turns[first..] {
        for message in *turn {
            prefix.push_str(role_label(message));
            prefix.push_str(":\n");
            prefix.push_str(&message.content);
            prefix.push('\n');
            prefix.push_str(&Vantage::Worker.history_note(&history.attachments, message.id));
            prefix.push('\n');
        }
    }
    prefix.push_str(&current);
    Ok(prefix)
}

fn role_label(message: &Message) -> &'static str {
    match message.role {
        MessageRole::User => "User",
        MessageRole::Assistant => "Assistant",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use factory_core::conversations::Conversation;
    use uuid::Uuid;

    fn history(turns: &[(&str, &str)]) -> ConversationHistory {
        ConversationHistory {
            conversation: Conversation {
                id: Uuid::new_v4(),
                user_id: Uuid::new_v4(),
                project_id: None,
                provider_thread: None,
                created_at: Utc::now(),
            },
            runs: vec![],
            attachments: vec![],
            requests: vec![],
            runs_truncated: false,
            messages_truncated: false,
            requests_truncated: false,
            messages: turns
                .iter()
                .flat_map(|(user, assistant)| {
                    let run_id = Uuid::new_v4();
                    [
                        (MessageRole::User, *user),
                        (MessageRole::Assistant, *assistant),
                    ]
                    .map(|(role, content)| Message {
                        id: Uuid::new_v4(),
                        run_id,
                        role,
                        content: content.into(),
                        created_at: Utc::now(),
                    })
                })
                .collect(),
        }
    }

    #[test]
    fn full_history_fits_at_the_exact_byte_boundary() {
        let history = history(&[("é😀", "answer"), ("next", "reply")]);
        let expected =
            "prefixUser:\né😀\n\nAssistant:\nanswer\n\nUser:\nnext\n\nAssistant:\nreply\n\ncurrent";
        assert_eq!(
            compose("prefix".into(), &history, "current".into(), expected.len()).unwrap(),
            expected
        );
        let windowed = compose(
            "prefix".into(),
            &history,
            "current".into(),
            expected.len() - 1,
        );
        // A notice does not fit this tiny budget: never truncate UTF-8 or omit silently.
        assert_eq!(windowed, Err(FeatureError::Invalid));
    }

    #[test]
    fn long_discussions_keep_pinned_context_and_recent_whole_turns() {
        let old = format!("old {}", "x".repeat(200_000));
        let recent = "récent😀".repeat(1_000);
        let history = history(&[(&old, "old answer"), (&recent, "latest answer")]);
        let before = history.clone();
        let prefix = format!(
            "instructions and canonical proposals {}\n",
            "p".repeat(28_000)
        );
        let current = format!(
            "Current user message:\n{}\ncomplete attachment manifest",
            "n".repeat(16_425)
        );
        let result = compose(prefix.clone(), &history, current.clone(), 240_000).unwrap();
        assert!(result.len() <= 240_000);
        assert!(result.starts_with(&prefix));
        assert!(result.ends_with(&current));
        assert!(result.contains("1 older turns (2 messages) omitted"));
        assert!(!result.contains("old answer"));
        assert!(result.contains(&format!("User:\n{recent}\n\nAssistant:\nlatest answer")));
        assert_eq!(history, before);
    }

    #[test]
    fn the_omission_notice_also_consumes_the_budget() {
        let old = "o".repeat(100);
        let middle = "m".repeat(350);
        let recent = "r".repeat(100);
        let history = history(&[(&old, "old"), (&middle, "middle"), (&recent, "recent")]);
        let complete = compose(String::new(), &history, String::new(), 240_000).unwrap();
        let limit = complete.len() - 1;
        let result = compose(String::new(), &history, String::new(), limit).unwrap();
        assert!(result.len() <= limit);
        assert!(result.contains("2 older turns (4 messages) omitted"));
        assert!(!result.contains(&middle));
        assert!(result.contains(&recent));
    }

    #[test]
    fn an_oversized_recent_turn_does_not_select_older_messages() {
        let large = "x".repeat(5_000);
        let history = history(&[
            ("small old request", "small old reply"),
            ("recent request", &large),
        ]);
        let result = compose("prefix".into(), &history, "current".into(), 1_000).unwrap();
        assert!(result.contains("2 older turns (4 messages) omitted"));
        assert!(!result.contains("small old"));
        assert!(!result.contains("recent request"));
    }

    #[test]
    fn current_message_manifest_and_proposals_are_never_cut() {
        let history = history(&[]);
        assert_eq!(
            compose("canonical".into(), &history, "input and files".into(), 24).unwrap(),
            "canonicalinput and files"
        );
        assert_eq!(
            compose("canonical".into(), &history, "input and files".into(), 23),
            Err(FeatureError::Invalid)
        );
        let mut incomplete = history;
        incomplete.messages_truncated = true;
        assert_eq!(
            compose(String::new(), &incomplete, String::new(), 240_000),
            Err(FeatureError::Invalid)
        );
    }

    #[test]
    fn historical_attachment_notes_stay_with_their_turn() {
        use factory_core::attachments::{Attachment, AttachmentSource, AttachmentState, MediaKind};
        let mut history = history(&[("old", "old reply"), ("recent", "recent reply")]);
        history.attachments = [0, 2]
            .into_iter()
            .map(|index| Attachment {
                id: Uuid::new_v4(),
                conversation_id: history.conversation.id,
                run_id: Some(history.messages[index].run_id),
                message_id: Some(history.messages[index].id),
                source: AttachmentSource::User,
                state: AttachmentState::Attached,
                name: format!("attachment-{index}.txt"),
                media_type: "text/plain".into(),
                kind: MediaKind::Text,
                size: 1,
                created_at: Utc::now(),
            })
            .collect();
        let complete = compose(String::new(), &history, String::new(), 240_000).unwrap();
        assert!(complete.contains("attachment-0.txt"));
        assert!(complete.contains("attachment-2.txt"));
        let recent_note =
            Vantage::Worker.history_note(&history.attachments, history.messages[2].id);
        history.messages[0].content = "x".repeat(5_000);
        let result = compose(String::new(), &history, String::new(), 2_000).unwrap();
        assert!(!result.contains("attachment-0.txt"));
        assert!(result.contains(&recent_note));
    }
}
