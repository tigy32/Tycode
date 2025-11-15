pub mod get_type_docs;
pub mod search_types;

/// Supported languages for type analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    Rust,
}

impl SupportedLanguage {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" => Some(Self::Rust),
            _ => None,
        }
    }

    pub fn all() -> &'static [&'static str] {
        &["rust"]
    }
}
