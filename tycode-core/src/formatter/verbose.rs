use super::EventFormatter;
use crate::ai::model::Model;
use crate::ai::TokenUsage;
use crate::chat::events::{ChatMessage, ToolExecutionResult, ToolRequest, ToolRequestType};
use crate::chat::ModelInfo;
use crate::modules::task_list::{TaskList, TaskStatus};
use serde_json::Value;
use similar::{ChangeTag, TextDiff};
use std::io::Write;

#[derive(Clone)]
pub struct VerboseFormatter {
    use_colors: bool,
    spinner_state: usize,
    thinking_shown: bool,
    last_tool_request: Option<ToolRequest>,
}

impl Default for VerboseFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl VerboseFormatter {
    pub fn new() -> Self {
        Self {
            use_colors: true,
            spinner_state: 0,
            thinking_shown: false,
            last_tool_request: None,
        }
    }

    fn get_spinner_char(&mut self) -> char {
        const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let c = SPINNER_CHARS[self.spinner_state % SPINNER_CHARS.len()];
        self.spinner_state += 1;
        c
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

    fn print_tool_call(&mut self, name: &str, arguments: &serde_json::Value) {
        if self.use_colors {
            self.print_line(&format!("\x1b[36m🔧 Tool:\x1b[0m \x1b[1;36m{name}\x1b[0m \x1b[36mwith args:\x1b[0m \x1b[90m{arguments}\x1b[0m"));
        } else {
            self.print_line(&format!("🔧 Tool: {name} with args: {arguments}"));
        }
    }

    fn print_formatted_tool_call(&mut self, name: &str, args: &Value) {
        match name {
            "write_file" => {
                if let Some(path) = args.get("file_path").and_then(|v| v.as_str()) {
                    let content_len = args
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .len();
                    self.print_system(&format!("💾 Writing file {path} ({content_len} chars)"));
                } else {
                    self.print_tool_call(name, args);
                }
            }
            _ => {
                self.print_tool_call(name, args);
            }
        }
    }

    fn print_file_diff(&mut self, before: &str, after: &str, use_colors: bool) {
        let diff = TextDiff::from_lines(before, after);
        let mut diff = diff.unified_diff();
        let unified = diff.context_radius(7);

        for hunk in unified.iter_hunks() {
            self.print_line(&hunk.header().to_string());
            for change in hunk.iter_changes() {
                let line = change.value().trim_end_matches('\n');
                match change.tag() {
                    ChangeTag::Equal => self.print_line(&format!(" {line}")),
                    ChangeTag::Delete => {
                        if use_colors {
                            self.print_line(&format!("\x1b[91m-{line}\x1b[0m"));
                        } else {
                            self.print_line(&format!("-{line}"));
                        }
                    }
                    ChangeTag::Insert => {
                        if use_colors {
                            self.print_line(&format!("\x1b[92m+{line}\x1b[0m"));
                        } else {
                            self.print_line(&format!("+{line}"));
                        }
                    }
                }
            }
        }
    }

    fn format_bytes(&self, bytes: usize) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", bytes, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size, UNITS[unit_index])
        }
    }
}

impl EventFormatter for VerboseFormatter {
    fn print_system(&mut self, msg: &str) {
        if self.use_colors {
            self.print_line(&format!("\x1b[33m[System]\x1b[0m {msg}"));
        } else {
            self.print_line(&format!("[System] {msg}"));
        }
    }

    fn print_ai(
        &mut self,
        msg: &str,
        agent: &str,
        model_info: &Option<ModelInfo>,
        token_usage: &Option<TokenUsage>,
    ) {
        let model_name = model_info
            .as_ref()
            .map(|m| m.model.name())
            .unwrap_or_default();

        let usage_text = token_usage
            .as_ref()
            .map(|usage| {
                let display_input =
                    usage.input_tokens + usage.cache_creation_input_tokens.unwrap_or(0);
                let input_part = if let Some(cached) = usage.cached_prompt_tokens {
                    if cached > 0 {
                        format!("{} ({} cached)", display_input, cached)
                    } else {
                        format!("{}", display_input)
                    }
                } else {
                    format!("{}", display_input)
                };

                let display_output = usage.output_tokens + usage.reasoning_tokens.unwrap_or(0);
                let output_part = if let Some(reasoning) = usage.reasoning_tokens {
                    if reasoning > 0 {
                        format!("{} ({} reasoning)", display_output, reasoning)
                    } else {
                        format!("{}", display_output)
                    }
                } else {
                    format!("{}", display_output)
                };

                format!(" (usage: {}/{})", input_part, output_part)
            })
            .unwrap_or_default();

        if self.use_colors {
            self.print_line(&format!(
                "\x1b[32m[{agent}]\x1b[0m \x1b[90m({model_name}){usage_text}\x1b[0m {msg}"
            ));
        } else {
            self.print_line(&format!("[{agent}] ({model_name}){usage_text} {msg}"));
        }
    }

    fn print_warning(&mut self, msg: &str) {
        if self.use_colors {
            self.eprint_line(&format!("\x1b[33m[Warning]\x1b[0m {msg}"));
        } else {
            self.eprint_line(&format!("[Warning] {msg}"));
        }
    }

    fn print_error(&mut self, msg: &str) {
        if self.use_colors {
            self.eprint_line(&format!("\x1b[31m[Error]\x1b[0m {msg}"));
        } else {
            self.eprint_line(&format!("[Error] {msg}"));
        }
    }

    fn print_retry_attempt(&mut self, attempt: u32, max_retries: u32, error: &str) {
        if self.use_colors {
            self.print_line(&format!(
                "\x1b[33m🔄 Retry attempt {}/{}: {}\x1b[0m",
                attempt, max_retries, error
            ));
        } else {
            self.print_line(&format!(
                "🔄 Retry attempt {}/{}: {}",
                attempt, max_retries, error
            ));
        }
    }

    fn print_tool_request(&mut self, tool_request: &ToolRequest) {
        self.last_tool_request = Some(tool_request.clone());
        match &tool_request.tool_type {
            ToolRequestType::ModifyFile {
                file_path,
                before,
                after,
            } => {
                self.print_system(&format!("📝 Modifying file {file_path}"));
                self.print_file_diff(before, after, self.use_colors);
            }
            ToolRequestType::RunCommand {
                command,
                working_directory,
            } => self.print_system(&format!(
                "💻 Running command `{command}` (in directory {working_directory})"
            )),
            ToolRequestType::ReadFiles { .. } => (), //handled in execute result
            ToolRequestType::Other { args } => {
                self.print_formatted_tool_call(&tool_request.tool_name, args);
            }
            ToolRequestType::SearchTypes {
                type_name,
                workspace_root,
                ..
            } => {
                self.print_system(&format!(
                    "🔍 Searching types for '{type_name}' in {workspace_root}"
                ));
            }
            ToolRequestType::GetTypeDocs {
                type_path,
                workspace_root,
                ..
            } => {
                self.print_system(&format!(
                    "📚 Getting docs for '{type_path}' in {workspace_root}"
                ));
            }
        }
    }

    fn print_tool_result(
        &mut self,
        name: &str,
        success: bool,
        result: ToolExecutionResult,
        verbose: bool,
    ) {
        if success {
            self.print_system(&format!("✅ {name} completed"));
        }

        // This is an awful hack to not have complete a task show up with a json
        // string in addition to the system emssage the actor sends. This should
        // be a strongly typed tool result and have UI specific rendoring...
        if name == "complete_task" {
            self.last_tool_request = None;
            return;
        }

        match result {
            ToolExecutionResult::RunCommand {
                exit_code,
                stdout,
                stderr,
            } => {
                let status = if exit_code == 0 {
                    if self.use_colors {
                        "\x1b[32mSuccess\x1b[0m"
                    } else {
                        "Success"
                    }
                } else {
                    if self.use_colors {
                        "\x1b[31mFailed\x1b[0m"
                    } else {
                        "Failed"
                    }
                };

                self.print_system(&format!("💻 Command completed with status: {status}"));
                if self.use_colors {
                    self.print_line(&format!("  \x1b[36mExit Code:\x1b[0m {exit_code}"));
                } else {
                    self.print_line(&format!("  Exit Code: {exit_code}"));
                }

                if !stdout.is_empty() {
                    if self.use_colors {
                        self.print_line("  \x1b[32mStdout:\x1b[0m");
                    } else {
                        self.print_line("  Stdout:");
                    }
                    for line in stdout.lines() {
                        self.print_line(&format!("    {line}"));
                    }
                }

                if !stderr.is_empty() {
                    if self.use_colors {
                        self.print_line("  \x1b[31mStderr:\x1b[0m");
                    } else {
                        self.print_line("  Stderr:");
                    }
                    for line in stderr.lines() {
                        self.print_line(&format!("    {line}"));
                    }
                }
            }
            ToolExecutionResult::ReadFiles { files } => {
                self.print_system(&format!("📁 Tracked {} files", files.len()));
                for file in files {
                    let formatted_size = self.format_bytes(file.bytes);
                    self.print_system(&format!("  📁 {} ({})", file.path, formatted_size));
                }
            }
            ToolExecutionResult::ModifyFile {
                lines_added,
                lines_removed,
            } => {
                self.print_system(&format!(
                    "📝 File modified: {lines_added} additions, {lines_removed} deletions"
                ));
            }
            ToolExecutionResult::Error {
                short_message,
                detailed_message,
            } => {
                let message = if verbose {
                    detailed_message
                } else {
                    short_message
                };
                self.print_error(&format!("❌ Tool failed: {message}"));
            }
            ToolExecutionResult::SearchTypes { types } => {
                self.print_system(&format!("🔍 Found {} types", types.len()));
                for type_path in types {
                    self.print_line(&format!("  📦 {}", type_path));
                }
            }
            ToolExecutionResult::GetTypeDocs { documentation } => {
                self.print_system("📚 Documentation retrieved");
                for line in documentation.lines().take(20) {
                    self.print_line(&format!("  {}", line));
                }
                if documentation.lines().count() > 20 {
                    self.print_line("  ...(truncated)");
                }
            }
            ToolExecutionResult::Other { result } => {
                if let Ok(pretty) = serde_json::to_string_pretty(&result) {
                    self.print_line(&format!("  {}", pretty.replace("\n", "\n  ")));
                }
            }
        }
        self.last_tool_request = None;
    }

    fn print_thinking(&mut self) {
        let spinner = self.get_spinner_char();
        let text = if let Some(ref tool_request) = self.last_tool_request {
            match &tool_request.tool_type {
                ToolRequestType::ModifyFile { file_path, .. } => {
                    format!("Modifying {}...", file_path)
                }
                ToolRequestType::RunCommand { command, .. } => {
                    format!("Running `{}`...", command)
                }
                ToolRequestType::ReadFiles { file_paths } => {
                    if file_paths.is_empty() {
                        "Reading files...".to_string()
                    } else if file_paths.len() == 1 {
                        format!("Reading {}...", file_paths[0])
                    } else {
                        format!("Reading {} files...", file_paths.len())
                    }
                }
                ToolRequestType::Other { .. } => {
                    format!("Executing {}...", tool_request.tool_name)
                }
                ToolRequestType::SearchTypes { type_name, .. } => {
                    format!("Searching types for '{}'...", type_name)
                }
                ToolRequestType::GetTypeDocs { type_path, .. } => {
                    format!("Getting docs for '{}'...", type_path)
                }
            }
        } else {
            "Thinking...".to_string()
        };

        if self.use_colors {
            print!("\r\x1b[2K\x1b[36m{} {}\x1b[0m", spinner, text);
        } else {
            print!("\r{} {}", spinner, text);
        }
        let _ = std::io::stdout().flush();
        self.thinking_shown = true;
    }

    fn print_stream_start(&mut self, _message_id: &str, agent: &str, model: &Model) {
        self.clear_thinking_if_shown();
        let model_name = model.name();
        if self.use_colors {
            print!("\r\x1b[2K\x1b[32m[{agent}]\x1b[0m \x1b[90m({model_name})\x1b[0m ");
        } else {
            print!("\r[{agent}] ({model_name}) ");
        }
        let _ = std::io::stdout().flush();
    }

    fn print_stream_delta(&mut self, _message_id: &str, text: &str) {
        print!("{text}");
        let _ = std::io::stdout().flush();
    }

    fn print_stream_end(&mut self, message: &ChatMessage) {
        println!();
        if let Some(ref usage) = message.token_usage {
            let display_input = usage.input_tokens + usage.cache_creation_input_tokens.unwrap_or(0);
            let display_output = usage.output_tokens + usage.reasoning_tokens.unwrap_or(0);
            let usage_text = format!("(usage: {}/{})", display_input, display_output);
            if self.use_colors {
                self.print_line(&format!("  \x1b[90m{usage_text}\x1b[0m"));
            } else {
                self.print_line(&format!("  {usage_text}"));
            }
        }
    }

    fn print_task_update(&mut self, task_list: &TaskList) {
        self.print_system("Task List:");
        for task in &task_list.tasks {
            let (status_text, color_code) = match task.status {
                TaskStatus::Pending => ("Pending", "\x1b[37m"),
                TaskStatus::InProgress => ("InProgress", "\x1b[33m"),
                TaskStatus::Completed => ("Completed", "\x1b[32m"),
                TaskStatus::Failed => ("Failed", "\x1b[31m"),
            };
            let status_display = format!("{color_code}[{status_text}]\x1b[0m");
            self.print_system(&format!(
                "  - {} Task {}: {}",
                status_display, task.id, task.description
            ));
        }
    }

    fn clone_box(&self) -> Box<dyn EventFormatter> {
        Box::new(self.clone())
    }
}
