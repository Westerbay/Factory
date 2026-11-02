//! Host execution facilities and HTTP serialization for conversation policy.
use super::contracts::{AnswerView, QuestionOptionView, QuestionView};
pub use factory_application::conversations::coordinator::ConversationRuntime;
use factory_application::conversations::coordinator::{
    ConversationCodec, ExecutionHost, ExecutionTicker, Task, Wait,
};
use factory_core::conversations::{
    ConversationError,
    execution::{ExecutionOutput, ExecutionQuestion, QuestionAnswer},
};
use std::{sync::Arc, time::Duration};

pub(crate) struct ApiHost;
impl ExecutionHost for ApiHost {
    fn spawn(&self, task: Task) {
        tokio::spawn(task);
    }
    fn ticker(&self) -> Box<dyn ExecutionTicker> {
        Box::new(ApiTicker(tokio::time::interval(Duration::from_secs(5))))
    }
}
struct ApiTicker(tokio::time::Interval);
impl ExecutionTicker for ApiTicker {
    fn next<'a>(&'a mut self, output: &'a mut dyn ExecutionOutput) -> Wait<'a> {
        Box::pin(async move {
            tokio::select! { value = output.next() => Some(value), _ = self.0.tick() => None }
        })
    }
}
struct ApiCodec;
impl ConversationCodec for ApiCodec {
    fn questions(&self, questions: &[ExecutionQuestion]) -> Result<String, ConversationError> {
        let views = questions
            .iter()
            .map(|q| QuestionView {
                id: q.id,
                header: q.header.clone(),
                question: q.question.clone(),
                allow_free_text: q.allow_free_text,
                options: q
                    .options
                    .iter()
                    .map(|o| QuestionOptionView {
                        label: o.label.clone(),
                        description: o.description.clone(),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&views).map_err(|_| ConversationError::Unavailable)
    }
    fn answers(&self, answers: &[QuestionAnswer]) -> Result<String, ConversationError> {
        let views = answers
            .iter()
            .map(|a| AnswerView {
                id: a.id,
                value: a.value.clone(),
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&views).map_err(|_| ConversationError::Invalid)
    }
}
pub fn new(
    service: Arc<factory_application::conversations::ConversationService>,
    runtime: Arc<dyn factory_core::conversations::execution::ExecutionRuntime>,
    accounts: Arc<factory_application::agent_connections::AgentConnectionService>,
) -> Arc<ConversationRuntime> {
    ConversationRuntime::new(
        service,
        runtime,
        accounts,
        Arc::new(ApiHost),
        Arc::new(ApiCodec),
    )
}

pub fn with_providers(
    service: Arc<factory_application::conversations::ConversationService>,
    providers: Vec<factory_application::conversations::coordinator::ExecutionProvider>,
) -> Result<Arc<ConversationRuntime>, ConversationError> {
    ConversationRuntime::with_providers(service, providers, Arc::new(ApiHost), Arc::new(ApiCodec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_core::conversations::execution::QuestionOption;
    use uuid::Uuid;

    #[test]
    fn codec_preserves_existing_question_and_answer_json() {
        let id = Uuid::new_v4();
        let questions = vec![ExecutionQuestion {
            id,
            header: "Choice".into(),
            question: "Which?".into(),
            allow_free_text: false,
            options: vec![QuestionOption {
                label: "Yes".into(),
                description: "Confirm".into(),
            }],
        }];
        let encoded = ApiCodec.questions(&questions).unwrap();
        let decoded: Vec<QuestionView> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded[0].id, id);
        assert!(!decoded[0].allow_free_text);
        assert_eq!(decoded[0].options[0].label, "Yes");
        let answer = ApiCodec
            .answers(&[QuestionAnswer {
                id,
                value: "Yes".into(),
            }])
            .unwrap();
        let decoded: Vec<AnswerView> = serde_json::from_str(&answer).unwrap();
        assert_eq!(decoded[0].id, id);
        assert_eq!(decoded[0].value, "Yes");
    }
}
