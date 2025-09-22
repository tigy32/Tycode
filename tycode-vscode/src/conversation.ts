import { EventEmitter } from 'events';
import { ChatActorClient } from '../lib/client';
import * as vscode from 'vscode';
import {
    MessageSender,
    ChatEvent,
    ChatMessage,
    ToolRequest,
    CONVERSATION_EVENTS,
    getChatEventTag
} from './events';

export class Conversation extends EventEmitter {
    public client: ChatActorClient;
    private _id: string;
    private _title: string;

    private _isActive: boolean = false;
    private _isManuallyNamed: boolean = false;
    private _hasFirstMessage: boolean = false;
    private _selectedProvider: string | undefined;
    private eventConsumer: Promise<void> | null = null;
    private shouldStop: boolean = false;

    constructor(
        private context: vscode.ExtensionContext,
        id: string,
        title?: string,
        selectedProvider?: string
    ) {
        super();
        this._id = id;
        this._title = title || 'New Chat';
        this._isManuallyNamed = !!title;
        this._selectedProvider = selectedProvider;

        // Get workspace roots for the client
        const workspaceFolders = vscode.workspace.workspaceFolders;
        const workspaceRoots = workspaceFolders ? workspaceFolders.map(f => f.uri.fsPath) : [];

        console.log('[Conversation] Creating ChatActorClient with workspaceRoots:', workspaceRoots);
        console.log('[Conversation] Using default settings path (~/.tycode/settings.toml)');

        // Use default settings path (~/.tycode/settings.toml)
        this.client = new ChatActorClient(workspaceRoots);
    }

    get id(): string {
        return this._id;
    }

    get title(): string {
        return this._title;
    }

    set title(value: string) {
        this._title = value;
        this._isManuallyNamed = true;  // Mark as manually named when user sets title
        this.emit(CONVERSATION_EVENTS.TITLE_CHANGED, value);
    }



    get isActive(): boolean {
        return this._isActive;
    }

    get selectedProvider(): string | undefined {
        return this._selectedProvider;
    }

    set selectedProvider(provider: string | undefined) {
        if (this._selectedProvider !== provider) {
            this._selectedProvider = provider;
            this.emit(CONVERSATION_EVENTS.PROVIDER_CHANGED, provider);
        }
    }

    async initialize(): Promise<void> {
        this._isActive = true;

        // Start consuming events from the client
        this.startEventConsumption();
    }

    private startEventConsumption(): void {
        this.shouldStop = false;
        this.eventConsumer = this.consumeEvents();
    }

    private async consumeEvents(): Promise<void> {
        try {
            for await (const event of this.client.events()) {
                if (this.shouldStop) {
                    break;
                }

                console.log('[Conversation] Received event:', event);
                await this.handleEvent(event);
            }
        } catch (error) {
            console.error('[Conversation] Event consumption error:', error);
            if (!this.shouldStop) {
                this.emit(CONVERSATION_EVENTS.ERROR, {
                    Error: `Event consumption error: ${error}`
                });
            }
        }
    }

    private async handleEvent(event: ChatEvent): Promise<void> {
        const tag = getChatEventTag(event);
        switch (tag) {
            case 'ConversationCleared':
                this.emit(CONVERSATION_EVENTS.CLEARED);
                break;
            case 'MessageAdded':
                this.emit(CONVERSATION_EVENTS.MESSAGE_ADDED, event);
                break;
            case 'Settings':
                this.emit(CONVERSATION_EVENTS.SETTINGS, event);
                break;
            case 'TypingStatusChanged':
                this.emit(CONVERSATION_EVENTS.TYPING_STATUS, event);
                break;
            case 'ToolExecutionCompleted':
                this.emit(CONVERSATION_EVENTS.TOOL_EXECUTION_COMPLETED, event);
                break;
            case 'OperationCancelled':
                this.emit(CONVERSATION_EVENTS.OPERATION_CANCELLED, event);
                break;
            case 'RetryAttempt':
                this.emit(CONVERSATION_EVENTS.RETRY_ATTEMPT, event);
                break;
            case 'Error':
                this.emit(CONVERSATION_EVENTS.ERROR, event);
                break;
            case 'ToolRequest':
                this.emit(CONVERSATION_EVENTS.TOOL_REQUEST, event);
                break;
            default:
                // exhaustiveness check
                const _exhaustive: never = tag;
                return _exhaustive;
        }
    }

    async sendMessage(content: string): Promise<void> {
        if (!this._isActive) {
            throw new Error('Conversation is not active');
        }

        // Auto-generate title from first message if not manually named
        if (!this._hasFirstMessage && !this._isManuallyNamed) {
            this._hasFirstMessage = true;
            const generatedTitle = this.generateTitleFromMessage(content);
            if (generatedTitle && generatedTitle !== this._title) {
                this._title = generatedTitle;
                // Don't mark as manually named since this is auto-generated
                this.emit(CONVERSATION_EVENTS.TITLE_CHANGED, generatedTitle);
            }
        }

        // Send to subprocess with selected provider
        await this.client.sendMessage(content);
    }

    async sendCancel(): Promise<void> {
        if (!this._isActive) {
            throw new Error('Conversation is not active');
        }

        // Send cancel message to subprocess
        await this.client.cancel();
    }

    private generateTitleFromMessage(message: string): string {
        // Remove leading/trailing whitespace
        let title = message.trim();

        // Remove code blocks for cleaner titles
        title = title.replace(/```[\s\S]*?```/g, '[code]');
        title = title.replace(/`[^`]+`/g, '...');

        // Remove URLs
        title = title.replace(/https?:\/\/[^\s]+/g, '[link]');

        // Remove excessive whitespace
        title = title.replace(/\s+/g, ' ');

        // Take first line/sentence
        const firstLine = title.split('\n')[0];
        const firstSentence = firstLine.split(/[.!?]/)[0];

        // Use whichever is shorter but meaningful
        title = firstSentence.length > 10 ? firstSentence : firstLine;

        // Truncate if too long (keep it concise)
        const maxLength = 40;
        if (title.length > maxLength) {
            title = title.substring(0, maxLength - 3) + '...';
        }

        // Fallback if message is too short or empty after processing
        if (title.length < 3) {
            // Try to extract something meaningful from original
            const words = message.trim().split(/\s+/).slice(0, 5).join(' ');
            title = words.length > 3 ? words : 'New Chat';
        }

        return title;
    }

    clearMessages(): void {
        this.emit(CONVERSATION_EVENTS.CLEARED);
    }

    async switchProvider(provider: string): Promise<void> {
        if (this._selectedProvider === provider) {
            return; // No change needed
        }

        // Store old provider
        const oldProvider = this._selectedProvider;
        this._selectedProvider = provider;

        // Send cancel first to stop any ongoing processing
        await this.client.cancel();

        // Then send provider change message to the subprocess
        await this.client.changeProvider(provider);

        // Emit event
        this.emit(CONVERSATION_EVENTS.PROVIDER_SWITCHED, oldProvider, provider);
    }

    dispose(): void {
        this._isActive = false;
        this.shouldStop = true;

        // Stop the event consumer
        if (this.eventConsumer) {
            this.eventConsumer.catch(() => { }); // Ignore errors during shutdown
        }

        // Close the client
        this.client.close();
        this.removeAllListeners();
    }


}