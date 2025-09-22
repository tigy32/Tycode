// Re-export types from tycode-client-typescript
export type {
    ChatEvent,
    ChatMessage,
    ChatActorMessage,
    ContextInfo,
    FileInfo,
    Model,
    ModelInfo,
    MessageSender,
    ReasoningData,
    ToolUseData,
    TokenUsage,
    ChatEventTag,
    ToolRequest
} from '../lib/types';

export {
  getChatEventTag
} from '../lib/types';

// ConversationMessage interface removed - using ChatMessage directly

// Settings structure
export interface Settings {
    providers?: {
        [key: string]: any;
    };
    active_provider?: string;
    [key: string]: any;
}

// Event constants for conversation management
export const CONVERSATION_EVENTS = {
    MESSAGE_ADDED: 'messageAdded',
    ERROR: 'error',
    TOOL_EXECUTION_COMPLETED: 'toolExecutionCompleted',
    TOOL_REQUEST: 'toolRequest',
    SETTINGS: 'settings',
    TYPING_STATUS: 'typingStatus',
    RETRY_ATTEMPT: 'retryAttempt',
    OPERATION_CANCELLED: 'operationCancelled',
    TITLE_CHANGED: 'titleChanged',
    PROVIDER_CHANGED: 'providerChanged',
    PROVIDER_SWITCHED: 'providerSwitched',
    DISCONNECTED: 'disconnected',
    CLEARED: 'cleared'
} as const;

// Event constants for conversation manager
export const MANAGER_EVENTS = {
    CONVERSATION_CREATED: 'conversationCreated',
    CONVERSATION_UPDATE: 'conversationUpdate',
    CONVERSATION_TITLE_CHANGED: 'conversationTitleChanged',
    CONVERSATION_PROVIDER_CHANGED: 'conversationProviderChanged',
    CONVERSATION_PROVIDER_SWITCHED: 'conversationProviderSwitched',
    CONVERSATION_DISCONNECTED: 'conversationDisconnected',
    ACTIVE_CONVERSATION_CHANGED: 'activeConversationChanged',
    CONVERSATION_CLOSED: 'conversationClosed',
    ALL_CONVERSATIONS_CLOSED: 'allConversationsClosed'
} as const;