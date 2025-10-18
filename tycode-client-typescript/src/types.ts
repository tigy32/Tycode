// Exact port of tycode-core/src/chat/events.rs, actor.rs, ai/types.rs, ai/model.rs

export type ChatEvent =
  | { kind: 'MessageAdded'; data: ChatMessage }
  | { kind: 'Settings'; data: any }
  | { kind: 'TypingStatusChanged'; data: boolean }
  | { kind: 'ConversationCleared' }
  | { kind: 'ToolRequest'; data: ToolRequest }
  | {
      kind: 'ToolExecutionCompleted';
      data: {
        tool_call_id: string;
        tool_name: string;
        tool_result: ToolExecutionResult;
        success: boolean;
        error?: string;
      };
    }
  | { kind: 'OperationCancelled'; data: { message: string } }
  | {
      kind: 'RetryAttempt';
      data: {
        attempt: number;
        max_retries: number;
        error: string;
        backoff_ms: number;
      };
    }
  | { kind: 'Error'; data: string };

export type ChatEventTag = ChatEvent['kind'];

export function getChatEventTag(event: ChatEvent): ChatEventTag {
  return event.kind;
}

export interface ChatMessage {
  timestamp: number;
  sender: MessageSender;
  content: string;
  reasoning?: ReasoningData;
  tool_calls: ToolUseData[];
  model_info?: ModelInfo;
  context_info?: ContextInfo;
  token_usage?: TokenUsage;
}

export interface ContextInfo {
  directory_list_bytes: number;
  files: FileInfo[];
}

export interface FileInfo {
  path: string;
  bytes: number;
}

export type Model =
  | 'claude-opus-4-1'
  | 'claude-opus-4'
  | 'claude-sonnet-4'
  | 'claude-sonnet-3-7'
  | 'gpt-oss-120b'
  | 'grok-code-fast-1'
  | 'None';

export interface ModelInfo {
  model: Model;
}

export type MessageSender =
  | 'User'
  | 'System'
  | 'Error'
  | { Assistant: { agent: string } };

export interface ReasoningData {
  text: string;
  signature?: string;
  blob?: number[]; // byte array
}

export interface ToolUseData {
  id: string;
  name: string;
  arguments: any;
}

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  cached_prompt_tokens?: number;
  cache_creation_input_tokens?: number;
  reasoning_tokens?: number;
}

export type ToolRequestType =
  | { kind: 'ModifyFile'; file_path: string; before: string; after: string }
  | { kind: 'RunCommand'; command: string; working_directory: string }
  | { kind: 'ReadFiles'; file_paths: string[] }
  | { kind: 'Other'; args: any };

export type ToolExecutionResult =
  | { kind: 'ModifyFile'; lines_added: number; lines_removed: number }
  | { kind: 'RunCommand'; exit_code: number; stdout: string; stderr: string }
  | { kind: 'ReadFiles'; files: FileInfo[] }
  | { kind: 'Error'; short_message: string; detailed_message: string }
  | { kind: 'Other'; result: any };

export interface ToolRequest {
  tool_call_id: string;
  tool_name: string;
  tool_type: ToolRequestType;
}

// Exact port from tycode-core/src/chat/actor.rs
export type ChatActorMessage =
  | { UserInput: string }
  | { ChangeProvider: string }
  | 'GetSettings'
  | { SaveSettings: { settings: any } };
