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
  | { kind: 'TaskUpdate'; data: TaskList }
  | { kind: 'SessionsList'; data: { sessions: SessionMetadata[] } }
  | { kind: 'ProfilesList'; data: { profiles: string[] } }
  | { kind: 'ModuleSchemas'; data: { schemas: ModuleSchemaInfo[] } }
  | { kind: 'Orchestration'; data: OrchestrationEvent }
  | { kind: 'Error'; data: string }
  | { kind: 'StreamStart'; data: { message_id: string; agent: string; model: string } }
  | { kind: 'StreamDelta'; data: { message_id: string; text: string } }
  | { kind: 'StreamReasoningDelta'; data: { message_id: string; text: string } }
  | { kind: 'StreamEnd'; data: { message: ChatMessage } };

export interface ModuleSchemaInfo {
  namespace: string;
  schema: object;
}

// Exact port of tycode-core/src/orchestration/events.rs.
// Structured, machine-readable orchestration progress. Consumers must ignore
// unknown payload kinds so the set can grow. Rust Option<T> fields serialize
// as explicit null, so optional data is typed `T | null` here.

/**
 * Stable id for an agent instance, fan-out, or worker slot. Opaque; ids
 * minted by different subprocess runs never collide, so replayed session
 * events and live events can share one tree.
 */
export type AgentId = string;

export interface OrchestrationEvent {
  /** The agent this event describes; for fan-out/workflow events, the orchestrating on-stack agent. */
  agent_id: AgentId;
  /** Catalog name of that agent (e.g. "coder", "swarm"). */
  agent_type: string;
  payload: OrchestrationPayload;
}

export type OrchestrationPayload =
  | {
      kind: 'AgentStarted';
      /** Null only for root agents (origin "Root"). */
      parent_agent_id: AgentId | null;
      /** First line of the agent's task, truncated; full tasks are not carried on the stream. */
      task_preview: string;
      origin: AgentOrigin;
      /** Stack depth including this agent; the root agent is depth 1. */
      depth: number;
      /** True for on-stack agents that can receive user input; fan-out workers use Worker* events. */
      interactive: boolean;
      /** Model pinned by orchestration; null means per-agent settings selection at request time. */
      model: Model | null;
    }
  | { kind: 'AgentCompleted'; status: OutcomeStatus; result: string }
  | { kind: 'PhaseChanged'; phase: WorkflowPhase }
  | {
      kind: 'FanOutStarted';
      fanout_id: AgentId;
      total: number;
      concurrency: number;
      workers: WorkerInfo[];
    }
  | { kind: 'WorkerStarted'; fanout_id: AgentId; worker_id: AgentId; label: string }
  | {
      kind: 'WorkerCompleted';
      fanout_id: AgentId;
      worker_id: AgentId;
      label: string;
      status: OutcomeStatus;
      /**
       * Final worker report. Review/fix rounds inside a reviewed worker pair
       * are not individually surfaced; this summary carries the verdict text.
       */
      summary: string;
    }
  | { kind: 'FanOutCompleted'; fanout_id: AgentId; status: OutcomeStatus }
  | {
      kind: 'ConsensusRoundResolved';
      round: number;
      verdicts: PanelVerdict[];
      eliminated: CandidateInfo | null;
      remaining: CandidateInfo[];
    }
  | { kind: 'PlanSelected'; candidate: CandidateInfo | null }
  | { kind: 'ReviewRoundResolved'; round: number; verdict: ReviewVerdict; feedback: string };

export type AgentOrigin =
  | { kind: 'Tool'; tool_call_id: string }
  | { kind: 'Workflow' }
  | { kind: 'Root' };

/**
 * "Aborted" means the agent was discarded by an agent switch, conversation
 * reset, or session change. Note: cancelling a turn (OperationCancelled)
 * drops in-flight fan-out WITHOUT terminal worker events; treat it as
 * aborting everything started but not completed.
 */
export type OutcomeStatus = 'Succeeded' | 'Failed' | 'Aborted';

export type ReviewVerdict = 'Approved' | 'Rejected' | 'RoundLimitReached';

export interface WorkerInfo {
  worker_id: AgentId;
  label: string;
  agent_type: string;
  model: Model | null;
  /** Paired with a reviewer that must approve before the worker counts as successful. */
  reviewed: boolean;
  /** First line of the worker's task, truncated for display. */
  task_preview: string;
}

export interface CandidateInfo {
  label: string;
  /** The model that authored the candidate; the winning author implements. */
  author: Model | null;
}

export interface PanelVerdict {
  judge: Model | null;
  position: PanelPosition;
  worst_vote: CandidateInfo | null;
}

export type PanelPosition =
  | { kind: 'Endorsed'; candidate: CandidateInfo }
  | { kind: 'Revised' }
  | { kind: 'NoPosition' }
  | { kind: 'Failed' };

export type WorkflowPhase =
  | { kind: 'Reviewing'; round: number }
  | { kind: 'Fixing'; round: number }
  | { kind: 'BuilderPlanning' }
  | { kind: 'BuilderImplementing' }
  | { kind: 'BuilderReviewing'; round: number }
  | { kind: 'BuilderFixing'; round: number }
  | { kind: 'SwarmPlanning' }
  | { kind: 'SwarmPlanFanOut'; models: Model[] }
  | { kind: 'SwarmConsensus'; round: number; candidates: CandidateInfo[] }
  | { kind: 'SwarmImplementing'; fixer_model: Model | null }
  | { kind: 'SwarmFanOut'; model: Model | null }
  | { kind: 'SwarmIntegration'; round: number; models: Model[] }
  | { kind: 'SwarmFixing'; round: number };

export type ChatEventTag = ChatEvent['kind'];

export function getChatEventTag(event: ChatEvent): ChatEventTag {
  return event.kind;
}

export interface ContextBreakdown {
  system_prompt_bytes: number;
  tool_io_bytes: number;
  conversation_history_bytes: number;
  reasoning_bytes: number;
  context_injection_bytes: number;
  input_tokens: number;
  context_window: number;
}

export interface ChatMessage {
  timestamp: number;
  sender: MessageSender;
  content: string;
  reasoning?: ReasoningData;
  tool_calls: ToolUseData[];
  model_info?: ModelInfo;
  token_usage?: TokenUsage;
  context_breakdown?: ContextBreakdown;
  images?: ImageData[];
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
  | 'claude-fable'
  | 'claude-opus'
  | 'claude-opus-fast'
  | 'claude-sonnet'
  | 'claude-haiku'
  | 'gpt'
  | 'gpt-pro'
  | 'gpt-mini'
  | 'gpt-codex'
  | 'gpt-codex-max'
  | 'gpt-oss-120b'
  | 'gpt-oss-120b-free'
  | 'gemini-flash'
  | 'gemini-pro'
  | 'gemini-flash-lite'
  | 'kimi-k2'
  | 'kimi-k2-free'
  | 'qwen-max'
  | 'qwen-plus'
  | 'qwen-flash'
  | 'qwen-coder'
  | 'deepseek-pro'
  | 'deepseek-flash'
  | 'deepseek-flash-free'
  | 'glm'
  | 'minimax-m2'
  | 'grok'
  | 'grok-build'
  | 'ring'
  | 'step-flash'
  | 'openrouter/auto'
  | 'None';

export interface ModelInfo {
  model: Model;
}

export type MessageSender =
  | 'User'
  | 'System'
  | 'Warning'
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

export interface ImageData {
  media_type: string;
  data: string;
}

export type ToolRequestType =
  | { kind: 'ModifyFile'; file_path: string; before: string; after: string }
  | { kind: 'RunCommand'; command: string; working_directory: string }
  | { kind: 'ReadFiles'; file_paths: string[] }
  | { kind: 'SearchTypes'; language: string; workspace_root: string; type_name: string }
  | { kind: 'GetTypeDocs'; language: string; workspace_root: string; type_path: string }
  | { kind: 'Other'; args: any };

export type ToolExecutionResult =
  | { kind: 'ModifyFile'; lines_added: number; lines_removed: number }
  | { kind: 'RunCommand'; exit_code: number; stdout: string; stderr: string }
  | { kind: 'ReadFiles'; files: FileInfo[] }
  | { kind: 'SearchTypes'; types: string[] }
  | { kind: 'GetTypeDocs'; documentation: string }
  | { kind: 'Error'; short_message: string; detailed_message: string }
  | { kind: 'Other'; result: any };

export interface ToolRequest {
  tool_call_id: string;
  tool_name: string;
  tool_type: ToolRequestType;
}

export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'failed';

export interface Task {
  id: number;
  description: string;
  status: TaskStatus;
}

export interface TaskList {
  title: string;
  tasks: Task[];
}

export interface SessionMetadata {
  id: string;
  title: string;
  last_modified: number;
}

export interface SessionData {
  id: string;
  created_at: number;
  last_modified: number;
  messages: ChatMessage[];
  task_list: TaskList;
}

// Exact port from tycode-core/src/chat/actor.rs
export type ChatActorMessage =
  | { UserInput: string }
  | { UserInputWithImages: { text: string; images: ImageData[] } }
  | { ChangeProvider: string }
  | 'GetSettings'
  | { SaveSettings: { settings: any; persist: boolean } }
  | { SwitchProfile: { profile_name: string } }
  | { SaveProfile: { profile_name: string } }
  | 'ListProfiles'
  | 'ListSessions'
  | { ResumeSession: { session_id: string } }
  | 'GetModuleSchemas';
