// use crate::agents::code_review::CodeReviewAgent;

// use crate::settings::config::ReviewLevel;
// use crate::settings::manager::SettingsManager;
// use anyhow::{anyhow, Result};
// use serde_json::Value;

// pub struct ReviewService {
//     settings_manager: SettingsManager,
// }

// #[derive(Debug)]
// pub struct ReviewRequest {
//     pub file_path: String,
//     pub proposed_change: String,
//     pub change_type: ChangeType,
// }

// #[derive(Debug)]
// pub enum ChangeType {
//     Replace { search: String, replace: String },
//     Write { content: String },
// }

// #[derive(Debug)]
// pub struct ReviewResult {
//     pub approved: bool,
//     pub feedback: Option<String>,
// }

// impl ReviewService {
//     pub fn new(settings_manager: SettingsManager) -> Self {
//         Self { settings_manager }
//     }

//     /// Check if review is required for the given change type
//     pub fn requires_review(&self, change_type: &ChangeType) -> Result<bool> {
//         let settings = self.settings_manager.get_settings();

//         match settings.review_level {
//             ReviewLevel::None => Ok(false),
//             ReviewLevel::Modification => Ok(true),
//             ReviewLevel::All => Ok(true),
//         }
//     }

//     /// Review a proposed change using the CodeReviewAgent
//     pub async fn review_change(&self, request: ReviewRequest) -> Result<ReviewResult> {
//         if !self.requires_review(&request.change_type)? {
//             return Ok(ReviewResult {
//                 approved: true,
//                 feedback: None,
//             });
//         }

//         let review_prompt = self.build_review_prompt(&request)?;

//         let spawn_request = SpawnAgentRequest {
//             agent_name: "code_reviewer".to_string(),
//             initial_message: review_prompt,
//             coordinator_settings: CoordinatorSettings::default(),
//         };

//         // Execute the review agent
//         let result = crate::tools::spawn_agent::execute_spawn_agent(spawn_request).await?;

//         self.parse_review_result(&result)
//     }

//     fn build_review_prompt(&self, request: &ReviewRequest) -> Result<String> {
//         let change_description = match &request.change_type {
//             ChangeType::Replace { search, replace } => {
//                 format!(
//                     "File: {}\n\nProposed change (search/replace):\n\n--- SEARCH ---\n{}\n\n--- REPLACE ---\n{}\n",
//                     request.file_path, search, replace
//                 )
//             }
//             ChangeType::Write { content } => {
//                 format!(
//                     "File: {}\n\nProposed content (new file or complete overwrite):\n\n--- CONTENT ---\n{}\n",
//                     request.file_path, content
//                 )
//             }
//         };

//         Ok(format!(
//             "Please review this proposed code change against the Style Mandates:\n\n{}",
//             change_description
//         ))
//     }

//     fn parse_review_result(&self, agent_result: &AgentCommand) -> Result<ReviewResult> {
//         match agent_result {
//             AgentCommand::CompleteTask {
//                 success, summary, ..
//             } => Ok(ReviewResult {
//                 approved: *success,
//                 feedback: if *success {
//                     None
//                 } else {
//                     Some(summary.clone())
//                 },
//             }),
//             _ => Err(anyhow!(
//                 "Unexpected result from review agent: {:?}",
//                 agent_result
//             )),
//         }
//     }
// }
