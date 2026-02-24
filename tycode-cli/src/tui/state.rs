use tycode_core::ai::types::TokenUsage;
use tycode_core::modules::task_list::TaskList;

use crate::state::State;

/// A single entry in the chat history panel.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum ChatEntry {
    UserMessage {
        content: String,
    },
    AssistantMessage {
        agent: String,
        model: String,
        content: String,
        token_usage: Option<TokenUsage>,
    },
    /// An assistant message currently being streamed token-by-token.
    StreamingMessage {
        agent: String,
        model: String,
        content: String,
    },
    SystemMessage {
        content: String,
    },
    WarningMessage {
        content: String,
    },
    ErrorMessage {
        content: String,
    },
    ToolRequest {
        tool_name: String,
        summary: String,
    },
    ToolResult {
        tool_name: String,
        success: bool,
        summary: String,
    },
    TaskUpdate {
        task_list: TaskList,
    },
}

/// Data for the startup banner displayed in the chat panel.
pub struct BannerData {
    pub version: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub agent: String,
    pub workspace: String,
    pub memory_enabled: bool,
    pub memory_count: usize,
}

pub struct TuiState {
    /// Ordered list of chat entries, newest last.
    pub chat_history: Vec<ChatEntry>,

    /// Scroll offset from the bottom (0 = fully scrolled down).
    pub scroll_offset: u16,

    /// Maximum scroll offset (computed during render).
    pub max_scroll: u16,

    /// Whether auto-scroll is active (true when user is at the bottom).
    pub auto_scroll: bool,

    /// Whether the AI is currently processing.
    pub is_thinking: bool,

    /// Text shown as the thinking status in the status bar.
    pub thinking_text: String,

    /// Spinner animation frame counter.
    pub spinner_frame: usize,

    /// Current agent name for the status bar.
    pub current_agent: String,

    /// Current model name for the status bar.
    pub current_model: String,

    /// Session-level token usage for the status bar.
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,

    /// Current task list (if any).
    pub current_tasks: Option<TaskList>,

    /// Inner state from the existing State struct (show_reasoning, show_timing).
    pub inner_state: State,

    /// Whether the app should exit.
    pub should_quit: bool,

    /// Whether we are currently waiting for a response.
    pub awaiting_response: bool,

    /// Banner info for initial display.
    pub banner_data: Option<BannerData>,
}

impl TuiState {
    pub fn new(banner_data: Option<BannerData>) -> Self {
        let current_agent = banner_data
            .as_ref()
            .map(|b| b.agent.clone())
            .unwrap_or_else(|| "tycode".to_string());
        let current_model = banner_data
            .as_ref()
            .and_then(|b| b.model.clone())
            .unwrap_or_default();

        Self {
            chat_history: Vec::new(),
            scroll_offset: 0,
            max_scroll: 0,
            auto_scroll: true,
            is_thinking: false,
            thinking_text: String::new(),
            spinner_frame: 0,
            current_agent,
            current_model,
            total_input_tokens: 0,
            total_output_tokens: 0,
            current_tasks: None,
            inner_state: State::default(),
            should_quit: false,
            awaiting_response: false,
            banner_data,
        }
    }

    /// Append an entry and maintain auto-scroll.
    pub fn push_entry(&mut self, entry: ChatEntry) {
        self.chat_history.push(entry);
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    /// Accumulate token usage from a response.
    pub fn accumulate_tokens(&mut self, usage: &TokenUsage) {
        self.total_input_tokens += usage.input_tokens
            + usage.cache_creation_input_tokens.unwrap_or(0);
        self.total_output_tokens += usage.output_tokens + usage.reasoning_tokens.unwrap_or(0);
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(amount)
            .min(self.max_scroll);
        self.auto_scroll = false;
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        if self.scroll_offset == 0 {
            self.auto_scroll = true;
        }
    }
}
