use crate::ai::{model::Model, ReasoningData, TokenUsage, ToolUseData};
use crate::persistence::session::SessionMetadata;
use crate::tools::tasks::TaskList;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// `ChatEvent` are the messages sent from the actor - the output of the actor.
///
/// The actor is built with 2 channels - an input and output channel. Requests
/// are sent to the actor through the input channel. Requests may generate 1 or
/// move `ChatEvent`s in response which are sent to the output channel. Various
/// applications (CLI/VSCode/Tests) process chat events to implement their
/// application sepecific logic/rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum ChatEvent {
    MessageAdded(ChatMessage),
    Settings(serde_json::Value),
    TypingStatusChanged(bool),
    ConversationCleared,
    ToolRequest(ToolRequest),
    ToolExecutionCompleted {
        tool_call_id: String,
        tool_name: String,
        tool_result: ToolExecutionResult,
        success: bool,
        error: Option<String>,
    },
    OperationCancelled {
        message: String,
    },
    RetryAttempt {
        attempt: u32,
        max_retries: u32,
        error: String,
        backoff_ms: u64,
    },
    TaskUpdate(TaskList),
    SessionsList {
        sessions: Vec<SessionMetadata>,
    },
    ProfilesList {
        profiles: Vec<String>,
    },
    TimingUpdate {
        waiting_for_human: Duration,
        ai_processing: Duration,
        tool_execution: Duration,
    },
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub timestamp: u64,
    pub sender: MessageSender,
    pub content: String,
    pub reasoning: Option<ReasoningData>,
    pub tool_calls: Vec<ToolUseData>,
    pub model_info: Option<ModelInfo>,
    pub token_usage: Option<TokenUsage>,
}

impl ChatMessage {
    pub fn user(content: String) -> Self {
        Self {
            timestamp: Utc::now().timestamp_millis() as u64,
            sender: MessageSender::User,
            content,
            reasoning: None,
            tool_calls: vec![],
            model_info: None,
            token_usage: None,
        }
    }

    pub fn assistant(
        agent: String,
        content: String,
        tool_calls: Vec<ToolUseData>,
        model_info: ModelInfo,
        token_usage: TokenUsage,
        reasoning: Option<ReasoningData>,
    ) -> Self {
        Self {
            timestamp: Utc::now().timestamp_millis() as u64,
            sender: MessageSender::Assistant { agent },
            content,
            reasoning,
            tool_calls,
            model_info: Some(model_info),
            token_usage: Some(token_usage),
        }
    }

    pub fn system(content: String) -> Self {
        Self {
            timestamp: Utc::now().timestamp_millis() as u64,
            sender: MessageSender::System,
            content,
            reasoning: None,
            tool_calls: vec![],
            model_info: None,
            token_usage: None,
        }
    }

    pub fn warning(content: String) -> Self {
        Self {
            timestamp: Utc::now().timestamp_millis() as u64,
            sender: MessageSender::Warning,
            content,
            reasoning: None,
            tool_calls: vec![],
            model_info: None,
            token_usage: None,
        }
    }

    pub fn error(content: String) -> Self {
        Self {
            timestamp: Utc::now().timestamp_millis() as u64,
            sender: MessageSender::Error,
            content,
            reasoning: None,
            tool_calls: vec![],
            model_info: None,
            token_usage: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model: Model,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageSender {
    User,
    Assistant { agent: String },
    System,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    pub tool_call_id: String,
    pub tool_name: String,

    pub tool_type: ToolRequestType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ToolRequestType {
    ModifyFile {
        file_path: String,
        before: String,
        after: String,
    },
    RunCommand {
        command: String,
        working_directory: String,
    },
    ReadFiles {
        file_paths: Vec<String>,
    },
    Other {
        args: serde_json::Value,
    },
    SearchTypes {
        language: String,
        workspace_root: String,
        type_name: String,
    },
    GetTypeDocs {
        language: String,
        workspace_root: String,
        type_path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ToolExecutionResult {
    ModifyFile {
        lines_added: u32,
        lines_removed: u32,
    },
    RunCommand {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    ReadFiles {
        files: Vec<FileInfo>,
    },
    SearchTypes {
        types: Vec<String>,
    },
    GetTypeDocs {
        documentation: String,
    },
    Error {
        short_message: String,
        detailed_message: String,
    },
    Other {
        result: serde_json::Value,
    },
}

/// A small wrapper over the `event_tx` for convienance.
#[derive(Clone)]
pub struct EventSender {
    event_tx: mpsc::UnboundedSender<ChatEvent>,
    event_history: Arc<Mutex<Vec<ChatEvent>>>,
}

impl EventSender {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<ChatEvent>) {
        let (event_tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                event_tx,
                event_history: Arc::new(Mutex::new(Vec::new())),
            },
            rx,
        )
    }

    pub fn add_message(&self, message: ChatMessage) {
        let _ = self.event_tx.send(ChatEvent::MessageAdded(message));
    }

    pub fn set_typing(&self, typing: bool) {
        let _ = self.event_tx.send(ChatEvent::TypingStatusChanged(typing));
    }

    pub fn clear_conversation(&self) {
        let _ = self.event_tx.send(ChatEvent::ConversationCleared);
    }

    pub fn send(&self, event: ChatEvent) {
        self.event_history.lock().unwrap().push(event.clone());
        let _ = self.event_tx.send(event);
    }

    pub fn send_message(&self, message: ChatMessage) {
        let event = ChatEvent::MessageAdded(message);
        self.send(event);
    }

    pub fn send_replay(&self, event: ChatEvent) {
        let _ = self.event_tx.send(event);
    }

    pub fn event_history(&self) -> Vec<ChatEvent> {
        self.event_history.lock().unwrap().clone()
    }

    pub(crate) fn clear_history(&self) {
        self.event_history.lock().unwrap().clear();
    }
}
