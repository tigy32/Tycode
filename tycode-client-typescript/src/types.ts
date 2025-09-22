// Exact port of tycode-core/src/chat/events.rs, actor.rs, ai/types.rs, ai/model.rs

export type ChatEvent =
  | { MessageAdded: ChatMessage }
  | { Settings: any }
  | { TypingStatusChanged: boolean }
  | 'ConversationCleared'
  | { ToolRequest: ToolRequest }
  | {
    ToolExecutionCompleted: {
      tool_name: string,
      success: boolean,
      result?: any,
      ui_data?: any,
      error?: string
    }
  }
  | { OperationCancelled: { message: string } }
  | {
    RetryAttempt: {
      attempt: number,
      max_retries: number,
      error: string,
      backoff_ms: number
    }
  }
  | { Error: string }

// Helper for exhaustiveness
export type ChatEventTag =
  | 'MessageAdded'
  | 'Settings'
  | 'TypingStatusChanged'
  | 'ConversationCleared'
  | 'ToolRequest'
  | 'ToolExecutionCompleted'
  | 'OperationCancelled'
  | 'RetryAttempt'
  | 'Error';

export function getChatEventTag(event: ChatEvent): ChatEventTag {
  if (typeof event === 'string') return 'ConversationCleared';
  if ('MessageAdded' in event) return 'MessageAdded';
  if ('Settings' in event) return 'Settings';
  if ('TypingStatusChanged' in event) return 'TypingStatusChanged';
  if ('ToolRequest' in event) return 'ToolRequest';
  if ('ToolExecutionCompleted' in event) return 'ToolExecutionCompleted';
  if ('OperationCancelled' in event) return 'OperationCancelled';
  if ('RetryAttempt' in event) return 'RetryAttempt';
  if ('Error' in event) return 'Error';
  throw new Error('Unknown event');
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

export type Model = 'claude-opus-4-1' | 'claude-opus-4' | 'claude-sonnet-4' | 'claude-sonnet-3-7' | 'gpt-oss-120b' | 'grok-code-fast-1' | 'None';

export interface ModelInfo {
  model: Model;
}

export type MessageSender = 'User' | 'System' | 'Error' | { Assistant: { agent: string } }

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
}

export type ToolRequestType = { ModifyFile: { file_path: string; before: string; after: string } } | { Other: { args: any } }

export interface ToolRequest {
  tool_name: string;
  arguments: any;
  tool_type: ToolRequestType;
}

// Exact port from tycode-core/src/chat/actor.rs
export type ChatActorMessage =
  | { UserInput: string }
  | { ChangeProvider: string }
  | 'GetSettings'
  | { SaveSettings: { settings: any } }