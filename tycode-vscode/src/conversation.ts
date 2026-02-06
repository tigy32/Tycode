import { EventEmitter } from 'events';
import { ChatActorClient } from '../lib/client';
import * as vscode from 'vscode';
import {
    MessageSender,
    ChatEvent,
    ChatMessage,
    ImageData,
    CONVERSATION_EVENTS
} from './events';

export class Conversation extends EventEmitter {
    public client: ChatActorClient;
    private _id: string;
    private _title: string;

    private _isActive: boolean = false;
    private _isManuallyNamed: boolean = false;
    private _hasFirstMessage: boolean = false;
    private _selectedProfile: string | undefined;
    private _autonomyLevel: 'fully_autonomous' | 'plan_approval_required' = 'plan_approval_required';
    private eventConsumer: Promise<void> | null = null;
    private shouldStop: boolean = false;

    constructor(
        private context: vscode.ExtensionContext,
        id: string,
        title?: string,
        selectedProfile?: string
    ) {
        super();
        this._id = id;
        this._title = title || 'New Chat';
        this._isManuallyNamed = !!title;
        this._selectedProfile = selectedProfile;

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

    get selectedProfile(): string | undefined {
        return this._selectedProfile;
    }

    set selectedProfile(profile: string | undefined) {
        if (this._selectedProfile !== profile) {
            this._selectedProfile = profile;
            this.emit(CONVERSATION_EVENTS.PROFILE_CHANGED, profile);
        }
    }

    get autonomyLevel(): 'fully_autonomous' | 'plan_approval_required' {
        return this._autonomyLevel;
    }

    set autonomyLevel(level: 'fully_autonomous' | 'plan_approval_required') {
        this._autonomyLevel = level;
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
        for await (const event of this.client.events()) {
            if (this.shouldStop) {
                break;
            }

            console.log('[Conversation] Received event from client, kind:', event.kind);
            console.log('[Conversation] Full event:', JSON.stringify(event, null, 2));
            this.emit(CONVERSATION_EVENTS.CHAT_EVENT, event);
        }
    }

    async sendMessage(content: string, images?: ImageData[]): Promise<void> {
        if (!this._isActive) {
            throw new Error('Conversation is not active');
        }

        // Auto-generate title from first message if not manually named
        if (!this._hasFirstMessage && !this._isManuallyNamed) {
            this._hasFirstMessage = true;
            const generatedTitle = this.generateTitleFromMessage(content);
            if (generatedTitle && generatedTitle !== this._title) {
                this._title = generatedTitle;
                this.emit(CONVERSATION_EVENTS.TITLE_CHANGED, generatedTitle);
            }
        }

        if (images && images.length > 0) {
            await this.client.sendMessageWithImages(content, images);
        } else {
            await this.client.sendMessage(content);
        }
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

    async switchProfile(profile: string): Promise<void> {
        if (this._selectedProfile === profile) {
            return;
        }

        const oldProfile = this._selectedProfile;
        this._selectedProfile = profile;

        // Send cancel first to stop any ongoing processing
        await this.client.cancel();
        await this.client.switchProfile(profile);

        this.emit(CONVERSATION_EVENTS.PROFILE_SWITCHED, oldProfile, profile);
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
