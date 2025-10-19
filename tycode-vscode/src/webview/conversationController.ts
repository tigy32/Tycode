import { WebviewContext } from './context.js';
import {
    ActiveConversationChangedMessage,
    ConversationClearedMessage,
    ConversationClosedMessage,
    ConversationCreatedMessage,
    ConversationState,
    ConversationDisconnectedMessage,
    ConversationMessageMessage,
    ConversationTitleChangedMessage,
    InitialStateMessage,
    ProviderConfigMessage,
    ProviderSwitchedMessage,
    RetryAttemptMessage,
    ShowTypingMessage,
    PendingToolUpdate,
    ToolRequestMessage,
    ToolResultMessage
} from './types.js';
import {
    addCodeActions,
    escapeHtml,
    formatBytes,
    getRoleFromSender,
    renderContent
} from './utils.js';

type ToolContext = {
    command?: string;
};

export interface ConversationController {
    handleInitialState(message: InitialStateMessage): void;
    handleConversationCreated(message: ConversationCreatedMessage): void;
    handleConversationMessage(message: ConversationMessageMessage): void;
    handleActiveConversationChanged(message: ActiveConversationChangedMessage): void;
    handleConversationClosed(message: ConversationClosedMessage): void;
    handleConversationTitleChanged(message: ConversationTitleChangedMessage): void;
    handleConversationCleared(message: ConversationClearedMessage): void;
    handleShowTyping(message: ShowTypingMessage): void;
    handleRetryAttempt(message: RetryAttemptMessage): void;
    handleConversationDisconnected(message: ConversationDisconnectedMessage): void;
    handleToolRequest(message: ToolRequestMessage): void;
    handleToolResult(message: ToolResultMessage): void;
    handleProviderConfig(message: ProviderConfigMessage): void;
    handleProviderSwitched(message: ProviderSwitchedMessage): void;
    registerGlobalListeners(): void;
}

export function createConversationController(context: WebviewContext): ConversationController {
    const reasoningToggleState = new Map<string, boolean>();

    function ensurePendingToolMap(conversation: ConversationState): Map<string, PendingToolUpdate> {
        if (!conversation.pendingToolUpdates) {
            conversation.pendingToolUpdates = new Map<string, PendingToolUpdate>();
        }
        return conversation.pendingToolUpdates;
    }

    function locateToolItem(
        conversation: ConversationState,
        toolName: string,
        toolCallId: string
    ): HTMLElement | null {
        const byId = conversation.viewElement.querySelector<HTMLElement>(
            `.tool-call-item[data-tool-call-id="${toolCallId}"]`
        );
        if (byId) {
            return byId;
        }

        const toolItems = conversation.viewElement.querySelectorAll<HTMLElement>(
            `.tool-call-item[data-tool-name="${toolName}"]`
        );
        if (toolItems.length === 0) {
            return null;
        }

        const fallback = toolItems[toolItems.length - 1];
        fallback.setAttribute('data-tool-call-id', toolCallId);
        return fallback;
    }

    function applyToolRequest(
        conversation: ConversationState,
        toolItem: HTMLElement,
        message: ToolRequestMessage
    ): void {
        const statusIcon = toolItem.querySelector<HTMLElement>('.tool-status-icon');
        const statusText = toolItem.querySelector<HTMLElement>('.tool-status-text');
        if (statusIcon) statusIcon.textContent = '🔧';
        if (statusText) statusText.textContent = 'Requested';

        toolItem.setAttribute('data-tool-call-id', message.toolCallId);
        if (message.diffId) {
            toolItem.setAttribute('data-diff-id', message.diffId);
        }

        if (message.toolType.kind === 'ModifyFile') {
            toolItem.setAttribute('data-file-path', message.toolType.file_path);
        }

        if (message.toolType.kind === 'RunCommand') {
            toolItem.setAttribute('data-run-command', message.toolType.command);
        }

        const debugRequest = toolItem.querySelector<HTMLDivElement>('.tool-debug-request');
        if (debugRequest) {
            const payload: Record<string, unknown> = {
                toolCallId: message.toolCallId,
                toolName: message.toolName,
                toolType: message.toolType
            };
            if (message.diffId) {
                payload.diffId = message.diffId;
            }

            const compactPayload = Object.fromEntries(
                Object.entries(payload).filter(([, value]) => value !== undefined)
            );

            debugRequest.innerHTML = `<strong>Request:</strong><pre>${escapeHtml(JSON.stringify(compactPayload, null, 2))}</pre>`;
        }

        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        if (messagesContainer) {
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }
    }

    function applyToolResult(
        conversation: ConversationState,
        toolItem: HTMLElement,
        message: ToolResultMessage
    ): void {
        const statusIcon = toolItem.querySelector<HTMLElement>('.tool-status-icon');
        const statusText = toolItem.querySelector<HTMLElement>('.tool-status-text');
        const resultDiv = toolItem.querySelector<HTMLDivElement>('.tool-result');

        if (statusIcon && statusText) {
            if (message.success) {
                statusIcon.textContent = '✅';
                statusText.textContent = 'Success';
                toolItem.classList.add('tool-success');
                toolItem.classList.remove('tool-error');
            } else {
                statusIcon.textContent = '❌';
                statusText.textContent = 'Failed';
                toolItem.classList.add('tool-error');
                toolItem.classList.remove('tool-success');
            }
        }

        let diffId = toolItem.getAttribute('data-diff-id');
        if (!diffId && message.diffId) {
            diffId = message.diffId;
            toolItem.setAttribute('data-diff-id', diffId);
        }

        let toolContext: ToolContext | undefined;
        const datasetCommand = toolItem.getAttribute('data-run-command') ?? undefined;
        if (datasetCommand) {
            toolContext = { command: datasetCommand };
        }

        const filePath = toolItem.getAttribute('data-file-path') ?? undefined;

        if (resultDiv) {
            resultDiv.style.display = 'block';

            let resultContent = '';
            if (message.error) {
                resultContent = `<div class="tool-error-message">${escapeHtml(message.error)}</div>`;
            } else {
                resultContent = formatToolResult(message.tool_result, diffId, toolContext, filePath);
            }

            resultDiv.innerHTML = resultContent;
        }

        const debugSection = toolItem.querySelector<HTMLDivElement>('.tool-debug-section');
        if (debugSection) {
            const debugResponse = debugSection.querySelector<HTMLDivElement>('.tool-debug-response');
            if (debugResponse) {
                const responseData = message.error ? { error: message.error } : message.tool_result;
                if (responseData) {
                    debugResponse.innerHTML = `<strong>Response:</strong><pre>${escapeHtml(JSON.stringify(responseData, null, 2))}</pre>`;
                } else {
                    debugResponse.innerHTML = '';
                }
            }
        }

        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        if (messagesContainer) {
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }
    }

    function hydrateToolItem(
        conversation: ConversationState,
        toolItem: HTMLElement,
        toolCallId: string
    ): void {
        if (!conversation.pendingToolUpdates) {
            return;
        }

        const entry = conversation.pendingToolUpdates.get(toolCallId);
        if (!entry) {
            return;
        }

        if (entry.request) {
            applyToolRequest(conversation, toolItem, entry.request);
            entry.request = undefined;
        }

        if (entry.result) {
            applyToolResult(conversation, toolItem, entry.result);
            entry.result = undefined;
        }

        if (!entry.request && !entry.result) {
            conversation.pendingToolUpdates.delete(toolCallId);
        } else {
            conversation.pendingToolUpdates.set(toolCallId, entry);
        }
    }

    function handleInitialState(message: InitialStateMessage): void {
        context.store.clear();
        context.dom.tabsContainer.innerHTML = '';
        context.dom.conversationsContainer.innerHTML = '';

        if (message.conversations && message.conversations.length > 0) {
            for (const conv of message.conversations) {
                createConversationUI(conv.id, conv.title);

                if (conv.messages) {
                    for (const msg of conv.messages) {
                        displayMessage(conv.id, msg);
                    }
                }
            }

            if (message.activeConversationId) {
                setActiveConversation(message.activeConversationId);
            }

            showConversations();
        } else {
            showWelcomeScreen();
        }
    }

    function handleConversationCreated(message: ConversationCreatedMessage): void {
        createConversationUI(message.id, message.title);
        setActiveConversation(message.id);
        showConversations();
    }

    function handleConversationMessage(message: ConversationMessageMessage): void {
        displayMessage(message.conversationId, message.message);
    }

    function handleActiveConversationChanged(message: ActiveConversationChangedMessage): void {
        setActiveConversation(message.id);
    }

    function handleConversationClosed(message: ConversationClosedMessage): void {
        const conversation = context.store.get(message.id);
        if (conversation) {
            conversation.tabElement.remove();
            conversation.viewElement.remove();
            context.store.delete(message.id);
        }

        if (context.store.size() === 0) {
            showWelcomeScreen();
        }
    }

    function handleConversationTitleChanged(message: ConversationTitleChangedMessage): void {
        const conversation = context.store.get(message.id);
        if (!conversation) return;

        conversation.title = message.title;

        const titleElement = conversation.tabElement.querySelector<HTMLSpanElement>('.tab-title');
        const inputElement = conversation.tabElement.querySelector<HTMLInputElement>('.tab-title-input');
        if (titleElement) titleElement.textContent = message.title;
        if (inputElement) inputElement.value = message.title;

        const header = conversation.viewElement.querySelector<HTMLHeadingElement>('.chat-header h3');
        if (header) header.textContent = message.title;
    }

    function handleConversationCleared(message: ConversationClearedMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (conversation) {
            conversation.messages = [];
        }
    }

    function handleShowTyping(message: ShowTypingMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const typingIndicator = conversation.viewElement.querySelector<HTMLDivElement>('.typing-indicator');
        if (typingIndicator) {
            typingIndicator.style.display = message.show ? 'flex' : 'none';
        }

        if (message.show) {
            const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
            if (messagesContainer) {
                messagesContainer.scrollTop = messagesContainer.scrollHeight;
            }
        }

        const sendButton = conversation.viewElement.querySelector<HTMLButtonElement>('.send-button');
        const cancelButton = conversation.viewElement.querySelector<HTMLButtonElement>('.cancel-button');

        if (sendButton && cancelButton) {
            if (message.show) {
                sendButton.style.display = 'none';
                cancelButton.style.display = 'block';
                conversation.isProcessing = true;
            } else {
                sendButton.style.display = 'block';
                cancelButton.style.display = 'none';
                conversation.isProcessing = false;

                const retryElement = context.retryElements.get(message.conversationId);
                if (retryElement) {
                    retryElement.remove();
                    context.retryElements.delete(message.conversationId);
                }
            }
        }
    }

    function handleRetryAttempt(message: RetryAttemptMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        if (!messagesContainer) return;

        let retryElement = context.retryElements.get(message.conversationId);
        if (!retryElement) {
            retryElement = document.createElement('div');
            retryElement.className = 'message system retry-status';
            messagesContainer.appendChild(retryElement);
            context.retryElements.set(message.conversationId, retryElement);
        }

        const errorMsg = message.error ? message.error.substring(0, 100) : 'Request failed';
        const nextAttemptIn = Math.ceil(message.backoffMs / 1000);

        retryElement.innerHTML = `
            <div class="retry-info">
                <span class="retry-icon">🔄</span>
                <span class="retry-text">
                    [Request failed - retrying (attempt ${message.attempt}/${message.maxRetries})]
                    <br>
                    <span class="retry-error">${escapeHtml(errorMsg)}</span>
                    <br>
                    <span class="retry-countdown">Next attempt in ${nextAttemptIn}s...</span>
                </span>
            </div>
        `;

        messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }

    function handleConversationDisconnected(message: ConversationDisconnectedMessage): void {
        if (context.store.get(message.id)) {
            displayMessage(message.id, {
                role: 'error',
                content: 'Connection to backend lost. Please close this tab and start a new chat.'
            });
        }
    }

    function handleToolRequest(message: ToolRequestMessage): void {
        const { conversationId, toolName, toolCallId } = message;
        const conversation = context.store.get(conversationId);
        if (!conversation) return;

        const pendingMap = ensurePendingToolMap(conversation);
        const entry = pendingMap.get(toolCallId) ?? {};
        entry.request = message;
        pendingMap.set(toolCallId, entry);

        const toolItem = locateToolItem(conversation, toolName, toolCallId);
        if (!toolItem) {
            const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
            if (messagesContainer) {
                messagesContainer.scrollTop = messagesContainer.scrollHeight;
            }
            return;
        }

        hydrateToolItem(conversation, toolItem, toolCallId);
    }

    function handleToolResult(message: ToolResultMessage): void {
        const { conversationId, toolName, toolCallId } = message;

        const conversation = context.store.get(conversationId);
        if (!conversation) return;

        const pendingMap = ensurePendingToolMap(conversation);
        const entry = pendingMap.get(toolCallId) ?? {};
        entry.result = message;
        pendingMap.set(toolCallId, entry);

        const toolItem = locateToolItem(conversation, toolName, toolCallId);
        if (!toolItem) {
            return;
        }

        hydrateToolItem(conversation, toolItem, toolCallId);
    }

    function handleProviderConfig(message: ProviderConfigMessage): void {
        const { conversationId, providers, selectedProvider } = message;
        const conversation = context.store.get(conversationId);
        if (!conversation) return;

        const providerSelect = conversation.viewElement.querySelector<HTMLSelectElement>('.provider-select');
        if (!providerSelect) return;

        providerSelect.innerHTML = '';

        if (providers && providers.length > 0) {
            providers.forEach(provider => {
                const option = document.createElement('option');
                option.value = provider;
                option.textContent = provider;
                if (provider === selectedProvider) {
                    option.selected = true;
                }
                providerSelect.appendChild(option);
            });
        } else {
            const option = document.createElement('option');
            option.value = 'default';
            option.textContent = 'default';
            option.selected = true;
            providerSelect.appendChild(option);
        }

        conversation.selectedProvider = selectedProvider;
    }

    function handleProviderSwitched(message: ProviderSwitchedMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const providerSelect = conversation.viewElement.querySelector<HTMLSelectElement>('.provider-select');
        if (providerSelect && message.newProvider) {
            providerSelect.value = message.newProvider;
        }

        conversation.selectedProvider = message.newProvider;
    }

    function registerGlobalListeners(): void {
        document.addEventListener('click', (e: MouseEvent) => {
            const target = e.target as HTMLElement | null;
            if (target?.classList?.contains('view-diff-button')) {
                const diffId = target.getAttribute('data-diff-id');
                if (diffId) {
                    context.vscode.postMessage({
                        type: 'viewDiff',
                        diffId
                    });
                }
            }
            
            if (target?.classList?.contains('tool-debug-toggle')) {
                const toolItem = target.closest('.tool-call-item');
                const debugContent = toolItem?.querySelector<HTMLDivElement>('.tool-debug-content');
                if (debugContent) {
                    const isExpanded = debugContent.classList.contains('expanded');
                    if (isExpanded) {
                        debugContent.classList.remove('expanded');
                        target.textContent = '▶';
                    } else {
                        debugContent.classList.add('expanded');
                        target.textContent = '▼';
                    }
                }
            }
        });
    }

    function createConversationUI(id: string, title: string): void {
        const tab = document.createElement('div');
        tab.className = 'tab';
        tab.dataset.conversationId = id;
        tab.innerHTML = `
            <span class="tab-title">${escapeHtml(title)}</span>
            <input class="tab-title-input" type="text" value="${escapeHtml(title)}" style="display: none;">
            <button class="tab-close" title="Close">×</button>
        `;

        const tabTitle = tab.querySelector<HTMLSpanElement>('.tab-title');
        const tabInput = tab.querySelector<HTMLInputElement>('.tab-title-input');
        const tabCloseButton = tab.querySelector<HTMLButtonElement>('.tab-close');
        if (!tabTitle || !tabInput || !tabCloseButton) {
            throw new Error('Tab template is missing expected elements');
        }

        let isEditing = false;

        tabTitle.addEventListener('dblclick', (e: MouseEvent) => {
            e.stopPropagation();
            startEditingTitle(id, tab, tabTitle, tabInput);
        });

        tab.addEventListener('click', (e: MouseEvent) => {
            const target = e.target as HTMLElement;
            if (!target.classList.contains('tab-close') && !isEditing) {
                context.vscode.postMessage({ type: 'switchTab', conversationId: id });
            }
        });

        tabInput.addEventListener('keydown', (e: KeyboardEvent) => {
            if (e.key === 'Enter') {
                e.preventDefault();
                saveTabTitle(id, tab, tabTitle, tabInput);
            } else if (e.key === 'Escape') {
                e.preventDefault();
                cancelEditingTitle(tab, tabTitle, tabInput);
            }
        });

        tabInput.addEventListener('blur', () => {
            if (tabInput.style.display !== 'none') {
                saveTabTitle(id, tab, tabTitle, tabInput);
            }
        });

        tab.addEventListener('contextmenu', (e: MouseEvent) => {
            e.preventDefault();
            showTabContextMenu(e, id, tab, tabTitle, tabInput);
        });

        tabCloseButton.addEventListener('click', (e: MouseEvent) => {
            e.stopPropagation();
            context.vscode.postMessage({ type: 'closeTab', conversationId: id });
        });

        context.dom.tabsContainer.appendChild(tab);

        const conversationView = document.createElement('div');
        conversationView.className = 'conversation-view';
        conversationView.dataset.conversationId = id;
        conversationView.style.display = 'none';
        conversationView.innerHTML = `
            <div class="chat-header">
                <h3>${escapeHtml(title)}</h3>
                <button class="header-button clear-chat" title="Clear chat">🗑️</button>
            </div>
            <div class="messages"></div>
            <div class="typing-indicator" style="display: none;">
                <span></span>
                <span></span>
                <span></span>
            </div>
            <div class="input-container">
                <textarea class="message-input" placeholder="Ask me anything about your code..." rows="3"></textarea>
                <button class="send-button">Send</button>
                <button class="cancel-button" style="display: none;">Cancel</button>
            </div>
            <div class="provider-selector" style="display: flex !important; align-items: center; gap: 10px; padding: 8px 16px; background-color: var(--vscode-input-background, #1e1e1e); border-top: 1px solid var(--vscode-panel-border, #3c3c3c); font-size: 12px;">
                <label for="provider-select-${id}" style="color: var(--vscode-descriptionForeground, #cccccc); font-weight: 500; white-space: nowrap;">Provider:</label>
                <select id="provider-select-${id}" class="provider-select" style="flex: 1; min-width: 100px; padding: 4px 8px; background-color: var(--vscode-input-background, #3c3c3c); color: var(--vscode-input-foreground, #cccccc); border: 1px solid var(--vscode-input-border, #3c3c3c); border-radius: 2px; font-size: 12px; cursor: pointer;">
                    <option value="loading">Loading...</option>
                </select>
                <button class="refresh-providers" title="Refresh providers" style="padding: 4px 8px; background-color: transparent; color: var(--vscode-foreground, #cccccc); border: 1px solid var(--vscode-input-border, #3c3c3c); border-radius: 2px; cursor: pointer; font-size: 14px; line-height: 1;">↻</button>
            </div>
        `;

        const messageInput = conversationView.querySelector<HTMLTextAreaElement>('.message-input');
        const sendButton = conversationView.querySelector<HTMLButtonElement>('.send-button');
        const cancelButton = conversationView.querySelector<HTMLButtonElement>('.cancel-button');
        const clearButton = conversationView.querySelector<HTMLButtonElement>('.clear-chat');
        const providerSelect = conversationView.querySelector<HTMLSelectElement>('.provider-select');
        const refreshProvidersBtn = conversationView.querySelector<HTMLButtonElement>('.refresh-providers');

        if (!messageInput || !sendButton || !cancelButton || !clearButton) {
            throw new Error('Conversation view missing expected controls');
        }

        sendButton.addEventListener('click', () => sendMessage(id, messageInput));

        cancelButton.addEventListener('click', () => {
            const pendingMessage = messageInput.value.trim();

            context.vscode.postMessage({
                type: 'cancel',
                conversationId: id
            });

            if (pendingMessage) {
                messageInput.value = '';
                messageInput.style.height = 'auto';

                setTimeout(() => {
                    context.vscode.postMessage({
                        type: 'sendMessage',
                        conversationId: id,
                        message: pendingMessage
                    });
                }, 100);
            }
        });

        messageInput.addEventListener('keydown', (e: KeyboardEvent) => {
            if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                const conversation = context.store.get(id);
                if (conversation && conversation.isProcessing) {
                    cancelButton.click();
                } else {
                    sendMessage(id, messageInput);
                }
            }
        });

        messageInput.addEventListener('input', () => {
            messageInput.style.height = 'auto';
            messageInput.style.height = `${messageInput.scrollHeight}px`;
        });

        clearButton.addEventListener('click', () => {
            const messagesContainer = conversationView.querySelector<HTMLDivElement>('.messages');
            if (messagesContainer) {
                messagesContainer.innerHTML = '';
            }
            context.vscode.postMessage({ type: 'clearChat', conversationId: id });
        });

        if (providerSelect) {
            providerSelect.addEventListener('change', (e: Event) => {
                const target = e.target as HTMLSelectElement;
                const selectedProvider = target.value;
                context.vscode.postMessage({
                    type: 'switchProvider',
                    conversationId: id,
                    provider: selectedProvider
                });
            });
        }

        if (refreshProvidersBtn) {
            refreshProvidersBtn.addEventListener('click', () => {
                context.vscode.postMessage({
                    type: 'refreshProviders',
                    conversationId: id
                });
            });
        }

        context.dom.conversationsContainer.appendChild(conversationView);

        const state: ConversationState = {
            title,
            messages: [],
            tabElement: tab,
            viewElement: conversationView,
            selectedProvider: null,
            pendingToolUpdates: new Map<string, PendingToolUpdate>()
        };

        context.store.set(id, state);

        context.vscode.postMessage({
            type: 'getProviders',
            conversationId: id
        });
    }

    function sendMessage(conversationId: string, inputElement: HTMLTextAreaElement): void {
        const message = inputElement.value.trim();
        if (!message) return;

        inputElement.value = '';
        inputElement.style.height = 'auto';

        context.vscode.postMessage({
            type: 'sendMessage',
            conversationId,
            message
        });
    }

    function displayMessage(conversationId: string, chatMessage: any): void {
        const conversation = context.store.get(conversationId);
        if (!conversation) return;

        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        if (!messagesContainer) return;

        let role;
        let content;
        let reasoning;
        let toolCalls;
        let model;
        let isComplete;
        let tokenUsage;

        if (chatMessage.sender) {
            role = getRoleFromSender(chatMessage.sender);
            content = chatMessage.content;
            reasoning = chatMessage.reasoning?.text;
            toolCalls = chatMessage.tool_calls || [];
            model = chatMessage.model_info?.model;
            isComplete = true;
            tokenUsage = chatMessage.token_usage;
        } else {
            role = chatMessage.role || 'system';
            content = chatMessage.content;
            reasoning = chatMessage.reasoning;
            toolCalls = chatMessage.toolCalls || [];
            model = chatMessage.model;
            isComplete = chatMessage.isComplete;
            tokenUsage = chatMessage.tokenUsage;
        }

        const messageDiv = document.createElement('div');
        messageDiv.className = `message ${role}`;

        if (role === 'assistant') {
            const modelInfo = model ? `<div class="model-info">Model: ${model}</div>` : '';
            const completionInfo = isComplete !== undefined
                ? `<div class="completion-info">${isComplete ? '✅ Complete' : '⏳ Pending AI response'}</div>`
                : '';

            let tokenInfo = '';
            if (tokenUsage) {
                const displayInputTokens = tokenUsage.input_tokens + (tokenUsage.cache_creation_input_tokens || 0);
                const displayOutputTokens = tokenUsage.output_tokens + (tokenUsage.reasoning_tokens || 0);

                let inputPart = `${displayInputTokens}`;
                if (tokenUsage.cached_prompt_tokens && tokenUsage.cached_prompt_tokens > 0) {
                    inputPart += ` (${tokenUsage.cached_prompt_tokens} cached)`;
                }

                let outputPart = `${displayOutputTokens}`;
                if (tokenUsage.reasoning_tokens && tokenUsage.reasoning_tokens > 0) {
                    outputPart += ` (${tokenUsage.reasoning_tokens} reasoning)`;
                }

                tokenInfo = `<div class="token-info">📊 Tokens: ${inputPart}/${outputPart}</div>`;
            }

            let reasoningSection = '';
            if (reasoning) {
                const reasoningId = `reasoning-${Date.now()}`;
                const isLong = reasoning.length > 120;
                const truncated = isLong ? `${reasoning.substring(0, 120)}...` : reasoning;

                reasoningSection = `
                    <div class="embedded-reasoning">
                        <div class="reasoning-header reasoning-header-clickable" data-reasoning-id="${reasoningId}">
                            💭 Reasoning
                            <span class="reasoning-toggle" id="${reasoningId}-toggle">
                                ${isLong ? '▶' : ''}
                            </span>
                        </div>
                        <div class="reasoning-content ${isLong ? 'collapsed' : ''}" id="${reasoningId}">
                            <div class="reasoning-truncated">${renderContent(truncated)}</div>
                            <div class="reasoning-full" style="display: none;">${renderContent(reasoning)}</div>
                        </div>
                    </div>
                `;

                reasoningToggleState.set(reasoningId, isLong);

                setTimeout(() => {
                    const header = messageDiv.querySelector('.reasoning-header-clickable');
                    header?.addEventListener('click', () => toggleReasoning(reasoningId));
                }, 0);
            }

            let toolCallsSection = '';
            const toolCallMetadata: Array<{ elementId: string; toolCallId: string; command?: string }>
                = [];
            if (toolCalls && toolCalls.length > 0) {
                const toolCallsHtml = toolCalls.map((toolCall: any) => {
                    const toolId = `tool-${conversationId}-${Date.now()}-${toolCall.name}`;
                    const toolCallId = toolCall.id ?? toolCall.tool_call_id ?? toolId;
                    const runCommand =
                        toolCall?.arguments && typeof toolCall.arguments === 'object'
                            ? (toolCall.arguments as Record<string, unknown>).command
                            : undefined;
                    const commandString = typeof runCommand === 'string' ? runCommand : undefined;
                    toolCallMetadata.push({ elementId: toolId, toolCallId, command: commandString });
                    const initialRequestHtml = toolCall.arguments
                        ? `<strong>Request:</strong><pre>${escapeHtml(JSON.stringify(toolCall.arguments, null, 2))}</pre>`
                        : '';
                    return `
                        <div class="tool-call-item" data-tool-name="${toolCall.name}" data-tool-call-id="${toolCallId}" data-conversation-id="${conversationId}" id="${toolId}">
                            <div class="tool-header">
                                <span class="tool-status-icon">⏳</span>
                                <span class="tool-name">${toolCall.name}</span>
                                <span class="tool-status-text">Executing...</span>
                                <span class="tool-debug-toggle">▶</span>
                            </div>
                            <div class="tool-result" style="display: none;"></div>
                            <div class="tool-debug-section">
                                <div class="tool-debug-content">
                                    <div class="tool-debug-request">${initialRequestHtml}</div>
                                    <div class="tool-debug-response"></div>
                                </div>
                            </div>
                        </div>
                    `;
                }).join('');

                toolCallsSection = `
                    <div class="embedded-tool-calls">
                        ${toolCallsHtml}
                    </div>
                `;
            }

            messageDiv.innerHTML = `
                ${modelInfo}
                ${tokenInfo}
                ${reasoningSection}
                <div class="message-content">${renderContent(content)}</div>
                ${completionInfo}
                ${toolCallsSection}
            `;

            if (toolCallMetadata.length > 0) {
                for (const meta of toolCallMetadata) {
                    const toolElement = messageDiv.querySelector<HTMLElement>(`#${meta.elementId}`);
                    if (toolElement) {
                        if (meta.command) {
                            toolElement.setAttribute('data-run-command', meta.command);
                        }
                        hydrateToolItem(conversation, toolElement, meta.toolCallId);
                    }
                }
            }
        } else {
            messageDiv.innerHTML = renderContent(content);
        }

        addCodeActions(messageDiv, context.vscode);

        messagesContainer.appendChild(messageDiv);
        messagesContainer.scrollTop = messagesContainer.scrollHeight;

        conversation.messages.push(chatMessage);
    }

    function toggleReasoning(reasoningId: string): void {
        const content = document.getElementById(reasoningId);
        const toggle = document.getElementById(`${reasoningId}-toggle`);
        if (!content || !toggle) return;

        const truncated = content.querySelector<HTMLElement>('.reasoning-truncated');
        const full = content.querySelector<HTMLElement>('.reasoning-full');
        if (!truncated || !full) return;

        const isToggleable = reasoningToggleState.get(reasoningId);
        if (!isToggleable) return;

        if (content.classList.contains('collapsed')) {
            content.classList.remove('collapsed');
            content.classList.add('expanded');
            truncated.style.display = 'none';
            full.style.display = 'block';
            toggle.textContent = '▼';
        } else {
            content.classList.remove('expanded');
            content.classList.add('collapsed');
            truncated.style.display = 'block';
            full.style.display = 'none';
            toggle.textContent = '▶';
        }
    }

    function setActiveConversation(id: string): void {
        context.activeConversationId = id;

        document.querySelectorAll<HTMLDivElement>('.tab').forEach(tab => {
            if (tab.dataset.conversationId === id) {
                tab.classList.add('active');
            } else {
                tab.classList.remove('active');
            }
        });

        document.querySelectorAll<HTMLDivElement>('.conversation-view').forEach(view => {
            if (view.dataset.conversationId === id) {
                view.style.display = 'flex';
                const input = view.querySelector<HTMLTextAreaElement>('.message-input');
                input?.focus();
            } else {
                view.style.display = 'none';
            }
        });
    }

    function showWelcomeScreen(): void {
        context.dom.welcomeScreen.style.display = 'flex';
        context.dom.tabBar.style.display = 'none';
        context.dom.conversationsContainer.style.display = 'none';
    }

    function showConversations(): void {
        context.dom.welcomeScreen.style.display = 'none';
        context.dom.tabBar.style.display = 'flex';
        context.dom.conversationsContainer.style.display = 'flex';
    }

    function startEditingTitle(
        conversationId: string,
        tab: HTMLElement,
        titleElement: HTMLElement,
        inputElement: HTMLInputElement
    ): void {
        titleElement.style.display = 'none';
        inputElement.style.display = 'block';
        inputElement.value = titleElement.textContent || '';
        inputElement.select();
        inputElement.focus();
        tab.classList.add('editing');
    }

    function saveTabTitle(
        conversationId: string,
        tab: HTMLElement,
        titleElement: HTMLElement,
        inputElement: HTMLInputElement
    ): void {
        const newTitle = inputElement.value.trim();

        if (!newTitle) {
            cancelEditingTitle(tab, titleElement, inputElement);
            return;
        }

        if (newTitle !== (titleElement.textContent || '')) {
            context.vscode.postMessage({
                type: 'renameTab',
                conversationId,
                title: newTitle
            });

            titleElement.textContent = newTitle;
            inputElement.value = newTitle;
        }

        inputElement.style.display = 'none';
        titleElement.style.display = 'block';
        tab.classList.remove('editing');
    }

    function cancelEditingTitle(
        tab: HTMLElement,
        titleElement: HTMLElement,
        inputElement: HTMLInputElement
    ): void {
        inputElement.value = titleElement.textContent || '';
        inputElement.style.display = 'none';
        titleElement.style.display = 'block';
        tab.classList.remove('editing');
    }

    function showTabContextMenu(
        event: MouseEvent,
        conversationId: string,
        tab: HTMLElement,
        titleElement: HTMLElement,
        inputElement: HTMLInputElement
    ): void {
        const existingMenu = document.querySelector('.tab-context-menu');
        existingMenu?.remove();

        const menu = document.createElement('div');
        menu.className = 'tab-context-menu';
        menu.style.position = 'fixed';
        menu.style.left = `${event.clientX}px`;
        menu.style.top = `${event.clientY}px`;
        menu.innerHTML = `
            <div class="context-menu-item" data-action="rename">
                <span class="context-menu-icon">✏️</span>
                Rename
            </div>
            <div class="context-menu-item" data-action="close">
                <span class="context-menu-icon">✖️</span>
                Close
            </div>
        `;

        menu.addEventListener('click', (e: MouseEvent) => {
            const target = e.target as HTMLElement;
            const item = target.closest('.context-menu-item') as HTMLElement | null;
            if (!item) return;

            const action = item.dataset.action;
            if (action === 'rename') {
                startEditingTitle(conversationId, tab, titleElement, inputElement);
            } else if (action === 'close') {
                context.vscode.postMessage({ type: 'closeTab', conversationId });
            }
            menu.remove();
        });

        setTimeout(() => {
            document.addEventListener('click', function closeMenu(this: Document, e: MouseEvent) {
                const target = e.target as Node;
                if (!menu.contains(target)) {
                    menu.remove();
                    document.removeEventListener('click', closeMenu);
                }
            });
        }, 0);

        document.body.appendChild(menu);
    }

    function formatToolResult(
        toolResult: any,
        diffId?: string | null,
        toolContext?: ToolContext,
        filePath?: string
    ): string {
        if (!toolResult) {
            return '';
        }

        if (typeof toolResult !== 'object' || !toolResult.kind) {
            return `<pre>${escapeHtml(JSON.stringify(toolResult, null, 2))}</pre>`;
        }

        switch (toolResult.kind) {
            case 'ModifyFile': {
                const diffButton = diffId ? `<button class="view-diff-button" data-diff-id="${diffId}">📝 View Diff</button>` : '';
                const filePathDisplay = filePath ? escapeHtml(filePath) : 'file';
                let content = `<div class="tool-file-result">
                    <span class="tool-success-message">✓ Modified: ${filePathDisplay}</span>
                    ${diffButton}
                </div>`;
                content += `<div class="tool-detail">+${toolResult.lines_added} -${toolResult.lines_removed} lines</div>`;
                return content;
            }

            case 'RunCommand': {
                const exitStatus = toolResult.exit_code === 0 ? '✓' : '⚠';
                const command = toolContext?.command?.trim();

                let commandLine = '';
                if (command) {
                    commandLine = `<div class="tool-command-highlight"><code>${escapeHtml(command)}</code></div>`;
                }

                let content = `${commandLine}<div class="tool-success-message">${exitStatus} Exit code: ${toolResult.exit_code}</div>`;
                if (toolResult.stdout) {
                    content += `<details><summary>Output</summary><pre>${escapeHtml(toolResult.stdout)}</pre></details>`;
                }
                if (toolResult.stderr) {
                    content += `<details><summary>Errors</summary><pre>${escapeHtml(toolResult.stderr)}</pre></details>`;
                }
                return content;
            }

            case 'ReadFiles': {
                const fileCount = toolResult.files?.length || 0;
                let content = `<div class="tool-success-message">✓ Read ${fileCount} file${fileCount === 1 ? '' : 's'}</div>`;
                if (toolResult.files && toolResult.files.length > 0) {
                    for (const file of toolResult.files) {
                        const size = formatBytes(file.bytes);
                        content += `<div class="tool-detail"><code>${escapeHtml(file.path)}</code> — ${size}</div>`;
                    }
                }
                return content;
            }

            case 'Error': {
                return `<div class="tool-error-message">${escapeHtml(toolResult.message)}</div>`;
            }

            case 'Other': {
                const result = toolResult.result;
                if (!result) {
                    return '';
                }

                if (result.content) {
                    const lines = result.content.split('\n').length;
                    return `<div class="tool-success-message">✓ Read ${lines} lines</div>`;
                }

                if (Array.isArray(result.entries)) {
                    return `<div class="tool-success-message">✓ Found ${result.entries.length} entries</div>`;
                }

                return `<pre>${escapeHtml(JSON.stringify(result, null, 2))}</pre>`;
            }

            default:
                return `<pre>${escapeHtml(JSON.stringify(toolResult, null, 2))}</pre>`;
        }
    }

    return {
        handleInitialState,
        handleConversationCreated,
        handleConversationMessage,
        handleActiveConversationChanged,
        handleConversationClosed,
        handleConversationTitleChanged,
        handleConversationCleared,
        handleShowTyping,
        handleRetryAttempt,
        handleConversationDisconnected,
        handleToolRequest,
        handleToolResult,
        handleProviderConfig,
        handleProviderSwitched,
        registerGlobalListeners
    };
}
