use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTask {
    ClassifyComplexity,
    ChooseRetrievalStrategy,
    NeedMoreContext,
    EvidenceSufficient,
    NextSource,
    EscalateToClaude,
    UseSemanticFallback,
    ChooseWorkflow,
}

impl DecisionTask {
    pub fn question_name(self) -> &'static str {
        match self {
            DecisionTask::ClassifyComplexity => "complexity",
            DecisionTask::ChooseRetrievalStrategy => "retrieval_strategy",
            DecisionTask::NeedMoreContext => "need_more_context",
            DecisionTask::EvidenceSufficient => "evidence_sufficient",
            DecisionTask::NextSource => "next_source",
            DecisionTask::EscalateToClaude => "escalate",
            DecisionTask::UseSemanticFallback => "semantic_fallback",
            DecisionTask::ChooseWorkflow => "workflow",
        }
    }

    pub fn instructions(self) -> &'static str {
        match self {
            DecisionTask::ClassifyComplexity => "How complex is the requested engineering task?",
            DecisionTask::ChooseRetrievalStrategy => {
                "Which retrieval strategy best fits the current evidence?"
            }
            DecisionTask::NeedMoreContext => {
                "Is additional source context required to answer correctly?"
            }
            DecisionTask::EvidenceSufficient => "Is the collected evidence sufficient to act on?",
            DecisionTask::NextSource => "Which source should be queried next?",
            DecisionTask::EscalateToClaude => {
                "Should the decision be escalated to the coding agent?"
            }
            DecisionTask::UseSemanticFallback => {
                "Is a semantic fallback worth its cost at this point?"
            }
            DecisionTask::ChooseWorkflow => {
                "Which bounded workflow path best fits the available evidence?"
            }
        }
    }

    pub fn options(self) -> Vec<String> {
        match self {
            DecisionTask::ClassifyComplexity => vec![
                "trivial".into(),
                "localized".into(),
                "cross_module".into(),
                "architectural".into(),
            ],
            DecisionTask::ChooseRetrievalStrategy | DecisionTask::NextSource => vec![
                "exact_symbol".into(),
                "lexical".into(),
                "graph_expansion".into(),
                "semantic".into(),
            ],
            DecisionTask::NeedMoreContext | DecisionTask::EvidenceSufficient => {
                vec!["yes".into(), "no".into()]
            }
            DecisionTask::EscalateToClaude => vec!["yes".into(), "no".into()],
            DecisionTask::UseSemanticFallback => vec!["yes".into(), "no".into()],
            DecisionTask::ChooseWorkflow => vec![
                "read_then_patch".into(),
                "impact_then_patch".into(),
                "plan_then_execute".into(),
                "escalate".into(),
            ],
        }
    }

    /// True when the task is a yes/no judgement, answered with a Noul.
    pub fn is_boolean(self) -> bool {
        matches!(
            self,
            DecisionTask::NeedMoreContext
                | DecisionTask::EvidenceSufficient
                | DecisionTask::EscalateToClaude
                | DecisionTask::UseSemanticFallback
        )
    }

    pub fn from_name(name: &str) -> Option<DecisionTask> {
        Some(match name {
            "complexity" | "classify_complexity" => DecisionTask::ClassifyComplexity,
            "retrieval_strategy" | "choose_retrieval_strategy" => {
                DecisionTask::ChooseRetrievalStrategy
            }
            "need_more_context" => DecisionTask::NeedMoreContext,
            "evidence_sufficient" => DecisionTask::EvidenceSufficient,
            "next_source" => DecisionTask::NextSource,
            "escalate" | "escalate_to_claude" => DecisionTask::EscalateToClaude,
            "semantic_fallback" | "use_semantic_fallback" => DecisionTask::UseSemanticFallback,
            "workflow" | "choose_workflow" => DecisionTask::ChooseWorkflow,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DecisionRequest {
    pub task: DecisionTask,
    /// Bounded, non sensitive summary of the current evidence.
    pub state: String,
    /// Closed set of admissible answers.
    pub options: Vec<String>,
    /// Optional hint that refines the question without changing its scope.
    pub hint: Option<String>,
}

impl DecisionRequest {
    pub fn new(task: DecisionTask, state: impl Into<String>) -> Self {
        Self {
            task,
            state: state.into(),
            options: task.options(),
            hint: None,
        }
    }

    pub fn with_options(mut self, options: Vec<String>) -> Self {
        if !options.is_empty() {
            self.options = options;
        }
        self
    }

    pub fn cache_key(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.task.question_name().as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.state.as_bytes());
        hasher.update(b"\x00");
        hasher.update(self.options.join(",").as_bytes());
        hasher.finalize().to_hex()[..24].to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionOutcome {
    pub task: DecisionTask,
    pub choice: Option<String>,
    pub probability: Option<f64>,
    pub confidence: Option<f64>,
    pub rationale: Option<String>,
    pub provider: String,
    pub cached: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub escalated: bool,
}

impl DecisionOutcome {
    pub fn deterministic(task: DecisionTask, choice: impl Into<String>, confidence: f64) -> Self {
        Self {
            task,
            choice: Some(choice.into()),
            probability: Some(confidence),
            confidence: Some(confidence),
            rationale: None,
            provider: "deterministic".to_string(),
            cached: false,
            input_tokens: 0,
            output_tokens: 0,
            escalated: false,
        }
    }

    pub fn escalated(task: DecisionTask, rationale: impl Into<String>) -> Self {
        Self {
            task,
            choice: Some("escalate".to_string()),
            probability: Some(1.0),
            confidence: Some(1.0),
            rationale: Some(rationale.into()),
            provider: "router".to_string(),
            cached: false,
            input_tokens: 0,
            output_tokens: 0,
            escalated: true,
        }
    }

    pub fn is_yes(&self) -> bool {
        matches!(self.choice.as_deref(), Some("yes") | Some("true"))
    }
}

/// Maps a task to the provider primitive that answers it.
pub fn question_payload(
    task: DecisionTask,
    options: &[String],
    hint: Option<&str>,
) -> BTreeMap<String, serde_json::Value> {
    let instructions = match hint {
        Some(h) => format!("{} {h}", task.instructions()),
        None => task.instructions().to_string(),
    };
    let mut questions = BTreeMap::new();
    if task.is_boolean() {
        questions.insert(
            task.question_name().to_string(),
            serde_json::json!({
                "type": "noul",
                "instructions": instructions,
            }),
        );
    } else {
        let mut criteria = serde_json::Map::new();
        for option in options {
            criteria.insert(option.clone(), serde_json::Value::Null);
        }
        questions.insert(
            task.question_name().to_string(),
            serde_json::json!({
                "type": "choice",
                "instructions": instructions,
                "criteria": serde_json::Value::Object(criteria),
            }),
        );
    }
    questions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_tasks_use_the_noul_primitive() {
        let request = DecisionRequest::new(DecisionTask::EvidenceSufficient, "exact_symbol");
        let payload = question_payload(request.task, &request.options, None);
        let question = &payload["evidence_sufficient"];
        assert_eq!(question["type"], "noul");
    }

    #[test]
    fn classification_tasks_use_the_choice_primitive() {
        let request = DecisionRequest::new(DecisionTask::ClassifyComplexity, "cross_module");
        let payload = question_payload(request.task, &request.options, None);
        let question = &payload["complexity"];
        assert_eq!(question["type"], "choice");
        assert!(question["criteria"].is_object());
        assert!(question["criteria"].get("architectural").is_some());
    }

    #[test]
    fn the_cache_key_depends_on_task_state_and_options() {
        let a = DecisionRequest::new(DecisionTask::NextSource, "estado");
        let b = DecisionRequest::new(DecisionTask::NextSource, "estado");
        let c = DecisionRequest::new(DecisionTask::NextSource, "outro");
        assert_eq!(a.cache_key(), b.cache_key());
        assert_ne!(a.cache_key(), c.cache_key());
    }

    #[test]
    fn task_names_round_trip() {
        for task in [
            DecisionTask::ClassifyComplexity,
            DecisionTask::ChooseRetrievalStrategy,
            DecisionTask::NeedMoreContext,
            DecisionTask::EvidenceSufficient,
            DecisionTask::NextSource,
            DecisionTask::EscalateToClaude,
            DecisionTask::UseSemanticFallback,
            DecisionTask::ChooseWorkflow,
        ] {
            assert_eq!(DecisionTask::from_name(task.question_name()), Some(task));
        }
        assert_eq!(DecisionTask::from_name("inexistente"), None);
    }
}
