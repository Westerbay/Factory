//! Actual provider execution capabilities consumed by the application.
//! Provider references are internal and must not be exposed in HTTP projections.
use crate::environments::Environment;
use async_trait::async_trait;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionModel {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionQuestion {
    pub id: Uuid,
    pub header: String,
    pub question: String,
    pub allow_free_text: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionAnswer {
    pub id: Uuid,
    pub value: String,
}

/// Validate a complete grouped response before the application durably claims it.
/// Option answers use the displayed label; other text is accepted only when allowed.
pub fn validate_answers(
    questions: &[ExecutionQuestion],
    answers: &[QuestionAnswer],
) -> Result<(), ExecutionError> {
    if questions.is_empty() || questions.len() > 20 || answers.len() != questions.len() {
        return Err(ExecutionError::Invalid);
    }
    for (index, question) in questions.iter().enumerate() {
        if questions[..index].iter().any(|q| q.id == question.id) {
            return Err(ExecutionError::Invalid);
        }
        let mut matching = answers.iter().filter(|a| a.id == question.id);
        let answer = matching.next().ok_or(ExecutionError::Invalid)?;
        if matching.next().is_some()
            || answer.value.trim().is_empty()
            || answer.value.len() > 4000
            || answer.value.contains('\0')
            || (!question.allow_free_text
                && !question.options.iter().any(|o| o.label == answer.value))
        {
            return Err(ExecutionError::Invalid);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    #[error("Execution is unavailable before a process was started")]
    PreflightUnavailable,
    #[error("Execution is unavailable")]
    Unavailable,
    #[error("Execution protocol failed")]
    Protocol,
    #[error("Execution request is invalid")]
    Invalid,
    #[error("Execution state changed")]
    Conflict,
    /// Docker launch did not return conclusively; it may still reach the daemon.
    #[error("Execution launch could not be confirmed")]
    LaunchUncertain,
    /// The launch returned conclusively but later cleanup was not confirmed.
    #[error("Execution cleanup could not be confirmed")]
    CleanupUncertain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionEvent {
    /// Persist this private reference before calling begin_turn.
    ThreadReady {
        provider_thread: String,
    },
    Delta {
        provider_item: String,
        text: String,
    },
    Message {
        provider_item: String,
        text: String,
    },
    Questions {
        request_id: Uuid,
        questions: Vec<ExecutionQuestion>,
    },
    /// Only this event establishes response delivery, after correlation by the adapter.
    ResponseDelivered {
        request_id: Uuid,
    },
    Completed,
    Failed,
    Interrupted,
    /// A process or stream ended without proving the whole run stopped.
    Uncertain,
}

#[async_trait]
pub trait ExecutionControl: Send {
    /// Start once, after the owning application has persisted ThreadReady.
    async fn begin_turn(&mut self) -> Result<(), ExecutionError>;
    /// Send once after durable claim. A successful write is not delivery confirmation.
    async fn answer(
        &mut self,
        request_id: Uuid,
        answers: Vec<QuestionAnswer>,
    ) -> Result<(), ExecutionError>;
    /// An interrupt acknowledgement alone does not confirm cleanup.
    async fn interrupt(&mut self) -> Result<(), ExecutionError>;
}

#[async_trait]
pub trait ExecutionOutput: Send {
    async fn next(&mut self) -> Result<Option<ExecutionEvent>, ExecutionError>;
}

pub struct ExecutionSession {
    pub control: Box<dyn ExecutionControl>,
    pub output: Box<dyn ExecutionOutput>,
}

#[async_trait]
pub trait ExecutionRuntime: Send + Sync {
    async fn models(
        &self,
        environment: &Environment,
    ) -> Result<Vec<ExecutionModel>, ExecutionError>;
    async fn open(
        &self,
        environment: &Environment,
        provider_thread: Option<&str>,
        model: &str,
        input: &str,
    ) -> Result<ExecutionSession, ExecutionError>;
}
