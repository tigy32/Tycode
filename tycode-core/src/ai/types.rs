use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::model::Model;

#[derive(Debug, Clone)]
pub struct ConversationRequest {
    pub messages: Vec<Message>,
    pub model: ModelSettings,
    pub system_prompt: String,
    pub stop_sequences: Vec<String>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum ReasoningBudget {
    Off,
    Low,
    Medium,
    #[default]
    High,
    Max,
}

impl ReasoningBudget {
    pub fn get_max_tokens(&self) -> Option<u32> {
        match self {
            ReasoningBudget::Off => None,
            ReasoningBudget::Low => Some(4000),
            ReasoningBudget::Medium => Some(8000),
            ReasoningBudget::High => Some(16000),
            ReasoningBudget::Max => Some(32000),
        }
    }

    pub fn get_effort_level(&self) -> Option<&'static str> {
        match self {
            ReasoningBudget::Off => None,
            ReasoningBudget::Low => Some("low"),
            ReasoningBudget::Medium => Some("medium"),
            ReasoningBudget::High => Some("high"),
            ReasoningBudget::Max => Some("max"),
        }
    }

    pub fn from_u32(value: u32) -> Self {
        if value == 0 {
            ReasoningBudget::Off
        } else if value <= 4000 {
            ReasoningBudget::Low
        } else if value <= 8000 {
            ReasoningBudget::Medium
        } else if value <= 16000 {
            ReasoningBudget::High
        } else {
            ReasoningBudget::Max
        }
    }
}

impl std::fmt::Display for ReasoningBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ReasoningBudget::Off => write!(f, "off"),
            ReasoningBudget::Low => write!(f, "low"),
            ReasoningBudget::Medium => write!(f, "medium"),
            ReasoningBudget::High => write!(f, "high"),
            ReasoningBudget::Max => write!(f, "max"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]

pub struct ModelSettings {
    pub model: Model,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub reasoning_budget: ReasoningBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: Content,
}

impl Message {
    pub fn new(role: MessageRole, content: Content) -> Self {
        Self { role, content }
    }

    pub fn user(content: impl Into<Content>) -> Self {
        Self::new(MessageRole::User, content.into())
    }

    pub fn assistant(content: impl Into<Content>) -> Self {
        Self::new(MessageRole::Assistant, content.into())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningData {
    pub text: String,
    pub signature: Option<String>,
    pub blob: Option<Vec<u8>>,
    pub raw_json: Option<Value>,
}

impl std::fmt::Display for ReasoningData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseData {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultData {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageData {
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentBlock {
    Text(String),
    ReasoningContent(ReasoningData),
    ToolUse(ToolUseData),
    ToolResult(ToolResultData),
    Image(ImageData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    blocks: Vec<ContentBlock>,
}

impl Content {
    pub fn new(blocks: Vec<ContentBlock>) -> Self {
        Self { blocks }
    }

    pub fn empty() -> Self {
        Self { blocks: Vec::new() }
    }

    pub fn text_only(text: String) -> Self {
        Self {
            blocks: vec![ContentBlock::Text(text.trim().to_string())],
        }
    }

    pub fn blocks(&self) -> &[ContentBlock] {
        &self.blocks
    }

    pub fn into_blocks(self) -> Vec<ContentBlock> {
        self.blocks
    }

    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn reasoning(&self) -> Vec<&ReasoningData> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ReasoningContent(reasoning) => Some(reasoning),
                _ => None,
            })
            .collect()
    }

    pub fn tool_uses(&self) -> Vec<&ToolUseData> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse(tool_use) => Some(tool_use),
                _ => None,
            })
            .collect()
    }

    pub fn tool_results(&self) -> Vec<&ToolResultData> {
        self.blocks()
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolResult(tool_result) => Some(tool_result),
                _ => None,
            })
            .collect()
    }

    pub fn images(&self) -> Vec<&ImageData> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Image(img) => Some(img),
                _ => None,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn push(&mut self, block: ContentBlock) {
        self.blocks.push(block);
    }

    pub fn extend(&mut self, blocks: Vec<ContentBlock>) {
        self.blocks.extend(blocks);
    }
}

impl From<Vec<ContentBlock>> for Content {
    fn from(blocks: Vec<ContentBlock>) -> Self {
        Self::new(blocks)
    }
}

impl From<ContentBlock> for Content {
    fn from(block: ContentBlock) -> Self {
        Self::new(vec![block])
    }
}

impl From<String> for Content {
    fn from(text: String) -> Self {
        Self::text_only(text)
    }
}

impl From<&str> for Content {
    fn from(text: &str) -> Self {
        Self::text_only(text.to_string())
    }
}

impl IntoIterator for Content {
    type Item = ContentBlock;
    type IntoIter = std::vec::IntoIter<ContentBlock>;

    fn into_iter(self) -> Self::IntoIter {
        self.blocks.into_iter()
    }
}

impl<'a> IntoIterator for &'a Content {
    type Item = &'a ContentBlock;
    type IntoIter = std::slice::Iter<'a, ContentBlock>;

    fn into_iter(self) -> Self::IntoIter {
        self.blocks.iter()
    }
}

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub provider: String,
    pub model_id: String,
    pub region: Option<String>,
}

/// Breakdown of context usage by category.
/// Byte sizes are measured before sending; actual input_tokens come from the API response.
/// Per-category token estimates are derived by applying byte proportions to actual input_tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBreakdown {
    pub context_window: u32,
    pub input_tokens: u32,
    pub system_prompt_bytes: usize,
    pub tool_definitions_bytes: usize,
    pub conversation_history_bytes: usize,
    pub reasoning_bytes: usize,
    pub context_injection_bytes: usize,
}

impl ContextBreakdown {
    pub fn utilization_percent(&self) -> f64 {
        if self.context_window == 0 {
            return 0.0;
        }
        (self.input_tokens as f64 / self.context_window as f64) * 100.0
    }

    fn total_bytes(&self) -> usize {
        self.system_prompt_bytes
            + self.tool_definitions_bytes
            + self.conversation_history_bytes
            + self.reasoning_bytes
            + self.context_injection_bytes
    }

    fn proportion(&self, bytes: usize) -> f64 {
        let total = self.total_bytes();
        if total == 0 {
            return 0.0;
        }
        bytes as f64 / total as f64
    }

    pub fn system_prompt_tokens(&self) -> u32 {
        (self.proportion(self.system_prompt_bytes) * self.input_tokens as f64) as u32
    }

    pub fn tool_definitions_tokens(&self) -> u32 {
        (self.proportion(self.tool_definitions_bytes) * self.input_tokens as f64) as u32
    }

    pub fn conversation_tokens(&self) -> u32 {
        (self.proportion(self.conversation_history_bytes) * self.input_tokens as f64) as u32
    }

    pub fn reasoning_tokens_estimate(&self) -> u32 {
        (self.proportion(self.reasoning_bytes) * self.input_tokens as f64) as u32
    }

    pub fn context_injection_tokens(&self) -> u32 {
        (self.proportion(self.context_injection_bytes) * self.input_tokens as f64) as u32
    }
}

#[derive(Debug, Clone)]
pub struct ConversationResponse {
    pub content: Content,
    pub usage: TokenUsage,
    pub stop_reason: StopReason,
}

/// Token usage reported by the AI provider, normalized to a consistent contract.
///
/// All providers MUST adhere to the following semantics:
/// - `input_tokens`: Non-cached input tokens billed at full rate.
/// - `output_tokens`: All output tokens INCLUDING reasoning. Reasoning is a subset, not an addition.
/// - `total_tokens`: `input_tokens + output_tokens`. No double-counting of reasoning.
/// - `cached_prompt_tokens`: Tokens served from prompt cache (still consume context window, billed at reduced rate).
/// - `cache_creation_input_tokens`: Tokens written to prompt cache on this request.
/// - `reasoning_tokens`: Subset of `output_tokens` used for chain-of-thought. Informational only for display/cost breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub cached_prompt_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence(String),
    ToolUse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl TokenUsage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            cached_prompt_tokens: None,
            cache_creation_input_tokens: None,
            reasoning_tokens: None,
        }
    }

    pub fn empty() -> Self {
        Self::new(0, 0)
    }
}

#[derive(Debug, Clone)]
pub struct Cost {
    pub input_cost_per_million_tokens: f64,
    pub output_cost_per_million_tokens: f64,
    pub cache_write_cost_per_million_tokens: f64,
    pub cache_read_cost_per_million_tokens: f64,
}

#[derive(Debug, Clone)]
pub struct ImageGenerationRequest {
    pub prompt: String,
    pub model_id: String,
    pub aspect_ratio: Option<String>,
    pub image_size: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImageGenerationResponse {
    pub image_data: Vec<u8>,
    pub media_type: String,
}

#[derive(Debug)]
pub enum StreamEvent {
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ContentBlockStart,
    ContentBlockStop,
    MessageComplete { response: ConversationResponse },
}

impl Cost {
    pub fn new(
        input_cost_per_million_tokens: f64,
        output_cost_per_million_tokens: f64,
        cache_write_cost_per_million_tokens: f64,
        cache_read_cost_per_million_tokens: f64,
    ) -> Self {
        Self {
            input_cost_per_million_tokens,
            output_cost_per_million_tokens,
            cache_write_cost_per_million_tokens,
            cache_read_cost_per_million_tokens,
        }
    }

    pub fn calculate_cost(&self, usage: &TokenUsage) -> f64 {
        let input_cost =
            (usage.input_tokens as f64 / 1_000_000.0) * self.input_cost_per_million_tokens;
        let output_cost =
            (usage.output_tokens as f64 / 1_000_000.0) * self.output_cost_per_million_tokens;
        let cache_write_cost = (usage.cache_creation_input_tokens.unwrap_or(0) as f64
            / 1_000_000.0)
            * self.cache_write_cost_per_million_tokens;
        let cache_read_cost = (usage.cached_prompt_tokens.unwrap_or(0) as f64 / 1_000_000.0)
            * self.cache_read_cost_per_million_tokens;
        input_cost + output_cost + cache_write_cost + cache_read_cost
    }
}
