use crate::{
    driver::drive_conversation,
    fixture::{MessageCapturingReceiver, TestCase, TestResult},
};
use async_trait::async_trait;
use std::{path::PathBuf, process::Command};
use tycode_core::chat::ChatActor;

pub struct LeetCode21TestCase;

const MAX_MESSAGES: usize = 40;

#[async_trait]
impl TestCase for LeetCode21TestCase {
    fn directory(&self) -> String {
        "leetcode_21".to_string()
    }

    async fn execute(
        self,
        working_dir: PathBuf,
        mut actor: ChatActor,
        mut event_rx: MessageCapturingReceiver,
    ) -> TestResult {
        if let Err(e) = std::env::set_current_dir(&working_dir) {
            return TestResult {
                success: false,
                reason: format!("Failed to change directory: {e:?}"),
                actor,
                event_rx,
            };
        }

        if let Err(e) = actor.send_message(
            "Implement the merge_two_lists function in src/lib.rs to merge two sorted linked lists. \
             Modify the function to correctly merge the lists by splicing nodes together, handling all edge cases, \
             and ensuring the result is a single sorted list. You are running in an automated benchmark and are unable \
             to ask for approve, follow-up questions, or clarifications. Complete this task to the best of your ability
             and use the complete_task tool once finished."
                .to_string(),
        ) {
            return TestResult {
                success: false,
                reason: format!("Failed to send message: {e:?}"),
                actor,
                event_rx,
            };
        }

        if let Err(e) = drive_conversation(&mut actor, &mut event_rx, MAX_MESSAGES).await {
            return TestResult {
                success: false,
                reason: format!("Conversation failed: {e:?}"),
                actor,
                event_rx,
            };
        }

        // Run validation
        let output = match Command::new("cargo")
            .args(["test"])
            .current_dir(&working_dir)
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                return TestResult {
                    success: false,
                    reason: format!("Failed to execute command: {e:?}"),
                    actor,
                    event_rx,
                };
            }
        };

        let (success, reason) = if output.status.success() {
            println!(
                "Validation passed: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            (true, String::new())
        } else {
            println!(
                "Validation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (false, stderr)
        };

        TestResult {
            success,
            reason,
            actor,
            event_rx,
        }
    }
}
