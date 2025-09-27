import { EventEmitter } from 'events';
import { Conversation } from './conversation';
import * as vscode from 'vscode';
import { ChatEvent, CONVERSATION_EVENTS, MANAGER_EVENTS } from './events';

export class ConversationManager extends EventEmitter {
    private conversations: Map<string, Conversation> = new Map();
    private activeConversationId: string | null = null;
    private availableProviders: { [name: string]: any } = {};
    private defaultProvider: string | null = null;

    constructor(private context: vscode.ExtensionContext) {
        super();
        // Provider settings will be loaded from subprocess when needed
        this.availableProviders = {};
        this.defaultProvider = null;
    }

    getAvailableProviders(): string[] {
        return Object.keys(this.availableProviders);
    }

    getDefaultProvider(): string | null {
        return this.defaultProvider;
    }

    async createConversation(title?: string, selectedProvider?: string): Promise<Conversation> {
        const id = this.generateId();
        // Don't default to anything - let the subprocess use its settings
        const conversation = new Conversation(this.context, id, title, selectedProvider);

        await conversation.initialize();

        this.conversations.set(id, conversation);
        this.activeConversationId = id;

        // Forward conversation events - now using raw ChatEvent objects
        conversation.on(CONVERSATION_EVENTS.CHAT_EVENT, (event: ChatEvent) => {
            this.emit(MANAGER_EVENTS.CHAT_EVENT, id, event);
        });

        conversation.on(CONVERSATION_EVENTS.TITLE_CHANGED, (newTitle) => {
            this.emit(MANAGER_EVENTS.CONVERSATION_TITLE_CHANGED, id, newTitle);
        });

        conversation.on(CONVERSATION_EVENTS.PROVIDER_CHANGED, (provider) => {
            this.emit(MANAGER_EVENTS.CONVERSATION_PROVIDER_CHANGED, id, provider);
        });

        conversation.on(CONVERSATION_EVENTS.PROVIDER_SWITCHED, (oldProvider, newProvider) => {
            this.emit(MANAGER_EVENTS.CONVERSATION_PROVIDER_SWITCHED, id, oldProvider, newProvider);
        });

        conversation.on(CONVERSATION_EVENTS.DISCONNECTED, () => {
            this.emit(MANAGER_EVENTS.CONVERSATION_DISCONNECTED, id);
        });

        this.emit(MANAGER_EVENTS.CONVERSATION_CREATED, conversation);

        return conversation;
    }

    getConversation(id: string): Conversation | undefined {
        return this.conversations.get(id);
    }

    getActiveConversation(): Conversation | undefined {
        return this.activeConversationId ? this.conversations.get(this.activeConversationId) : undefined;
    }

    setActiveConversation(id: string): boolean {
        if (this.conversations.has(id)) {
            this.activeConversationId = id;
            this.emit(MANAGER_EVENTS.ACTIVE_CONVERSATION_CHANGED, id);
            return true;
        }
        return false;
    }

    getAllConversations(): Conversation[] {
        return Array.from(this.conversations.values());
    }

    closeConversation(id: string): boolean {
        const conversation = this.conversations.get(id);
        if (conversation) {
            conversation.dispose();
            this.conversations.delete(id);

            // If this was the active conversation, clear it or switch to another
            if (this.activeConversationId === id) {
                const remaining = Array.from(this.conversations.keys());
                this.activeConversationId = remaining.length > 0 ? remaining[remaining.length - 1] : null;
                if (this.activeConversationId) {
                    this.emit(MANAGER_EVENTS.ACTIVE_CONVERSATION_CHANGED, this.activeConversationId);
                }
            }

            this.emit(MANAGER_EVENTS.CONVERSATION_CLOSED, id);
            return true;
        }
        return false;
    }

    closeAllConversations(): void {
        for (const conversation of this.conversations.values()) {
            conversation.dispose();
        }
        this.conversations.clear();
        this.activeConversationId = null;
        this.emit(MANAGER_EVENTS.ALL_CONVERSATIONS_CLOSED);
    }

    private generateId(): string {
        return Date.now().toString(36) + Math.random().toString(36).substr(2);
    }

    dispose(): void {
        this.closeAllConversations();
        this.removeAllListeners();
    }
}
