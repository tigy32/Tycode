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
    private cachedProviders: string[] = [];
    private cachedActiveProvider: string | null = null;

    constructor(
        private readonly context: vscode.ExtensionContext
    ) {
        this.conversationManager = new ConversationManager(context);
        this.setupConversationListeners();
        // Load initial provider configuration
        this.loadProvidersFromSettings();
    }

    private async loadProvidersFromSettings() {
        try {
            const client = new ChatActorClient(this.workspaceRoots);

            // Load settings with timeout
            const settingsPromise = new Promise<any>((resolve, reject) => {
                const timeout = setTimeout(() => {
                    reject(new Error('Settings loading timeout'));
                }, 10000);

                this.consumeSettingsEvents(client, resolve, reject, timeout);
            });

            await client.getSettings();
            const settings = await settingsPromise;

            this.cachedProviders = Object.keys(settings.providers || {});
            this.cachedActiveProvider = settings.active_provider || (this.cachedProviders.length > 0 ? this.cachedProviders[0] : null);

            client.close();
            return { providers: this.cachedProviders, activeProvider: this.cachedActiveProvider };
        } catch (error) {
            console.error('[MainProvider] Error loading settings:', error);
            // Fallback to default provider
            this.cachedProviders = ['default'];
            this.cachedActiveProvider = 'default';
            return { providers: this.cachedProviders, activeProvider: this.cachedActiveProvider };
        }
    }

    private async consumeSettingsEvents(client: ChatActorClient, resolve: (value: any) => void, reject: (error: Error) => void, timeout: NodeJS.Timeout): Promise<void> {
        try {
            for await (const event of client.events()) {
                switch (event.kind) {
                    case 'Settings':
                        clearTimeout(timeout);
                        resolve(event.data);
                        return;
                    case 'Error':
                        clearTimeout(timeout);
                        reject(new Error(event.data));
                        return;
                    case 'ConversationCleared':
                    case 'MessageAdded':
                    case 'TypingStatusChanged':
                    case 'ToolExecutionCompleted':
                    case 'OperationCancelled':
                    case 'RetryAttempt':
                    case 'ToolRequest':
                        // Ignore these events during settings loading
                        break;
                    default:
                        // exhaustiveness check
                        const _exhaustive: never = event;
                        clearTimeout(timeout);
                        reject(new Error(`Unexpected ChatEvent: ${JSON.stringify(event)}`));
                        return _exhaustive;
                }
            }
        } catch (error) {
            clearTimeout(timeout);
            reject(error as Error);
        }
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
                case 'Settings':
                case 'OperationCancelled':
                case 'ConversationCleared':
                    // These are handled directly or not forwarded as UI updates
                    return;
                default:
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

        this.conversationManager.on(MANAGER_EVENTS.CONVERSATION_CLOSED, (id: string) => {
            this.sendToWebview({
                type: 'conversationClosed',
                id
            });
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
                case 'switchProvider':
                    await this.handleSwitchProvider(data.conversationId, data.provider);
                    break;
                case 'getProviders':
                    // Just get cached providers, no reload
                    this.handleGetCachedProviders(data.conversationId);
                    break;
                case 'refreshProviders':
                    // Force reload from disk
                    await this.handleRefreshProviders(data.conversationId);
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
                selectedProvider: c.selectedProvider
            })),
            activeConversationId: activeConversation?.id || null
        });

        // On initial load, get provider info from each conversation
        for (const c of conversations) {
            // Note: Settings will be handled through events in the new system
            // We'll send provider config when settings events are received
            this.sendToWebview({
                type: 'providerConfig',
                conversationId: c.id,
                providers: [],
                selectedProvider: null
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

    private async handleSwitchProvider(conversationId: string, provider: string): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        try {
            await conversation.switchProvider(provider);

            this.sendToWebview({
                type: 'providerSwitched',
                conversationId,
                newProvider: provider
            });
        } catch (error) {
            console.error('[MainProvider] Failed to switch provider:', error);
            vscode.window.showErrorMessage(`Failed to switch provider: ${error}`);
        }
    }

    private async handleGetCachedProviders(conversationId: string): Promise<void> {
        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        // Reload providers from settings
        await this.loadProvidersFromSettings();

        // Get the active provider for this conversation (initially the default)
        const selectedProvider = conversation.selectedProvider || this.cachedActiveProvider;

        this.sendToWebview({
            type: 'providerConfig',
            conversationId,
            providers: this.cachedProviders,
            selectedProvider
        });
    }

    private async handleRefreshProviders(conversationId: string): Promise<void> {
        // Force reload from disk
        await this.loadProvidersFromSettings();

        const conversation = this.conversationManager.getConversation(conversationId);
        if (!conversation) {
            return;
        }

        this.sendToWebview({
            type: 'providerConfig',
            conversationId,
            providers: this.cachedProviders,
            selectedProvider: conversation.selectedProvider || this.cachedActiveProvider
        });
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

        const nonce = this.getNonce();

        return `<!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="UTF-8">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
                <link href="${styleUri}" rel="stylesheet">
                <title>TyCode</title>
            </head>
            <body>
                <div class="main-container">
                    <!-- Tab bar (hidden when no conversations) -->
                    <div id="tab-bar" class="tab-bar" style="display: none;">
                        <div id="tabs" class="tabs"></div>
                        <button id="new-tab-button" class="new-tab-button" title="New Chat">+</button>
                    </div>

                    <!-- Welcome screen (shown when no conversations) -->
                    <div id="welcome-screen" class="welcome-screen">
                        <div class="welcome-content">
                            <div class="tiger-emoji">🐯</div>
                            <h1 class="welcome-title">TyCode</h1>
                            <div class="welcome-buttons">
                                <button id="welcome-new-chat" class="welcome-button primary">New Chat</button>
                                <button id="welcome-settings" class="welcome-button">Settings</button>
                            </div>
                            <div class="build-info">Build ${buildInfo.buildTime}</div>
                        </div>
                    </div>

                    <!-- Conversations container (hidden when no conversations) -->
                    <div id="conversations-container" class="conversations-container" style="display: none;">
                        <!-- Conversation views will be dynamically added here -->
                    </div>
                </div>
                <!-- Provider selector template (will be cloned for each conversation) -->
                <template id="provider-selector-template">
                    <div class="provider-selector">
                        <label for="provider-select">Provider:</label>
                        <select class="provider-select">
                            <!-- Options will be populated dynamically -->
                        </select>
                        <button class="refresh-providers" title="Refresh providers">↻</button>
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
