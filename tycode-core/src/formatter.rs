use crate::ai::TokenUsage;
use crate::chat::events::{ToolRequest, ToolRequestType};
use crate::chat::ModelInfo;
use serde_json::Value;
use similar::{ChangeTag, TextDiff};

#[derive(Clone)]
pub struct Formatter {
    use_colors: bool,
}

impl Default for Formatter {
    fn default() -> Self {
        Self::new()
    }
}

impl Formatter {
    pub fn new() -> Self {
        Self { use_colors: true }
    }

    pub fn print_system(&self, msg: &str) {
        if self.use_colors {
            println!("\x1b[33m[System]\x1b[0m {msg}");
        } else {
            println!("[System] {msg}");
        }
    }

    pub fn print_ai(
        &self,
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
            .map(|usage| format!(" (usage: {}/{})", usage.input_tokens, usage.output_tokens))
            .unwrap_or_default();

        if self.use_colors {
            println!(
                "\x1b[32m[{agent}]\x1b[0m \x1b[90m({model_name}){usage_text}\x1b[0m {msg}"
            );
        } else {
            println!("[{agent}] ({model_name}){usage_text} {msg}");
        }
    }

    pub fn print_error(&self, msg: &str) {
        if self.use_colors {
            eprintln!("\x1b[31m[Error]\x1b[0m {msg}");
        } else {
            eprintln!("[Error] {msg}");
        }
    }

    pub fn print_prompt(&self) -> String {
        if self.use_colors {
            "\x1b[35m>\x1b[0m ".to_string()
        } else {
            "> ".to_string()
        }
    }

    pub fn print_tool_call(&self, name: &str, arguments: &serde_json::Value) {
        if self.use_colors {
            println!("\x1b[36m🔧 Tool:\x1b[0m \x1b[1;36m{name}\x1b[0m \x1b[36mwith args:\x1b[0m \x1b[90m{arguments}\x1b[0m");
        } else {
            println!("🔧 Tool: {name} with args: {arguments}");
        }
    }

    pub fn print_formatted_tool_call(&self, name: &str, args: &Value) {
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

    pub fn print_tool_request(&self, tool_request: &ToolRequest) {
        match &tool_request.tool_type {
            ToolRequestType::ModifyFile {
                file_path,
                before,
                after,
            } => {
                self.print_system(&format!("📝 Modifying file {file_path}"));
                self.print_file_diff(before, after, self.use_colors);
            }
            ToolRequestType::Other { args } => {
                self.print_formatted_tool_call(&tool_request.tool_name, args);
            }
        }
    }

    fn print_file_diff(&self, before: &str, after: &str, use_colors: bool) {
        let diff = TextDiff::from_lines(before, after);
        let mut diff = diff.unified_diff();
        let unified = diff.context_radius(7);

        for hunk in unified.iter_hunks() {
            println!("{}", hunk.header());
            for change in hunk.iter_changes() {
                let line = change.value().trim_end_matches('\n');
                match change.tag() {
                    ChangeTag::Equal => println!(" {line}"),
                    ChangeTag::Delete => {
                        if use_colors {
                            println!("\x1b[91m-{line}\x1b[0m");
                        } else {
                            println!("-{line}");
                        }
                    }
                    ChangeTag::Insert => {
                        if use_colors {
                            println!("\x1b[92m+{line}\x1b[0m");
                        } else {
                            println!("+{line}");
                        }
                    }
                }
            }
        }
    }

    pub fn print_tool_result(
        &self,
        name: &str,
        success: bool,
        result: Option<&Value>,
        ui_data: Option<&Value>,
        error: Option<&str>,
    ) {
        if success {
            self.print_system(&format!("✅ {name} completed"));
            if let Some(res) = result {
                match name {
                    "write_file" => {
                        if let Some(bytes) = res.get("bytes_written").and_then(|v| v.as_u64()) {
                            let action = if res
                                .get("created")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                "created"
                            } else {
                                "updated"
                            };
                            self.print_system(&format!("  {action} file ({bytes} bytes)"));
                        }
                    }
                    "modify_file" => {
                        if let Some(reps) = res.get("replacements_made").and_then(|v| v.as_u64()) {
                            self.print_system(&format!("  {reps} replacements made"));
                        }
                    }
                    "search_files" => {
                        if let Some(count) = res.get("count").and_then(|v| v.as_u64()) {
                            self.print_system(&format!("  {count} matches"));
                        }
                    }

                    "run_build_test" => {
                        self.print_run_build_test_result(res);
                    }
                    _ => {
                        if let Ok(pretty) = serde_json::to_string_pretty(res) {
                            println!("  {}", pretty.replace("\n", "\n  "));
                        }
                    }
                }
            }
            if self.is_file_related(name) {
                if let Some(ui) = ui_data {
                    if let (Some(orig_str), Some(new_str)) = (
                        ui.get("original_content").and_then(|v| v.as_str()),
                        ui.get("new_content").and_then(|v| v.as_str()),
                    ) {
                        let (added, removed) = self.calculate_diff_summary(orig_str, new_str);
                        self.print_system(&format!("  {added} additions, {removed} deletions"));
                    }
                }
            }
        } else {
            let error_msg = if let Some(e) = error {
                e
            } else if let Some(r) = result {
                r.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
            } else {
                "unknown error"
            };
            self.print_error(&format!("❌ {name} failed: {error_msg}"));
        }
    }

    fn is_file_related(&self, name: &str) -> bool {
        matches!(name, "write_file" | "modify_file")
    }

    fn calculate_diff_summary(&self, orig: &str, new: &str) -> (usize, usize) {
        let orig_lines: Vec<&str> = orig.split('\n').collect();
        let new_lines: Vec<&str> = new.split('\n').collect();
        let max_len = orig_lines.len().max(new_lines.len());
        let mut added = 0;
        let mut removed = 0;
        for i in 0..max_len {
            let orig_line = orig_lines.get(i).copied().unwrap_or("");
            let new_line = new_lines.get(i).copied().unwrap_or("");
            if orig_line != new_line {
                if !orig_line.is_empty() {
                    removed += 1;
                }
                if !new_line.is_empty() {
                    added += 1;
                }
            }
        }
        (added, removed)
    }

    fn print_run_build_test_result(&self, res: &serde_json::Value) {
        let command = res
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let working_directory = res
            .get("working_directory")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        if self.use_colors {
            println!("  \x1b[32mCommand:\x1b[0m {command}");
            println!("  \x1b[32mWorking Directory:\x1b[0m {working_directory}");
            println!("  \x1b[32mStatus:\x1b[0m \x1b[32mSuccess\x1b[0m");
        } else {
            println!("  Command: {command}");
            println!("  Working Directory: {working_directory}");
            println!("  Status: Success");
        }
        if res
            .get("timed_out")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            if self.use_colors {
                println!("  \x1b[32mTimed Out:\x1b[0m \x1b[32mYes\x1b[0m");
            } else {
                println!("  Timed Out: Yes");
            }
        } else {
            let exit_code = res
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(-1);
            if self.use_colors {
                println!("  \x1b[32mExit Code:\x1b[0m \x1b[32m{exit_code}\x1b[0m");
            } else {
                println!("  Exit Code: {exit_code}");
            }
        }
        println!("  Stdout:");
        if let Some(stdout) = res.get("out").and_then(serde_json::Value::as_str) {
            for line in stdout.lines() {
                println!("    {line}");
            }
        }
        println!("  Stderr:");
        if let Some(stderr) = res.get("err").and_then(serde_json::Value::as_str) {
            for line in stderr.lines() {
                println!("    {line}");
            }
        }
    }
}
