/// <reference lib="dom" />

/* eslint-disable @typescript-eslint/no-explicit-any */

export interface VsCodeApi {
    postMessage(message: WebviewMessageOutbound): void;
    getState?(): unknown;
    setState?(state: unknown): void;
}

export interface ConversationState {
    title: string;
    messages: unknown[];
    tabElement: HTMLDivElement;
    viewElement: HTMLDivElement;
    selectedProvider: string | null;
    isProcessing?: boolean;
    pendingToolUpdates?: Map<string, PendingToolUpdate>;
}

export type InitialStateMessage = {
    type: 'initialState';
    conversations?: Array<{
        id: string;
        title: string;
        messages?: unknown[];
    }>;
    activeConversationId?: string | null;
};

export type ConversationCreatedMessage = {
    type: 'conversationCreated';
    id: string;
    title: string;
};

export type ConversationMessageMessage = {
    type: 'conversationMessage';
    conversationId: string;
    message: unknown;
};

export type ActiveConversationChangedMessage = {
    type: 'activeConversationChanged';
    id: string;
};

export type ConversationClosedMessage = {
    type: 'conversationClosed';
    id: string;
};

export type ConversationTitleChangedMessage = {
    type: 'conversationTitleChanged';
    id: string;
    title: string;
};

export type ConversationClearedMessage = {
    type: 'conversationCleared';
    conversationId: string;
};

export type ShowTypingMessage = {
    type: 'showTyping';
    conversationId: string;
    show: boolean;
};

export type ConversationDisconnectedMessage = {
    type: 'conversationDisconnected';
    id: string;
};

export type ToolResultMessage = {
    type: 'toolResult';
    conversationId: string;
    toolName: string;
    toolCallId: string;
    success: boolean;
    result?: Record<string, any> | null;
    error?: string | null;
    diffId?: string | null;
};

export type ProviderConfigMessage = {
    type: 'providerConfig';
    conversationId: string;
    providers: string[];
    selectedProvider: string | null;
};

export type ProviderSwitchedMessage = {
    type: 'providerSwitched';
    conversationId: string;
    newProvider: string;
};

export type RetryAttemptMessage = {
    type: 'retryAttempt';
    conversationId: string;
    attempt: number;
    maxRetries: number;
    error?: string | null;
    backoffMs: number;
};

export type ToolRequestMessage = {
    type: 'toolRequest';
    conversationId: string;
    toolName: string;
    toolCallId: string;
    arguments?: Record<string, any> | null;
    toolType?: { kind: string; [key: string]: any } | null;
    diffId?: string | null;
};

export type PendingToolUpdate = {
    request?: ToolRequestMessage;
    result?: ToolResultMessage;
};

export type WebviewMessageInbound =
    | InitialStateMessage
    | ConversationCreatedMessage
    | ConversationMessageMessage
    | ActiveConversationChangedMessage
    | ConversationClosedMessage
    | ConversationTitleChangedMessage
    | ConversationClearedMessage
    | ShowTypingMessage
    | ConversationDisconnectedMessage
    | ToolResultMessage
    | ProviderConfigMessage
    | ProviderSwitchedMessage
    | RetryAttemptMessage
    | ToolRequestMessage;

export type WebviewMessageOutbound =
    | { type: 'newChat' }
    | { type: 'openSettings' }
    | { type: 'switchTab'; conversationId: string }
    | { type: 'closeTab'; conversationId: string }
    | { type: 'renameTab'; conversationId: string; title: string }
    | { type: 'clearChat'; conversationId: string }
    | { type: 'sendMessage'; conversationId: string; message: string }
    | { type: 'cancel'; conversationId: string }
    | { type: 'switchProvider'; conversationId: string; provider: string }
    | { type: 'getProviders'; conversationId: string }
    | { type: 'refreshProviders'; conversationId: string }
    | { type: 'copyCode'; code: string }
    | { type: 'insertCode'; code: string }
    | { type: 'viewDiff'; diffId: string };

export function assertUnreachable(value: never): never {
    throw new Error(`Unhandled case in exhaustive switch: ${JSON.stringify(value)}`);
}
