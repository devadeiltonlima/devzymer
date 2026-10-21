use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;

use crate::schema::{question_payload, DecisionOutcome, DecisionRequest};

pub type DecisionFuture<'a> = Pin<Box<dyn Future<Output = Result<DecisionOutcome>> + Send + 'a>>;

pub type Result<T> = std::result::Result<T, ProviderError>;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider is not configured: {0}")]
    NotConfigured(String),
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned an unexpected response: {0}")]
    BadResponse(String),
    #[error("provider timed out")]
    Timeout,
    #[error("provider rejected the request: {0}")]
    Rejected(String),
}

/// Object safe decision provider.
pub trait DecisionProvider: Send + Sync {
    fn name(&self) -> &str;
    fn decide<'a>(&'a self, request: &'a DecisionRequest) -> DecisionFuture<'a>;
}

/// Fallback provider that answers from local rules only.
pub struct DeterministicProvider;

impl DecisionProvider for DeterministicProvider {
    fn name(&self) -> &str {
        "deterministic"
    }

    fn decide<'a>(&'a self, request: &'a DecisionRequest) -> DecisionFuture<'a> {
        Box::pin(async move { Ok(deterministic_outcome(request)) })
    }
}

/// Derives an answer from the request itself, without any network call.
pub fn deterministic_outcome(request: &DecisionRequest) -> DecisionOutcome {
    use crate::schema::DecisionTask;
    let state = request.state.to_ascii_lowercase();
    match request.task {
        DecisionTask::NeedMoreContext => {
            let empty = state.contains("no results") || state.contains("nenhum resultado");
            DecisionOutcome::deterministic(
                request.task,
                if empty { "yes" } else { "no" },
                if empty { 0.85 } else { 0.7 },
            )
        }
        DecisionTask::EvidenceSufficient => {
            let strong = state.matches("exact_symbol").count()
                + state.matches("exact_path").count()
                + state.matches("confirmed").count();
            let sufficient = strong >= 2;
            DecisionOutcome::deterministic(
                request.task,
                if sufficient { "yes" } else { "no" },
                if sufficient { 0.8 } else { 0.6 },
            )
        }
        DecisionTask::ClassifyComplexity => {
            let choice = if state.contains("architectural") || state.contains("security") {
                "architectural"
            } else if state.contains("cross_module") || state.contains("multi-file") {
                "cross_module"
            } else if state.contains("trivial") || state.contains("typo") {
                "trivial"
            } else {
                "localized"
            };
            DecisionOutcome::deterministic(request.task, choice, 0.6)
        }
        DecisionTask::ChooseRetrievalStrategy | DecisionTask::NextSource => {
            let choice = if state.contains("exact_symbol") {
                "exact_symbol"
            } else if state.contains("graph") {
                "graph_expansion"
            } else {
                "lexical"
            };
            DecisionOutcome::deterministic(request.task, choice, 0.55)
        }
        DecisionTask::UseSemanticFallback => {
            let choice = if state.contains("no results") || state.contains("ambiguous") {
                "yes"
            } else {
                "no"
            };
            DecisionOutcome::deterministic(request.task, choice, 0.5)
        }
        DecisionTask::EscalateToClaude => {
            let choice = if state.contains("security") || state.contains("architecture") {
                "yes"
            } else {
                "no"
            };
            DecisionOutcome::deterministic(request.task, choice, 0.5)
        }
        DecisionTask::ChooseWorkflow => {
            let choice = if state.contains("impact") {
                "impact_then_patch"
            } else if state.contains("plan") {
                "plan_then_execute"
            } else {
                "read_then_patch"
            };
            DecisionOutcome::deterministic(request.task, choice, 0.5)
        }
    }
}

/// Client for the OpenRouter chat completions endpoint.
///
/// The Jev model is published on OpenRouter as `typesafe/jev-1.14`, but the
/// endpoint there is the ordinary chat completion one, and the model does not
/// declare the structured output parameter. The decision is therefore asked
/// for in a schema and the answer is parsed leniently: a valid object is used
/// as it is, and a free text answer falls back to matching the known options.
pub struct OpenRouterProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_retries: u32,
}

impl OpenRouterProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        timeout: Duration,
        max_retries: u32,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_retries,
        })
    }

    fn endpoint(&self) -> String {
        if self.base_url.ends_with("/chat/completions") {
            self.base_url.clone()
        } else {
            format!("{}/chat/completions", self.base_url)
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f64,
    response_format: ResponseFormat,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageBody,
}

#[derive(Debug, Deserialize)]
struct ChatMessageBody {
    content: Option<String>,
}

impl DecisionProvider for OpenRouterProvider {
    fn name(&self) -> &str {
        "openrouter"
    }

    fn decide<'a>(&'a self, request: &'a DecisionRequest) -> DecisionFuture<'a> {
        Box::pin(async move {
            if self.api_key.trim().is_empty() {
                return Err(ProviderError::NotConfigured("missing API key".into()));
            }
            let options = request.options.clone();
            let question = describe_question(request, &options);
            let payload = ChatRequest {
                model: self.model.clone(),
                messages: vec![
                    ChatMessage {
                        role: "system".into(),
                        content: "You are a decision model. Answer with the single best option and a confidence between 0 and 1.".into(),
                    },
                    ChatMessage {
                        role: "user".into(),
                        content: format!("{}

{}", request.state, question),
                    },
                ],
                temperature: 0.0,
                response_format: ResponseFormat {
                    kind: "json_schema".into(),
                    json_schema: JsonSchema {
                        name: "decision".into(),
                        strict: false,
                        schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "choice": { "type": "string", "description": "The chosen option" },
                                "confidence": { "type": "number", "description": "Confidence between 0 and 1" },
                                "probability": { "type": "number", "description": "Probability of the chosen option between 0 and 1" },
                                "rationale": { "type": "string", "description": "Short reason" }
                            },
                            "required": ["choice", "confidence"]
                        }),
                    },
                },
            };
            let body =
                serde_json::to_vec(&payload).map_err(|e| ProviderError::Request(e.to_string()))?;

            let mut attempt = 0u32;
            loop {
                let response = self
                    .client
                    .post(self.endpoint())
                    .bearer_auth(&self.api_key)
                    .header("content-type", "application/json")
                    .body(body.clone())
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            let text = resp
                                .text()
                                .await
                                .map_err(|e| ProviderError::Request(e.to_string()))?;
                            let parsed: ChatResponse = serde_json::from_str(&text)
                                .map_err(|e| ProviderError::BadResponse(e.to_string()))?;
                            return Ok(interpret_chat(request, &options, &parsed));
                        }
                        if status == reqwest::StatusCode::UNAUTHORIZED
                            || status == reqwest::StatusCode::FORBIDDEN
                        {
                            return Err(ProviderError::Rejected(format!("status {status}")));
                        }
                        let body_text = resp.text().await.unwrap_or_default();
                        if attempt >= self.max_retries {
                            return Err(ProviderError::Rejected(format!(
                                "status {status}: {}",
                                body_text.chars().take(200).collect::<String>()
                            )));
                        }
                    }
                    Err(err) => {
                        if err.is_timeout() {
                            return Err(ProviderError::Timeout);
                        }
                        if attempt >= self.max_retries {
                            return Err(ProviderError::Request(err.to_string()));
                        }
                    }
                }
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
            }
        })
    }
}

/// Renders the question and the admissible options as text.
fn describe_question(request: &DecisionRequest, options: &[String]) -> String {
    let mut text = request.task.instructions().to_string();
    if let Some(hint) = &request.hint {
        text.push(' ');
        text.push_str(hint);
    }
    text.push_str("\nOptions:");
    for option in options {
        text.push_str("\n- ");
        text.push_str(option);
    }
    text.push_str("\nAnswer with the option exactly as written.");
    text
}

/// Interprets the answer of the chat endpoint, accepting both a JSON object
/// and a free text that names one of the options.
fn interpret_chat(
    request: &DecisionRequest,
    options: &[String],
    response: &ChatResponse,
) -> DecisionOutcome {
    let usage = response.usage.unwrap_or_default();
    let content = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default();

    let mut outcome = DecisionOutcome {
        task: request.task,
        choice: None,
        probability: None,
        confidence: None,
        rationale: None,
        provider: "openrouter".to_string(),
        cached: false,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        escalated: false,
    };

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) {
        if let Some(choice) = value.get("choice").and_then(|v| v.as_str()) {
            outcome.choice = Some(choice.to_string());
        }
        if let Some(confidence) = value.get("confidence").and_then(|v| v.as_f64()) {
            outcome.confidence = Some(confidence.clamp(0.0, 1.0));
        }
        if let Some(probability) = value.get("probability").and_then(|v| v.as_f64()) {
            outcome.probability = Some(probability.clamp(0.0, 1.0));
        }
        if let Some(rationale) = value.get("rationale").and_then(|v| v.as_str()) {
            outcome.rationale = Some(rationale.to_string());
        }
    }

    if outcome.choice.is_none() {
        let lowered = content.to_ascii_lowercase();
        for option in options {
            if lowered.contains(&option.to_ascii_lowercase()) {
                outcome.choice = Some(option.clone());
                break;
            }
        }
    }
    if outcome.choice.is_none() {
        outcome.choice = Some(options.first().cloned().unwrap_or_default());
    }
    outcome
}

/// Client for the TypeSafe System One API, the documented decision interface
/// of the Jev model family.
pub struct JevProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_retries: u32,
}

impl JevProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        timeout: Duration,
        max_retries: u32,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_retries,
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/systemone", self.base_url)
    }
}

#[derive(Debug, Serialize)]
struct SystemOneRequest {
    model: String,
    state: String,
    questions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SystemOneResponse {
    #[serde(default)]
    model: Option<String>,
    answers: BTreeMap<String, Answer>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(rename = "type", default)]
    #[allow(dead_code)]
    kind: Option<String>,
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    noul: Option<f64>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    probabilities: Option<BTreeMap<String, f64>>,
}

#[derive(Debug, Deserialize, Default, Clone, Copy)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

impl DecisionProvider for JevProvider {
    fn name(&self) -> &str {
        "jev"
    }

    fn decide<'a>(&'a self, request: &'a DecisionRequest) -> DecisionFuture<'a> {
        Box::pin(async move {
            if self.api_key.trim().is_empty() {
                return Err(ProviderError::NotConfigured("missing API key".into()));
            }
            let payload = SystemOneRequest {
                model: self.model.clone(),
                state: request.state.clone(),
                questions: question_payload(
                    request.task,
                    &request.options,
                    request.hint.as_deref(),
                ),
            };
            let body =
                serde_json::to_vec(&payload).map_err(|e| ProviderError::Request(e.to_string()))?;

            let mut attempt = 0u32;
            loop {
                let response = self
                    .client
                    .post(self.endpoint())
                    .bearer_auth(&self.api_key)
                    .header("content-type", "application/json")
                    .body(body.clone())
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            let text = resp
                                .text()
                                .await
                                .map_err(|e| ProviderError::Request(e.to_string()))?;
                            let parsed: SystemOneResponse = serde_json::from_str(&text)
                                .map_err(|e| ProviderError::BadResponse(e.to_string()))?;
                            return Ok(interpret(request, parsed));
                        }
                        if status == reqwest::StatusCode::UNAUTHORIZED
                            || status == reqwest::StatusCode::FORBIDDEN
                        {
                            return Err(ProviderError::Rejected(format!("status {status}")));
                        }
                        let body_text = resp.text().await.unwrap_or_default();
                        if attempt >= self.max_retries {
                            return Err(ProviderError::Rejected(format!(
                                "status {status}: {}",
                                body_text.chars().take(200).collect::<String>()
                            )));
                        }
                    }
                    Err(err) => {
                        if err.is_timeout() {
                            return Err(ProviderError::Timeout);
                        }
                        if attempt >= self.max_retries {
                            return Err(ProviderError::Request(err.to_string()));
                        }
                    }
                }
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
            }
        })
    }
}

fn interpret(request: &DecisionRequest, response: SystemOneResponse) -> DecisionOutcome {
    let usage = response.usage.unwrap_or_default();
    let answer = response.answers.get(request.task.question_name());
    let mut outcome = DecisionOutcome {
        task: request.task,
        choice: None,
        probability: None,
        confidence: None,
        rationale: None,
        provider: response.model.unwrap_or_else(|| "jev".to_string()),
        cached: false,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        escalated: false,
    };

    if let Some(answer) = answer {
        if let Some(choice) = &answer.choice {
            outcome.choice = Some(choice.clone());
            if let Some(probabilities) = &answer.probabilities {
                outcome.probability = probabilities.get(choice).copied();
            }
        }
        if let Some(noul) = answer.noul {
            outcome.probability = Some(noul);
            outcome.choice = Some(if noul >= 0.5 { "yes" } else { "no" }.to_string());
        }
        outcome.confidence = answer.confidence;
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{DecisionRequest, DecisionTask};

    fn chat(content: &str) -> ChatResponse {
        ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessageBody {
                    content: Some(content.to_string()),
                },
            }],
            usage: Some(Usage {
                input_tokens: 12,
                output_tokens: 3,
            }),
        }
    }

    #[test]
    fn a_json_answer_is_read_as_it_is() {
        let request = DecisionRequest::new(DecisionTask::EvidenceSufficient, "evidencia");
        let options = vec!["yes".to_string(), "no".to_string()];
        let outcome = interpret_chat(
            &request,
            &options,
            &chat(
                r#"{"choice":"yes","confidence":0.82,"probability":0.79,"rationale":"evidencia suficiente"}"#,
            ),
        );
        assert_eq!(outcome.choice.as_deref(), Some("yes"));
        assert_eq!(outcome.confidence, Some(0.82));
        assert_eq!(outcome.input_tokens, 12);
    }

    #[test]
    fn a_free_text_answer_falls_back_to_the_named_option() {
        let request = DecisionRequest::new(DecisionTask::EvidenceSufficient, "evidencia");
        let options = vec!["yes".to_string(), "no".to_string()];
        let outcome = interpret_chat(
            &request,
            &options,
            &chat("A resposta e yes, com alta confianca"),
        );
        assert_eq!(outcome.choice.as_deref(), Some("yes"));
    }

    #[test]
    fn the_question_lists_the_admissible_options() {
        let request = DecisionRequest::new(DecisionTask::NextSource, "estado");
        let options = vec!["lexical".to_string(), "graph".to_string()];
        let question = describe_question(&request, &options);
        assert!(question.contains("lexical"));
        assert!(question.contains("graph"));
        assert!(!question.contains("estado"));
    }
}
