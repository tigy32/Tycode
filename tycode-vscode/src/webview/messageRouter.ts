import { ConversationController } from './conversationController.js';
import {
    WebviewMessageInbound,
    assertUnreachable
} from './types.js';

export function routeMessage(controller: ConversationController, message: WebviewMessageInbound): void {
    switch (message.type) {
        case 'initialState':
            controller.handleInitialState(message);
            return;
        case 'conversationCreated':
            controller.handleConversationCreated(message);
            return;
        case 'conversationMessage':
            controller.handleConversationMessage(message);
            return;
        case 'activeConversationChanged':
            controller.handleActiveConversationChanged(message);
            return;
        case 'conversationClosed':
            controller.handleConversationClosed(message);
            return;
        case 'conversationTitleChanged':
            controller.handleConversationTitleChanged(message);
            return;
        case 'conversationCleared':
            controller.handleConversationCleared(message);
            return;
        case 'showTyping':
            controller.handleShowTyping(message);
            return;
        case 'conversationDisconnected':
            controller.handleConversationDisconnected(message);
            return;
        case 'toolResult':
            controller.handleToolResult(message);
            return;
        case 'profileConfig':
            controller.handleProfileConfig(message);
            return;
        case 'profileSwitched':
            controller.handleProfileSwitched(message);
            return;
        case 'settingsUpdate':
            controller.handleSettingsUpdate(message);
            return;
        case 'retryAttempt':
            controller.handleRetryAttempt(message);
            return;
        case 'toolRequest':
            controller.handleToolRequest(message);
            return;
        case 'taskUpdate':
            controller.handleTaskUpdate(message);
            return;
        case 'sessionsListUpdate':
            controller.handleSessionsListUpdate(message.sessions);
            return;
        case 'streamStart':
            controller.handleStreamStart(message);
            return;
        case 'streamDelta':
            controller.handleStreamDelta(message);
            return;
        case 'streamEnd':
            controller.handleStreamEnd(message);
            return;
        default:
            assertUnreachable(message);
    }
}
