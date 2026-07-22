use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ai::{Content, ContentBlock, Message, MessageRole, ToolResultData, ToolUseData};
use crate::chat::events::{ChatEvent, ChatMessage, EventSender, ToolExecutionResult, ToolRequest};
use crate::spawn::AgentStack;

pub struct TurnProtocol {
    event_sender: EventSender,
    spawn_module: Arc<AgentStack>,
    stream_open: bool,
    emitted_tool_requests: HashMap<String, String>,
    completed_tool_requests: HashSet<String>,
    expected_tool_results: HashMap<String, String>,
    completed_tool_results: HashSet<String>,
    staged_tool_results: Vec<ContentBlock>,
    finished: bool,
}

impl TurnProtocol {
    pub fn new(event_sender: EventSender, spawn_module: Arc<AgentStack>) -> Self {
        Self {
            event_sender,
            spawn_module,
            stream_open: false,
            emitted_tool_requests: HashMap::new(),
            completed_tool_requests: HashSet::new(),
            expected_tool_results: HashMap::new(),
            completed_tool_results: HashSet::new(),
            staged_tool_results: Vec::new(),
            finished: false,
        }
    }

    pub fn finish(mut self) {
        self.finished = true;
    }

    pub fn send(&self, event: ChatEvent) {
        self.event_sender.send(event);
    }

    pub fn send_message(&self, message: ChatMessage) {
        self.event_sender.send_message(message);
    }

    pub fn stream_start(
        &mut self,
        message_id: String,
        agent: String,
        model: crate::ai::model::Model,
        model_version: String,
    ) {
        self.stream_open = true;
        self.event_sender
            .stream_start(message_id, agent, model, model_version);
    }

    pub fn stream_delta(&self, message_id: String, text: String) {
        self.event_sender.stream_delta(message_id, text);
    }

    pub fn stream_reasoning_delta(&self, message_id: String, text: String) {
        self.event_sender.stream_reasoning_delta(message_id, text);
    }

    pub fn stream_end(&mut self, message: ChatMessage) {
        self.stream_open = false;
        self.event_sender.stream_end(message);
    }

    pub fn register_tool_uses(&mut self, tool_uses: &[ToolUseData]) {
        for tool_use in tool_uses {
            self.expected_tool_results
                .insert(tool_use.id.clone(), tool_use.name.clone());
        }
    }

    pub fn tool_request(&mut self, request: ToolRequest) {
        self.emitted_tool_requests
            .insert(request.tool_call_id.clone(), request.tool_name.clone());
        self.event_sender.send(ChatEvent::ToolRequest(request));
    }

    pub fn tool_completed(
        &mut self,
        tool_call_id: &str,
        tool_name: &str,
        tool_result: ToolExecutionResult,
        success: bool,
        error: Option<String>,
    ) {
        self.completed_tool_requests
            .insert(tool_call_id.to_string());
        self.event_sender.send(ChatEvent::ToolExecutionCompleted {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            tool_result,
            success,
            error,
        });
    }

    pub fn stage_tool_result(&mut self, result: ContentBlock) {
        self.staged_tool_results.push(result);
    }

    pub fn append_tool_results_to_conversation(&mut self, results: Vec<ContentBlock>) {
        if results.is_empty() {
            return;
        }

        self.mark_tool_results_completed(&results);
        self.staged_tool_results.clear();
        self.push_tool_results(results);
    }

    fn mark_tool_results_completed(&mut self, results: &[ContentBlock]) {
        for result in results {
            if let ContentBlock::ToolResult(data) = result {
                self.completed_tool_results.insert(data.tool_use_id.clone());
            }
        }
    }

    fn push_tool_results(&self, results: Vec<ContentBlock>) {
        let content = Content::from(results);
        if self
            .spawn_module
            .with_current_agent_mut(|agent| {
                agent.conversation.push(Message {
                    role: MessageRole::User,
                    content,
                });
            })
            .is_none()
        {
            tracing::warn!("TurnProtocol could not append tool results: no active agent");
        }
    }

    fn abort(&mut self) {
        if self.stream_open {
            self.event_sender
                .stream_end(ChatMessage::error("Operation cancelled".to_string()));
            self.stream_open = false;
        }

        let cancellation_message = "Tool execution was cancelled by user".to_string();
        let pending_requests: Vec<(String, String)> = self
            .emitted_tool_requests
            .iter()
            .filter(|(id, _)| !self.completed_tool_requests.contains(*id))
            .map(|(id, name)| (id.clone(), name.clone()))
            .collect();

        for (tool_call_id, tool_name) in pending_requests {
            self.event_sender.send(ChatEvent::ToolExecutionCompleted {
                tool_call_id,
                tool_name,
                tool_result: ToolExecutionResult::Error {
                    short_message: "Cancelled".to_string(),
                    detailed_message: cancellation_message.clone(),
                },
                success: false,
                error: Some("Cancelled by user".to_string()),
            });
        }

        let mut results = self.staged_tool_results.clone();
        let staged_ids: HashSet<String> = results
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolResult(data) => Some(data.tool_use_id.clone()),
                _ => None,
            })
            .collect();

        for tool_use_id in self.expected_tool_results.keys() {
            if self.completed_tool_results.contains(tool_use_id) || staged_ids.contains(tool_use_id)
            {
                continue;
            }

            results.push(ContentBlock::ToolResult(ToolResultData {
                tool_use_id: tool_use_id.clone(),
                content: cancellation_message.clone(),
                is_error: true,
            }));
        }

        if !results.is_empty() {
            self.push_tool_results(results);
        }

        self.event_sender.send(ChatEvent::OperationCancelled {
            message: "Operation cancelled by user".to_string(),
        });
    }
}

impl Drop for TurnProtocol {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        self.abort();
    }
}
