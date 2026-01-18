use super::EventFormatter;
use crate::ai::TokenUsage;
use crate::chat::events::{ToolExecutionResult, ToolRequest, ToolRequestType};
use crate::chat::ModelInfo;
use crate::modules::task_list::{TaskList, TaskStatus};
use std::io::Write;

#[derive(Clone, Debug)]
pub enum MessageType {
    AI,
    System,
}

#[derive(Clone, Debug)]
pub struct LastMessage {
    pub content: String,
    pub message_type: MessageType,
    pub agent_name: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Clone)]
pub struct CompactFormatter {
    spinner_state: usize,
    typing_state: bool,
    last_message: Option<LastMessage>,
    thinking_shown: bool,
    last_tool_request: Option<ToolRequest>,
    terminal_width: usize,
    show_full_message: bool,
}

impl Default for CompactFormatter {
    fn default() -> Self {
        Self::new(80)
    }
}

impl CompactFormatter {
    pub fn new(terminal_width: usize) -> Self {
        Self {
            spinner_state: 0,
            typing_state: false,
            last_message: None,
            thinking_shown: false,
            last_tool_request: None,
            terminal_width,
            show_full_message: false,
        }
    }

    fn get_spinner_char(&mut self) -> char {
        const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let c = SPINNER_CHARS[self.spinner_state % SPINNER_CHARS.len()];
        self.spinner_state += 1;
        c
    }

    fn format_bytes_compact(bytes: usize) -> String {
        if bytes < 1024 {
            format!("{bytes}B")
        } else if bytes < 1024 * 1024 {
            format!("{}KB", bytes / 1024)
        } else {
            format!("{}MB", bytes / (1024 * 1024))
        }
    }

    fn format_token_usage_compact(usage: &TokenUsage) -> String {
        let input_k = (usage.input_tokens + usage.cache_creation_input_tokens.unwrap_or(0)) / 1000;
        let output_k = (usage.output_tokens + usage.reasoning_tokens.unwrap_or(0)) / 1000;
        format!("({input_k}k↑/{output_k}k↓)")
    }

    fn clear_thinking_if_shown(&mut self) {
        if self.thinking_shown {
            print!("\r\x1b[2K");
            self.thinking_shown = false;
        }
    }

    fn print_line(&mut self, line: &str) {
        self.clear_thinking_if_shown();
        println!("{line}");
    }

    fn eprint_line(&mut self, line: &str) {
        self.clear_thinking_if_shown();
        eprintln!("{line}");
    }

    fn print_compact_bullet(&self, text: &str) {
        print!("\r\x1b[2K  • {text}");
        let _ = std::io::stdout().flush();
    }

    fn finish_compact_bullet(&mut self, text: &str) {
        self.print_line(&format!("\r\x1b[2K  • {text}"));
    }

    fn shorten_path(path: &str) -> String {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() > 3 {
            format!(".../{}", parts[parts.len() - 1])
        } else {
            path.to_string()
        }
    }

    fn shorten_command(&self, cmd: &str) -> String {
        if cmd.len() > self.terminal_width {
            let truncate_at = self.terminal_width.saturating_sub(3);
            format!("{}...", &cmd[..truncate_at])
        } else {
            cmd.to_string()
        }
    }

    fn get_tool_display_text(&self, tool_request: &ToolRequest, spinner: char) -> String {
        match &tool_request.tool_type {
            ToolRequestType::ModifyFile { file_path, .. } => {
                format!(
                    "{} {} (patching...)",
                    spinner,
                    Self::shorten_path(file_path)
                )
            }
            ToolRequestType::RunCommand { command, .. } => {
                format!("{} {} (running...)", spinner, self.shorten_command(command))
            }
            ToolRequestType::ReadFiles { file_paths } => {
                if file_paths.is_empty() {
                    format!("{} reading files...", spinner)
                } else if file_paths.len() <= 3 {
                    let paths: Vec<String> =
                        file_paths.iter().map(|p| Self::shorten_path(p)).collect();
                    format!("{} reading {}...", spinner, paths.join(", "))
                } else {
                    let paths: Vec<String> = file_paths
                        .iter()
                        .take(3)
                        .map(|p| Self::shorten_path(p))
                        .collect();
                    format!(
                        "{} reading {}, +{} more...",
                        spinner,
                        paths.join(", "),
                        file_paths.len() - 3
                    )
                }
            }
            ToolRequestType::Other { .. } => {
                format!("{} {} (executing...)", spinner, tool_request.tool_name)
            }
            ToolRequestType::SearchTypes { type_name, .. } => {
                format!("{} Searching types: {}...", spinner, type_name)
            }
            ToolRequestType::GetTypeDocs { type_path, .. } => {
                format!("{} Getting docs: {}...", spinner, type_path)
            }
        }
    }

    fn finalize_last_message(&mut self) {
        if let Some(msg) = self.last_message.take() {
            let display_text = if self.show_full_message {
                msg.content.clone()
            } else {
                let first_line = msg.content.lines().next().unwrap_or(&msg.content);
                if first_line.chars().count() > self.terminal_width {
                    let truncate_at = self.terminal_width.saturating_sub(3);
                    let truncated_str: String = first_line.chars().take(truncate_at).collect();
                    format!("{}...", truncated_str)
                } else {
                    first_line.to_string()
                }
            };

            match msg.message_type {
                MessageType::AI => {
                    let agent = msg.agent_name.as_deref().unwrap_or("AI");
                    self.finish_compact_bullet(&format!("[{}] {}", agent, display_text));
                }
                MessageType::System => {
                    self.finish_compact_bullet(&format!("[System] {}", display_text));
                }
            }
        }
    }

    fn reprint_final_message(&mut self, msg: &LastMessage) {
        print!("\r\x1b[2K");
        match msg.message_type {
            MessageType::AI => {
                let usage_text = msg
                    .token_usage
                    .as_ref()
                    .map(Self::format_token_usage_compact)
                    .unwrap_or_default();
                let agent = msg.agent_name.as_deref().unwrap_or("AI");
                self.print_line(&format!(
                    "\x1b[32m[{agent}]\x1b[0m \x1b[90m{usage_text}\x1b[0m {}",
                    msg.content
                ));
            }
            MessageType::System => {
                self.print_line(&format!("\x1b[33m[System]\x1b[0m {}", msg.content));
            }
        }
    }
}

impl EventFormatter for CompactFormatter {
    fn print_system(&mut self, msg: &str) {
        if msg.contains("🔧") && msg.contains("tool call") {
            return;
        }

        if self.typing_state {
            self.finalize_last_message();

            let first_line = msg.lines().next().unwrap_or(msg);
            let truncated = if first_line.chars().count() > self.terminal_width {
                let truncate_at = self.terminal_width.saturating_sub(3);
                let truncated_str: String = first_line.chars().take(truncate_at).collect();
                format!("{}...", truncated_str)
            } else {
                first_line.to_string()
            };

            print!("\r\x1b[2K");
            self.print_compact_bullet(&format!("[System] {}", truncated));

            self.last_message = Some(LastMessage {
                content: msg.to_string(),
                message_type: MessageType::System,
                agent_name: None,
                token_usage: None,
            });
        } else {
            self.print_line(&format!("\x1b[33m[System]\x1b[0m {}", msg));
        }
    }

    fn print_ai(
        &mut self,
        msg: &str,
        agent: &str,
        _model_info: &Option<ModelInfo>,
        token_usage: &Option<TokenUsage>,
    ) {
        if self.typing_state {
            self.finalize_last_message();
            self.show_full_message = false;

            let first_line = msg.lines().next().unwrap_or(msg);
            let truncated = if first_line.chars().count() > self.terminal_width {
                let truncate_at = self.terminal_width.saturating_sub(3);
                let truncated_str: String = first_line.chars().take(truncate_at).collect();
                format!("{}...", truncated_str)
            } else {
                first_line.to_string()
            };

            print!("\r\x1b[2K");
            self.print_compact_bullet(&format!("[{}] {}", agent, truncated));

            self.last_message = Some(LastMessage {
                content: msg.to_string(),
                message_type: MessageType::AI,
                agent_name: Some(agent.to_string()),
                token_usage: token_usage.clone(),
            });
        } else {
            let usage_text = token_usage
                .as_ref()
                .map(Self::format_token_usage_compact)
                .unwrap_or_default();

            self.print_line(&format!(
                "\x1b[32m[{agent}]\x1b[0m \x1b[90m{usage_text}\x1b[0m {msg}"
            ));
        }
    }

    fn print_warning(&mut self, msg: &str) {
        self.clear_thinking_if_shown();
        print!("\r\x1b[2K");
        let _ = std::io::stdout().flush();
        self.eprint_line(&format!("\x1b[33m[Warning]\x1b[0m {msg}"));
    }

    fn print_error(&mut self, msg: &str) {
        self.clear_thinking_if_shown();
        print!("\r\x1b[2K");
        let _ = std::io::stdout().flush();
        self.eprint_line(&format!("\x1b[31m[Error]\x1b[0m {msg}"));
    }

    fn print_retry_attempt(&mut self, attempt: u32, max_retries: u32, error: &str) {
        let max_error_len = (self.terminal_width * 6 / 10).max(20);
        let error_preview = if error.len() > max_error_len {
            let truncate_at = max_error_len.saturating_sub(3);
            format!("{}...", &error[..truncate_at])
        } else {
            error.to_string()
        };
        self.print_compact_bullet(&format!(
            "⟳ Retry {}/{}: {}",
            attempt, max_retries, error_preview
        ));
    }

    fn print_tool_request(&mut self, tool_request: &ToolRequest) {
        self.last_tool_request = Some(tool_request.clone());
        if tool_request.tool_name == "complete_task"
            || tool_request.tool_name == "ask_user_question"
        {
            self.show_full_message = true;
        }
        let spinner = self.get_spinner_char();
        let text = self.get_tool_display_text(tool_request, spinner);
        self.print_compact_bullet(&text);
    }

    fn print_tool_result(
        &mut self,
        name: &str,
        success: bool,
        result: ToolExecutionResult,
        _verbose: bool,
    ) {
        if name == "complete_task" {
            self.last_tool_request = None;
            return;
        }

        let summary = match result {
            ToolExecutionResult::RunCommand {
                exit_code,
                stdout: _,
                stderr,
            } => {
                let cmd_context = self
                    .last_tool_request
                    .as_ref()
                    .and_then(|req| match &req.tool_type {
                        ToolRequestType::RunCommand { command, .. } => {
                            Some(self.shorten_command(command))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| name.to_string());

                if success {
                    format!("{} ✓ exit:{}", cmd_context, exit_code)
                } else {
                    let error_preview = if !stderr.is_empty() {
                        let first_line = stderr.lines().next().unwrap_or("");
                        let max_error_len = (self.terminal_width / 2).max(20);
                        if first_line.len() > max_error_len {
                            format!(" ({}...)", &first_line[..max_error_len])
                        } else {
                            format!(" ({})", first_line)
                        }
                    } else {
                        String::new()
                    };
                    format!("{} ✗ exit:{}{}", cmd_context, exit_code, error_preview)
                }
            }
            ToolExecutionResult::ReadFiles { ref files } => {
                let file_context =
                    self.last_tool_request
                        .as_ref()
                        .and_then(|req| match &req.tool_type {
                            ToolRequestType::ReadFiles { file_paths } if !file_paths.is_empty() => {
                                if file_paths.len() == 1 {
                                    Some(Self::shorten_path(&file_paths[0]))
                                } else if file_paths.len() <= 3 {
                                    let paths: Vec<String> =
                                        file_paths.iter().map(|p| Self::shorten_path(p)).collect();
                                    Some(paths.join(", "))
                                } else {
                                    let paths: Vec<String> = file_paths
                                        .iter()
                                        .take(2)
                                        .map(|p| Self::shorten_path(p))
                                        .collect();
                                    Some(format!(
                                        "{}, +{} more",
                                        paths.join(", "),
                                        file_paths.len() - 2
                                    ))
                                }
                            }
                            _ => None,
                        });

                let total_size = files.iter().map(|f| f.bytes).sum();
                if let Some(context) = file_context {
                    format!(
                        "{} ✓ {} files ({})",
                        context,
                        files.len(),
                        Self::format_bytes_compact(total_size)
                    )
                } else {
                    format!(
                        "{} ✓ {} files ({})",
                        name,
                        files.len(),
                        Self::format_bytes_compact(total_size)
                    )
                }
            }
            ToolExecutionResult::ModifyFile {
                lines_added,
                lines_removed,
            } => {
                let file_context = self
                    .last_tool_request
                    .as_ref()
                    .and_then(|req| match &req.tool_type {
                        ToolRequestType::ModifyFile { file_path, .. } => {
                            Some(Self::shorten_path(file_path))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| name.to_string());

                format!("{} ✓ +{} -{}", file_context, lines_added, lines_removed)
            }
            ToolExecutionResult::Error { short_message, .. } => {
                format!("{} ✗ {}", name, short_message)
            }
            ToolExecutionResult::SearchTypes { ref types } => {
                format!("{} ✓ {} types found", name, types.len())
            }
            ToolExecutionResult::GetTypeDocs { .. } => {
                format!("{} ✓ docs retrieved", name)
            }
            ToolExecutionResult::Other { .. } => {
                if success {
                    format!("{} ✓", name)
                } else {
                    format!("{} ✗", name)
                }
            }
        };
        self.finish_compact_bullet(&summary);
        self.last_tool_request = None;
    }

    fn print_thinking(&mut self) {
        if self.typing_state {
            let spinner = self.get_spinner_char();
            let text = if let Some(ref tool_request) = self.last_tool_request {
                self.get_tool_display_text(tool_request, spinner)
            } else {
                format!("{} Thinking...", spinner)
            };
            self.print_compact_bullet(&text);
            self.thinking_shown = true;
        }
    }

    fn print_task_update(&mut self, task_list: &TaskList) {
        // Find current InProgress task
        if let Some(current_task) = task_list
            .tasks
            .iter()
            .find(|t| matches!(t.status, TaskStatus::InProgress))
        {
            let completed = task_list
                .tasks
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::Completed))
                .count();
            let total = task_list.tasks.len();
            self.finish_compact_bullet(&format!(
                "Task {}/{}: {}",
                completed, total, current_task.description
            ));
        }
    }

    fn on_typing_status_changed(&mut self, typing: bool) {
        self.typing_state = typing;

        if !typing {
            if let Some(msg) = self.last_message.take() {
                self.reprint_final_message(&msg);
            }
            self.show_full_message = false;
        }
    }

    fn clone_box(&self) -> Box<dyn EventFormatter> {
        Box::new(self.clone())
    }
}
