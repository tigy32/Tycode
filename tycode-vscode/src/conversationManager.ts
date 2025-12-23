import { EventEmitter } from 'events';
import { Conversation } from './conversation';
import * as vscode from 'vscode';
import { ChatEvent, CONVERSATION_EVENTS, MANAGER_EVENTS } from './events';
import { SessionData } from '../lib/types';
import { ChatActorClient } from '../lib/client';

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

    private setupConversationEventListeners(conversation: Conversation, id: string): void {
        conversation.on(CONVERSATION_EVENTS.CHAT_EVENT, (event: ChatEvent) => {
            this.emit(MANAGER_EVENTS.CHAT_EVENT, id, event);
        });

        conversation.on(CONVERSATION_EVENTS.TITLE_CHANGED, (newTitle) => {
            this.emit(MANAGER_EVENTS.CONVERSATION_TITLE_CHANGED, id, newTitle);
        });

        conversation.on(CONVERSATION_EVENTS.PROFILE_CHANGED, (profile) => {
            this.emit(MANAGER_EVENTS.CONVERSATION_PROFILE_CHANGED, id, profile);
        });

        conversation.on(CONVERSATION_EVENTS.PROFILE_SWITCHED, (oldProfile, newProfile) => {
            this.emit(MANAGER_EVENTS.CONVERSATION_PROFILE_SWITCHED, id, oldProfile, newProfile);
        });

        conversation.on(CONVERSATION_EVENTS.DISCONNECTED, () => {
            this.emit(MANAGER_EVENTS.CONVERSATION_DISCONNECTED, id);
        });
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

        this.setupConversationEventListeners(conversation, id);

        this.emit(MANAGER_EVENTS.CONVERSATION_CREATED, conversation);

        return conversation;
    }

    async loadSessions(): Promise<void> {
        const workspaceRoots = vscode.workspace.workspaceFolders?.map(f => f.uri.fsPath) || [];
        const client = new ChatActorClient(workspaceRoots);

        try {
            const sessionsData = await client.listSessions();
            const sessions = (sessionsData as any).sessions || sessionsData;

            const event: ChatEvent = {
                kind: 'SessionsList',
                data: { sessions }
            };

            this.emit(MANAGER_EVENTS.CHAT_EVENT, 'system', event);
        } finally {
            client.close();
        }
    }

    async resumeSession(sessionId: string): Promise<string> {
        const conversation = new Conversation(this.context, sessionId);
        conversation.title = 'Loading...';

        // Register immediately so tab appears right away
        this.conversations.set(sessionId, conversation);
        this.activeConversationId = sessionId;

        this.setupConversationEventListeners(conversation, sessionId);

        // Additional listener for resumed sessions: update title from TaskUpdate events
        conversation.on(CONVERSATION_EVENTS.CHAT_EVENT, (event: ChatEvent) => {
            if (event.kind !== 'TaskUpdate') return;
            
            const newTitle = event.data.title || 'Resumed Session';
            if (conversation.title === newTitle) return;
            
            conversation.title = newTitle;
            this.emit(MANAGER_EVENTS.CONVERSATION_TITLE_CHANGED, sessionId, newTitle);
        });

        // Emit CONVERSATION_CREATED immediately so UI shows the tab
        this.emit(MANAGER_EVENTS.CONVERSATION_CREATED, conversation);

        // Now do async operations - user already sees the tab
        await conversation.initialize();
        await conversation.client.resumeSession(sessionId);

        return sessionId;
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
