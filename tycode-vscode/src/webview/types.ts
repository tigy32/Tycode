/// <reference lib="dom" />

/* eslint-disable @typescript-eslint/no-explicit-any */

import type { ImageData, ToolRequestType, ToolExecutionResult, SessionMetadata } from '../../lib/types';

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
    selectedProfile: string | null;
    isProcessing?: boolean;
    pendingToolUpdates?: Map<string, PendingToolUpdate>;
    pendingImages?: Array<ImageData & { name?: string }>;
    taskListState?: TaskListState;
    streamingElement?: HTMLDivElement;
    streamingText?: string;
    streamingReasoningText?: string;
    streamingAutoScroll?: boolean;
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
    tool_result: ToolExecutionResult;
    error?: string | null;
    diffId?: string | null;
};

export type ProfileConfigMessage = {
    type: 'profileConfig';
    conversationId: string;
    profiles: string[];
    selectedProfile: string | null;
};

export type ProfileSwitchedMessage = {
    type: 'profileSwitched';
    conversationId: string;
    newProfile: string;
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
    toolType: ToolRequestType;
    diffId?: string | null;
};

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

export type TaskUpdateMessage = {
    type: 'taskUpdate';
    conversationId: string;
    taskList: TaskList;
};

export type SessionsListUpdateMessage = {
    type: 'sessionsListUpdate';
    sessions: SessionMetadata[];
};

export type StreamStartMessage = {
    type: 'streamStart';
    conversationId: string;
    messageId: string;
    agent: string;
    model: string;
};

export type StreamDeltaMessage = {
    type: 'streamDelta';
    conversationId: string;
    messageId: string;
    text: string;
};

export type StreamReasoningDeltaMessage = {
    type: 'streamReasoningDelta';
    conversationId: string;
    messageId: string;
    text: string;
};

export type StreamEndMessage = {
    type: 'streamEnd';
    conversationId: string;
    message: any;
};

export type AddImageDataMessage = {
    type: 'addImageData';
    conversationId: string;
    media_type: string;
    data: string;
    name?: string;
};

export type PendingToolUpdate = {
    request?: ToolRequestMessage;
    result?: ToolResultMessage;
};

export interface TaskListState {
    title: string;
    tasks: Task[];
    isExpanded: boolean;
}

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
    | ProfileConfigMessage
    | ProfileSwitchedMessage
    | RetryAttemptMessage
    | ToolRequestMessage
    | TaskUpdateMessage
    | SessionsListUpdateMessage
    | SettingsUpdateMessage
    | StreamStartMessage
    | StreamDeltaMessage
    | StreamReasoningDeltaMessage
    | StreamEndMessage
    | AddImageDataMessage;

export type AutonomyLevel = 'fully_autonomous' | 'plan_approval_required';

export type SettingsUpdateMessage = {
    type: 'settingsUpdate';
    conversationId: string;
    autonomyLevel: AutonomyLevel;
    defaultAgent?: string;
    profile?: string;
};

export type WebviewMessageOutbound =
    | { type: 'newChat' }
    | { type: 'openSettings' }
    | { type: 'switchTab'; conversationId: string }
    | { type: 'closeTab'; conversationId: string }
    | { type: 'renameTab'; conversationId: string; title: string }
    | { type: 'clearChat'; conversationId: string }
    | { type: 'sendMessage'; conversationId: string; message: string; images?: ImageData[] }
    | { type: 'cancel'; conversationId: string }
    | { type: 'switchProfile'; conversationId: string; profile: string }
    | { type: 'getProfiles'; conversationId: string }
    | { type: 'refreshProfiles'; conversationId: string }
    | { type: 'copyCode'; code: string }
    | { type: 'insertCode'; code: string }
    | { type: 'viewDiff'; diffId: string }
    | { type: 'requestSessionsList' }
    | { type: 'resumeSession'; sessionId: string }
    | { type: 'setAutonomyLevel'; conversationId: string; autonomyLevel: AutonomyLevel }
    | { type: 'getSettings'; conversationId: string }
    | { type: 'imageDropped'; conversationId: string; uri: string };

export function assertUnreachable(value: never): never {
    throw new Error(`Unhandled case in exhaustive switch: ${JSON.stringify(value)}`);
}
