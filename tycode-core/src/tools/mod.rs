pub mod ask_user_question;
pub mod complete_task;
pub mod fuzzy_json;
pub mod registry;
pub mod spawn;
pub mod r#trait;

#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ToolName(String);

impl ToolName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
