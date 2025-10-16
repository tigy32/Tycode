use crate::{
    driver::drive_conversation,
    fixture::{MessageCapturingReceiver, TestCase, TestResult},
};
use async_trait::async_trait;
use std::{path::PathBuf, process::Command};
use tycode_core::chat::ChatActor;

pub struct ModifyFileStressTestCase;

const MAX_MESSAGES: usize = 20;

#[async_trait]
impl TestCase for ModifyFileStressTestCase {
    fn directory(&self) -> String {
        "modify_file_easy".to_string()
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

        // First switch to the file_writer agent
        if let Err(e) = actor.send_message("/agent file_writer".to_string()) {
            return TestResult {
                success: false,
                reason: format!("Failed to send agent switch command: {e:?}"),
                actor,
                event_rx,
            };
        }

        // Then send the actual task
        if let Err(e) = actor.send_message(
            "The file src/lib.rs contains many compilation errors including: mixed tabs/spaces indentation, \
             unresolved git merge conflict markers (<<<<<<<, =======, >>>>>>>), missing semicolons, mismatched brackets, \
             emoji in identifiers, and other syntax issues. Fix all compilation errors using the modify_file tool. \
             The code should logically make sense - you just need to fix the syntax and formatting issues. \
             Do not rewrite the entire file; make targeted fixes using modify_file. You are running in an automated benchmark \
             and are unable to ask for approve, follow-up questions, or clarifications. Complete this task to the best of your ability \
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

        // Run validation - just check if it compiles (no tests needed)
        let output = match Command::new("cargo")
            .args(["check"])
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
