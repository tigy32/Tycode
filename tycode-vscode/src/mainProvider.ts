import * as vscode from 'vscode';
import { ConversationManager } from './conversationManager';
import { Conversation } from './conversation';
import * as path from 'path';
import {
    ChatEvent,
    MANAGER_EVENTS
} from './events';
import { ChatActorClient } from '../lib/client';

// Import build info - will be generated at build time
let buildInfo = { buildTime: 'dev', timestamp: new Date().toISOString() };
try {
    const buildModule = require('./build-info');
    buildInfo = buildModule.buildInfo;
} catch (e) {
    // Build info not available in dev mode
}

interface DiffData {
    filePath: string;
    originalContent: string;
    newContent: string;
}

export class MainProvider implements vscode.WebviewViewProvider {
    private _view?: vscode.WebviewView;
    private conversationManager: ConversationManager;
    private _diffDataStore: Map<string, DiffData> = new Map();
    private workspaceRoots = vscode.workspace.workspaceFolders?.map(f => f.uri.fsPath) || [];
    private cachedProfiles: string[] = [];
    private cachedActiveProfile: string | null = null;

    constructor(
        private readonly context: vscode.ExtensionContext
    ) {
        this.conversationManager = new ConversationManager(context);
        this.setupConversationListeners();
        this.loadProfilesFromSettings().catch(error => {
            console.error('[MainProvider] Failed to load profiles on startup:', error);
        });
    }

    private async loadProfilesFromSettings() {
        const client = new ChatActorClient(this.workspaceRoots);

        const profiles = await client.listProfiles();

        this.cachedProfiles = profiles;
        this.cachedActiveProfile = profiles.length > 0 ? profiles[0] : null;

        client.close();
        return { profiles: this.cachedProfiles, activeProfile: this.cachedActiveProfile };
    }

    private setupConversationListeners(): void {
        this.conversationManager.on(MANAGER_EVENTS.CONVERSATION_CREATED, (conversation: Conversation) => {
            this.sendToWebview({
                type: 'conversationCreated',
                id: conversation.id,
                title: conversation.title
            });
        });

        // Handle raw ChatEvent from manager - dispatch based on tag to preserve typing
        this.conversationManager.on(MANAGER_EVENTS.CHAT_EVENT, (id: string, event: ChatEvent) => {
            switch (event.kind) {
                case 'ToolRequest':
                    {
                        const toolRequest = event.data;
                        let diffId: string | undefined;

                        if (toolRequest.tool_type.kind === 'ModifyFile') {
                            const modifyFile = toolRequest.tool_type;

                            diffId = `diff-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
                            console.log('[MainProvider] Storing diff from ToolRequest with ID:', diffId);
                            this._diffDataStore.set(diffId, {
                                filePath: modifyFile.file_path,
                                originalContent: modifyFile.before,
                                newContent: modifyFile.after
                            });
                        }

                        console.log('Sending toolRequest with diffId ', diffId);
                        this.sendToWebview({
                            type: 'toolRequest',
                            conversationId: id,
                            toolName: toolRequest.tool_name,
                            toolCallId: toolRequest.tool_call_id,
                            toolType: toolRequest.tool_type,
                            diffId
                        });
                    }
                    return;
                case 'ToolExecutionCompleted':
                    {
                        const toolResult = event.data;

                        // Send tool result to webview
                        this.sendToWebview({
                            type: 'toolResult',
                            conversationId: id,
                            toolName: toolResult.tool_name,
                            toolCallId: toolResult.tool_call_id,
                            success: toolResult.success,
                            tool_result: toolResult.tool_result,
                            error: toolResult.error
                        });
                    }
                    return;
                case 'MessageAdded':
                    {
                        const chatMessage = event.data;

                        this.sendToWebview({
                            type: 'conversationMessage',
                            conversationId: id,
                            messageType: 'messageAdded',
                            message: chatMessage
                        });
                    }
                    return;
                case 'Error':
                    {
                        const errorMessage = event.data;

                        this.sendToWebview({
                            type: 'conversationMessage',
                            conversationId: id,
                            messageType: 'error',
                            message: {
                                role: 'error',
                                content: errorMessage
                            }
                        });
                    }
                    return;
                case 'TypingStatusChanged':
                    {
                        const isTyping = event.data;

                        this.sendToWebview({
                            type: 'showTyping',
                            conversationId: id,
                            show: isTyping
                        });
                    }
                    return;
                case 'RetryAttempt':
                    {
                        const retryData = event.data;

                        this.sendToWebview({
                            type: 'retryAttempt',
                            conversationId: id,
                            attempt: retryData.attempt,
                            maxRetries: retryData.max_retries,
                            error: retryData.error,
                            backoffMs: retryData.backoff_ms
                        });
                    }
                    return;
                case 'TaskUpdate':
                    {
                        const taskList = event.data;

                        this.sendToWebview({
                            type: 'taskUpdate',
                            conversationId: id,
                            taskList
                        });
                    }
                    return;
                case 'Settings':
                case 'OperationCancelled':
                case 'ConversationCleared':
                case 'ProfilesList':
                case 'ModuleSchemas':
                    // These are handled directly or not forwarded as UI updates
                    return;
                case 'SessionsList':
                    {
                        const sessions = event.data.sessions;
                        this.sendToWebview({
                            type: 'sessionsListUpdate',
                            sessions
                        });
                    }
                    return;
                case 'StreamStart':
                    {
                        this.sendToWebview({
                            type: 'showTyping',
                            conversationId: id,
                            show: false
                        });
                        this.sendToWebview({
                            type: 'streamStart',
                            conversationId: id,
                            messageId: event.data.message_id,
                            agent: event.data.agent,
                            model: event.data.model
                        });
                    }
                    return;
                case 'StreamDelta':
                    {
                        this.sendToWebview({
                            type: 'streamDelta',
                            conversationId: id,
                            messageId: event.data.message_id,
                            text: event.data.text
                        });
                    }
                    return;
                case 'StreamEnd':
                    {
                        this.sendToWebview({
                            type: 'streamEnd',
                            conversationId: id,
                            message: event.data.message
                        });
                    }
                    return;
                default:
                    // TODO: Update this exhaustiveness check when new ChatEvent types are added
                    // exhaustiveness check
                    const _exhaustive: never = event;
                    return _exhaustive;
            }
        });

        this.conversationManager.on(MANAGER_EVENTS.CONVERSATION_TITLE_CHANGED, (id: string, title: string) => {
            this.sendToWebview({
                type: 'conversationTitleChanged',
                id,
                title
            });
        });

        this.conversationManager.on(MANAGER_EVENTS.ACTIVE_CONVERSATION_CHANGED, (id: string) => {
            this.sendToWebview({
                type: 'activeConversationChanged',
                id
            });
        });

        this.conversationManager.on(MANAGER_EVENTS.CONVERSATION_CLOSED, async (id: string) => {
            this.sendToWebview({
                type: 'conversationClosed',
                id
            });
            try {
                await this.conversationManager.loadSessions();
            } catch (error) {
                console.error('[MainProvider] Failed to load sessions after closing conversation:', error);
            }
        });

        this.conversationManager.on(MANAGER_EVENTS.CONVERSATION_DISCONNECTED, (id: string) => {
            this.sendToWebview({
                type: 'conversationDisconnected',
                id
            });
        });
    }

    public resolveWebviewView(
        webviewView: vscode.WebviewView,
        context: vscode.WebviewViewResolveContext,
        _token: vscode.CancellationToken
    ) {
        this._view = webviewView;

        webviewView.webview.options = {
            enableScripts: true,
            localResourceRoots: [this.context.extensionUri]
        };

        webviewView.webview.html = this.getHtmlForWebview(webviewView.webview);

        // Handle messages from the webview
        webviewView.webview.onDidReceiveMessage(async data => {
            console.log('[MainProvider] Received message from webview:', data);
            switch (data.type) {
                case 'newChat':
                    await this.handleNewChat();
                    break;
                case 'openSettings':
                    await this.handleOpenSettings();
                    break;
                case 'sendMessage':
                    await this.handleSendMessage(data.conversationId, data.message);
                    break;
                case 'switchTab':
                    this.handleSwitchTab(data.conversationId);
                    break;
                case 'closeTab':
                    this.handleCloseTab(data.conversationId);
                    break;
                case 'renameTab':
                    this.handleRenameTab(data.conversationId, data.title);
                    break;
                case 'clearChat':
                    this.handleClearChat(data.conversationId);
                    break;
                case 'copyCode':
                    await vscode.env.clipboard.writeText(data.code);
                    vscode.window.showInformationMessage('Code copied to clipboard');
                    break;
                case 'insertCode':
                    await this.insertCodeInEditor(data.code);
                    break;
                case 'viewDiff':
                    await this.showDiff(data.diffId);
                    break;
                case 'cancel':
                    await this.handleCancel(data.conversationId);
                    break;
                case 'switchProfile':
                    await this.handleSwitchProfile(data.conversationId, data.profile);
                    break;
                case 'getProfiles':
                    this.handleGetCachedProfiles(data.conversationId);
                    break;
                case 'refreshProfiles':
                    await this.handleRefreshProfiles(data.conversationId);
                    break;
                case 'requestSessionsList':
                    await this.handleRequestSessionsList();
                    break;
                case 'resumeSession':
                    await this.handleResumeSession(data.sessionId);
                    break;
                case 'setAutonomyLevel':
                    await this.handleSetAutonomyLevel(data.conversationId, data.autonomyLevel);
                    break;
                case 'getSettings':
                    await this.handleGetSettings(data.conversationId);
                    break;
            }
        });

        // Send initial state
        this.sendInitialState();
    }

    private async sendInitialState(): Promise<void> {
        const conversations = this.conversationManager.getAllConversations();
        const activeConversation = this.conversationManager.getActiveConversation();

        this.sendToWebview({
            type: 'initialState',
            conversations: conversations.map(c => ({
                id: c.id,
                title: c.title,
                selectedProfile: c.selectedProfile
            })),
            activeConversationId: activeConversation?.id || null
        });

        for (const c of conversations) {
            this.sendToWebview({
                type: 'profileConfig',
                conversationId: c.id,
                profiles: [],
                selectedProfile: null
            });
        }
    }

    private async handleNewChat(): Promise<void> {
        try {
            const conversation = await this.conversationManager.createConversation();
            this.sendToWebview({
                type: 'showTyping',
                conversationId: conversation.id,
                show: false
            });
        } catch (error) {
            vscode.window.showErrorMessage(`Failed to create new chat: ${error}`);
        }
    }

    private async handleOpenSettings(): Promise<void> {
        await vscode.commands.executeCommand('tycode.openSettings');
    }

    private async handleSendMessage(conversationId: string, message: string): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            vscode.window.showErrorMessage('Conversation not found');
            return;
        }

        try {
            await conversation.sendMessage(message);
        } catch (error) {
            vscode.window.showErrorMessage(`Failed to send message: ${error}`);
        }
    }

    private handleSwitchTab(conversationId: string): void {
        this.conversationManager.setActiveConversation(conversationId);
    }

    private handleCloseTab(conversationId: string): void {
        this.conversationManager.closeConversation(conversationId);
    }

    private handleRenameTab(conversationId: string, title: string): void {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (conversation) {
            conversation.title = title;
        }
    }

    private handleClearChat(conversationId: string): void {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (conversation) {
            conversation.clearMessages();
            this.sendToWebview({
                type: 'conversationCleared',
                conversationId
            });
        }
    }

    private async handleCancel(conversationId: string): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        try {
            await conversation.sendCancel();
            // Hide typing indicator immediately when cancel is successful
            this.sendToWebview({
                type: 'showTyping',
                conversationId,
                show: false
            });
        } catch (error) {
            console.error('[MainProvider] Failed to cancel:', error);
        }
    }

    private async handleSwitchProfile(conversationId: string, profile: string): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        try {
            await conversation.switchProfile(profile);

            this.sendToWebview({
                type: 'profileSwitched',
                conversationId,
                newProfile: profile
            });

            // Fetch new settings from the backend to get the autonomy level from the new profile
            const settings = await conversation.client.getSettings();
            const autonomyLevel = settings.autonomy_level || 'plan_approval_required';
            conversation.autonomyLevel = autonomyLevel;

            this.sendToWebview({
                type: 'settingsUpdate',
                conversationId,
                autonomyLevel,
                defaultAgent: settings.default_agent
            });
        } catch (error) {
            console.error('[MainProvider] Failed to switch profile:', error);
            vscode.window.showErrorMessage(`Failed to switch profile: ${error}`);
        }
    }

    private async handleSetAutonomyLevel(conversationId: string, autonomyLevel: 'fully_autonomous' | 'plan_approval_required'): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        conversation.autonomyLevel = autonomyLevel;

        const settings = await conversation.client.getSettings();
        settings.autonomy_level = autonomyLevel;
        await conversation.client.saveSettings(settings, false);

        console.log(`[MainProvider] Autonomy level set to ${autonomyLevel} for conversation ${conversationId}`);
    }

    private async handleGetSettings(conversationId: string): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        try {
            const settings = await conversation.client.getSettings();
            const autonomyLevel = settings.autonomy_level || 'plan_approval_required';
            const defaultAgent = settings.default_agent;
            const profile = settings.profile || conversation.selectedProfile || this.cachedActiveProfile;

            this.sendToWebview({
                type: 'settingsUpdate',
                conversationId,
                autonomyLevel,
                defaultAgent,
                profile
            });
        } catch (error) {
            console.error('[MainProvider] Failed to get settings:', error);
        }
    }

    private async handleGetCachedProfiles(conversationId: string): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        try {
            await this.loadProfilesFromSettings();
        } catch (error) {
            console.error('[MainProvider] Failed to load profiles:', error);
            vscode.window.showErrorMessage(`Failed to load profiles: ${error}`);
            return;
        }

        // Fetch the actual profile from settings to avoid race condition
        let selectedProfile: string | undefined = conversation.selectedProfile ?? undefined;
        if (!selectedProfile) {
            try {
                const settings = await conversation.client.getSettings();
                selectedProfile = settings.profile ?? this.cachedActiveProfile ?? undefined;
            } catch (error) {
                console.error('[MainProvider] Failed to get profile from settings:', error);
                selectedProfile = this.cachedActiveProfile ?? undefined;
            }
        }

        console.log('[MainProvider] handleGetCachedProfiles - final selectedProfile:', selectedProfile);
        this.sendToWebview({
            type: 'profileConfig',
            conversationId,
            profiles: this.cachedProfiles,
            selectedProfile
        });
    }

    private async handleRefreshProfiles(conversationId: string): Promise<void> {
        try {
            await this.loadProfilesFromSettings();
        } catch (error) {
            console.error('[MainProvider] Failed to refresh profiles:', error);
            vscode.window.showErrorMessage(`Failed to refresh profiles: ${error}`);
            return;
        }

        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        this.sendToWebview({
            type: 'profileConfig',
            conversationId,
            profiles: this.cachedProfiles,
            selectedProfile: conversation.selectedProfile || this.cachedActiveProfile
        });
    }

    private async handleRequestSessionsList(): Promise<void> {
        await this.conversationManager.loadSessions();
    }

    private async handleResumeSession(sessionId: string): Promise<void> {
        try {
            await this.conversationManager.resumeSession(sessionId);
        } catch (error) {
            console.error('[MainProvider] resumeSession failed:', error);
            vscode.window.showErrorMessage(`Failed to resume session: ${error}`);
        }
    }

    private async insertCodeInEditor(code: string): Promise<void> {
        const editor = vscode.window.activeTextEditor;
        if (!editor) {
            vscode.window.showWarningMessage('No active editor');
            return;
        }

        const position = editor.selection.active;
        await editor.edit(editBuilder => {
            editBuilder.insert(position, code);
        });
    }

    private async showDiff(diffId: string): Promise<void> {
        console.log('[MainProvider] showDiff called with diffId:', diffId);
        const diffData = this._diffDataStore.get(diffId);
        if (!diffData) {
            console.error('[MainProvider] Diff data not found for diffId:', diffId);
            vscode.window.showWarningMessage('Diff data not found');
            return;
        }

        console.log('[MainProvider] Diff data found:', {
            filePath: diffData.filePath,
            originalLength: diffData.originalContent?.length,
            newLength: diffData.newContent?.length
        });

        // Create URIs for the diff - use :// to properly set authority
        const originalUri = vscode.Uri.parse(`tycode-diff://before/${diffData.filePath}?${diffId}`);
        const modifiedUri = vscode.Uri.parse(`tycode-diff://after/${diffData.filePath}?${diffId}`);

        // Register a text document content provider for the diff
        const provider = new class implements vscode.TextDocumentContentProvider {
            constructor(private data: DiffData) { }

            provideTextDocumentContent(uri: vscode.Uri): string {
                console.log('[MainProvider] provideTextDocumentContent called');
                console.log('[MainProvider] URI scheme:', uri.scheme);
                console.log('[MainProvider] URI authority:', uri.authority);
                console.log('[MainProvider] URI path:', uri.path);
                console.log('[MainProvider] Full URI:', uri.toString());

                if (uri.scheme === 'tycode-diff') {
                    if (uri.authority === 'before') {
                        console.log('[MainProvider] Returning original content, length:', this.data.originalContent?.length);
                        return this.data.originalContent;
                    } else if (uri.authority === 'after') {
                        console.log('[MainProvider] Returning new content, length:', this.data.newContent?.length);
                        return this.data.newContent;
                    }
                }
                console.log('[MainProvider] No content match for URI');
                return '';
            }
        }(diffData);

        // Register the provider temporarily
        const disposable = vscode.workspace.registerTextDocumentContentProvider('tycode-diff', provider);

        try {
            // Open the diff editor
            const title = `Changes to ${path.basename(diffData.filePath)}`;
            console.log('[MainProvider] Opening diff editor with title:', title);
            await vscode.commands.executeCommand(
                'vscode.diff',
                originalUri,
                modifiedUri,
                title,
                { preview: true }
            );
        } catch (error) {
            console.error('[MainProvider] Error opening diff editor:', error);
            vscode.window.showErrorMessage('Failed to open diff: ' + (error as Error).message);
        }

        // Clean up after a delay (keep it alive for a while in case user switches tabs)
        setTimeout(() => {
            console.log('[MainProvider] Disposing diff provider for:', diffId);
            disposable.dispose();
        }, 300000); // 5 minutes
    }

    private sendToWebview(message: any): void {
        if (this._view) {
            this._view.webview.postMessage(message);
        }
    }

    public async openChat(): Promise<void> {
        if (!this._view) {
            await vscode.commands.executeCommand('tycode.chatView.focus');
        } else {
            this._view.show?.(true);
        }
    }

    public async sendMessageToActiveChat(message: string): Promise<void> {
        let conversation = this.conversationManager.getActiveConversation();
        if (!conversation) {
            // Create a new chat if none exists (without a default title)
            try {
                conversation = await this.conversationManager.createConversation();
                this.sendToWebview({
                    type: 'showTyping',
                    conversationId: conversation.id,
                    show: false
                });
            } catch (error) {
                vscode.window.showErrorMessage(`Failed to create new chat: ${error}`);
                return;
            }
        }

        if (conversation) {
            await this.handleSendMessage(conversation.id, message);
        }
    }

    private getHtmlForWebview(webview: vscode.Webview): string {
        const scriptUri = webview.asWebviewUri(
            vscode.Uri.joinPath(this.context.extensionUri, 'out', 'webview', 'main.js')
        );
        const styleUri = webview.asWebviewUri(
            vscode.Uri.joinPath(this.context.extensionUri, 'out', 'webview', 'main.css')
        );
        const tycodeIconUri = webview.asWebviewUri(
            vscode.Uri.joinPath(this.context.extensionUri, 'tycode.png')
        );

        const nonce = this.getNonce();

        return `<!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="UTF-8">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}'; img-src ${webview.cspSource};">
                <link href="${styleUri}" rel="stylesheet">
                <title>TyCode</title>
            </head>
            <body>
                <div class="main-container">
                    <!-- Tab bar (hidden when no conversations) -->
                    <div id="tab-bar" class="tab-bar" style="display: none;">
                        <div id="tabs" class="tabs"></div>
                        <div class="new-tab-container">
                            <button id="new-tab-button" class="new-tab-button" title="New Chat or Resume Session">+</button>
                            <div id="new-tab-dropdown" class="new-tab-dropdown" style="display: none;">
                                <div id="new-chat-option" class="dropdown-item">
                                    <span class="dropdown-icon">💬</span>
                                    <span>New Chat</span>
                                </div>
                                <div class="dropdown-divider"></div>
                                <div class="dropdown-section-header">Previous Sessions</div>
                                <div id="dropdown-sessions-list" class="dropdown-sessions-list"></div>
                            </div>
                        </div>
                    </div>

                    <!-- Welcome screen (shown when no conversations) -->
                    <div id="welcome-screen" class="welcome-screen">
                        <div class="welcome-content">
                            <img src="${tycodeIconUri}" class="tycode-icon" alt="TyCode Icon" />
                            <h1 class="welcome-title">TyCode</h1>
                            <div class="welcome-buttons">
                                <button id="welcome-new-chat" class="welcome-button primary">New Chat</button>
                                <button id="welcome-settings" class="welcome-button">Settings</button>
                            </div>
                            <div id="sessions-list" class="sessions-list"></div>
                            <div class="build-info">Build ${buildInfo.buildTime}</div>
                        </div>
                    </div>

                    <!-- Conversations container (hidden when no conversations) -->
                    <div id="conversations-container" class="conversations-container" style="display: none;">
                        <!-- Conversation views will be dynamically added here -->
                    </div>
                </div>
                <template id="profile-selector-template">
                    <div class="profile-selector">
                        <label for="profile-select">Profile:</label>
                        <select class="profile-select">
                        </select>
                        <button class="refresh-profiles" title="Refresh profiles">↻</button>
                    </div>
                </template>
                <script nonce="${nonce}" type="module" src="${scriptUri}"></script>
            </body>
            </html>`;
    }

    private getNonce(): string {
        let text = '';
        const possible = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
        for (let i = 0; i < 32; i++) {
            text += possible.charAt(Math.floor(Math.random() * possible.length));
        }
        return text;
    }

    public dispose(): void {
        this.conversationManager.dispose();
    }
}
