use terminal_size::{terminal_size, Width};

pub struct BannerInfo {
    pub version: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub agent: String,
    pub workspace: String,
    pub memory_enabled: bool,
    pub memory_count: usize,
}

pub fn print_startup_banner(info: &BannerInfo) {
    let term_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(80);

    // Tiger art lines (fixed width for alignment)
    let tiger = [
        r"       /\_/\   ",
        r"      / o o \  ",
        r"     =\  ^  /= ",
        r"       )---(   ",
        r"      /|   |\  ",
        r"     (_|   |_) ",
        r"               ",
    ];

    // Info lines to display on the right
    let title = format!("\x1b[1;35mTycode\x1b[0m v{}", info.version);

    let provider_line = info.provider.as_ref()
        .map(|p| format!("\x1b[90mProvider:\x1b[0m  \x1b[32m{}\x1b[0m", p))
        .unwrap_or_default();

    let model_line = info.model.as_ref()
        .map(|m| format!("\x1b[90mModel:\x1b[0m     \x1b[36m{}\x1b[0m", m))
        .unwrap_or_default();

    let agent_line = format!("\x1b[90mAgent:\x1b[0m     \x1b[33m{}\x1b[0m", info.agent);

    let workspace = shorten_path(&info.workspace, term_width.saturating_sub(30));
    let workspace_line = format!("\x1b[90mWorkspace:\x1b[0m {}", workspace);

    let memory_line = if info.memory_enabled {
        format!("\x1b[90mMemory:\x1b[0m    \x1b[32menabled\x1b[0m ({} recent)", info.memory_count)
    } else {
        "\x1b[90mMemory:\x1b[0m    \x1b[90mdisabled\x1b[0m".to_string()
    };

    // Build info lines array
    let info_lines: [&str; 7] = [
        &title,
        "",
        &provider_line,
        &model_line,
        &agent_line,
        &workspace_line,
        &memory_line,
    ];

    // Print side by side
    println!();
    for (i, tiger_line) in tiger.iter().enumerate() {
        let info_line = info_lines.get(i).copied().unwrap_or("");
        println!("\x1b[33m{}\x1b[0m    {}", tiger_line, info_line);
    }
    println!();

    // Print helpful hints
    println!(
        "  \x1b[90m/help\x1b[0m commands  \x1b[90m/settings\x1b[0m config  \x1b[90m/quit\x1b[0m exit"
    );
    println!();
}

fn shorten_path(path: &str, max_len: usize) -> String {
    // Replace home dir with ~
    let home = std::env::var("HOME").unwrap_or_default();
    let path = if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };

    if path.len() <= max_len {
        path
    } else {
        format!("...{}", &path[path.len().saturating_sub(max_len - 3)..])
    }
}
