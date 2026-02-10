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
    ProfileConfigMessage,
    ProfileSwitchedMessage,
    RetryAttemptMessage,
    SettingsUpdateMessage,
    ShowTypingMessage,
    PendingToolUpdate,
    ToolRequestMessage,
    ToolResultMessage,
    TaskUpdateMessage,
    StreamStartMessage,
    StreamDeltaMessage,
    StreamReasoningDeltaMessage,
    StreamEndMessage,
    AddImageDataMessage
} from './types.js';
import {
    addCodeActions,
    addMessageCopyButton,
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
    handleProfileConfig(message: ProfileConfigMessage): void;
    handleProfileSwitched(message: ProfileSwitchedMessage): void;
    handleSettingsUpdate(message: SettingsUpdateMessage): void;
    handleTaskUpdate(message: TaskUpdateMessage): void;
    handleSessionsListUpdate(sessions: any[]): void;
    handleStreamStart(message: StreamStartMessage): void;
    handleStreamDelta(message: StreamDeltaMessage): void;
    handleStreamReasoningDelta(message: StreamReasoningDeltaMessage): void;
    handleStreamEnd(message: StreamEndMessage): void;
    handleAddImageData(message: AddImageDataMessage): void;
    registerGlobalListeners(): void;
}

function isNearBottom(container: HTMLElement, threshold = 50): boolean {
    return container.scrollHeight - container.scrollTop - container.clientHeight <= threshold;
}

function scrollToBottom(container: HTMLElement): void {
    container.scrollTop = container.scrollHeight;
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
        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        const wasNearBottom = messagesContainer ? isNearBottom(messagesContainer) : false;

        toolItem.classList.remove('tool-hidden');

        const toolCallsContainer = toolItem.closest('.embedded-tool-calls');
        if (toolCallsContainer) {
            toolCallsContainer.classList.remove('tool-hidden');
        }

        const statusIcon = toolItem.querySelector<HTMLElement>('.tool-status-icon');
        const statusText = toolItem.querySelector<HTMLElement>('.tool-status-text');
        if (statusIcon) statusIcon.textContent = '🔧';
        if (statusText) statusText.textContent = 'Running';

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

        if (wasNearBottom && messagesContainer) {
            scrollToBottom(messagesContainer);
        }
    }

    function applyToolResult(
        conversation: ConversationState,
        toolItem: HTMLElement,
        message: ToolResultMessage
    ): void {
        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        const wasNearBottom = messagesContainer ? isNearBottom(messagesContainer) : false;

        toolItem.classList.remove('tool-hidden');

        const toolCallsContainer = toolItem.closest('.embedded-tool-calls');
        if (toolCallsContainer) {
            toolCallsContainer.classList.remove('tool-hidden');
        }

        const preview = toolItem.querySelector<HTMLElement>('.tool-preview');
        if (preview) {
            preview.style.display = 'none';
        }

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
            const resultElement = formatToolResult(message.tool_result, diffId, toolContext, filePath);
            resultDiv.replaceChildren(resultElement);
        }

        const debugSection = toolItem.querySelector<HTMLDivElement>('.tool-debug-section');
        if (debugSection) {
            const debugResponse = debugSection.querySelector<HTMLDivElement>('.tool-debug-response');
            if (debugResponse) {
                const responseData = message.tool_result;
                if (responseData) {
                    debugResponse.innerHTML = `<strong>Response:</strong><pre>${escapeHtml(JSON.stringify(responseData, null, 2))}</pre>`;
                } else {
                    debugResponse.innerHTML = '';
                }
            }
        }

        if (wasNearBottom && messagesContainer) {
            scrollToBottom(messagesContainer);
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

    function generateToolPreview(toolCall: any): string {
        if (!toolCall.arguments || typeof toolCall.arguments !== 'object') {
            return '';
        }

        const args = toolCall.arguments as Record<string, unknown>;
        if (typeof args.command === 'string') {
            return `<div class="tool-preview"><code>${escapeHtml(args.command)}</code></div>`;
        }
        if (typeof args.file_path === 'string') {
            return `<div class="tool-preview"><code>${escapeHtml(args.file_path)}</code></div>`;
        }
        return '';
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

        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        const wasNearBottom = messagesContainer ? isNearBottom(messagesContainer) : false;

        const typingIndicator = conversation.viewElement.querySelector<HTMLDivElement>('.typing-indicator');
        if (typingIndicator) {
            typingIndicator.style.display = message.show ? 'flex' : 'none';
        }

        if (message.show && wasNearBottom && messagesContainer) {
            scrollToBottom(messagesContainer);
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

        const wasNearBottom = isNearBottom(messagesContainer);
        messagesContainer.appendChild(retryElement);
        if (wasNearBottom) {
            scrollToBottom(messagesContainer);
        }
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

    function handleProfileConfig(message: ProfileConfigMessage): void {
        const { conversationId, profiles, selectedProfile } = message;
        const conversation = context.store.get(conversationId);
        if (!conversation) return;

        const profileSelect = conversation.viewElement.querySelector<HTMLSelectElement>('.profile-select');
        if (!profileSelect) return;

        profileSelect.innerHTML = '';

        // Use provided selectedProfile, or preserve existing selection from settingsUpdate
        const effectiveProfile = selectedProfile ?? conversation.selectedProfile;

        if (profiles && profiles.length > 0) {
            profiles.forEach(profile => {
                const option = document.createElement('option');
                option.value = profile;
                option.textContent = profile;
                if (effectiveProfile && profile === effectiveProfile) {
                    option.selected = true;
                }
                profileSelect.appendChild(option);
            });
        } else {
            const option = document.createElement('option');
            option.value = 'default';
            option.textContent = 'default';
            option.selected = true;
            profileSelect.appendChild(option);
        }

        // Only update state if a new profile was explicitly provided
        if (selectedProfile) {
            conversation.selectedProfile = selectedProfile;
        }
    }

    function handleProfileSwitched(message: ProfileSwitchedMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const profileSelect = conversation.viewElement.querySelector<HTMLSelectElement>('.profile-select');
        if (profileSelect && message.newProfile) {
            profileSelect.value = message.newProfile;
        }

        conversation.selectedProfile = message.newProfile;
    }

    function handleSettingsUpdate(message: SettingsUpdateMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const autonomySlider = conversation.viewElement.querySelector<HTMLInputElement>('.autonomy-slider');
        const autonomyValue = conversation.viewElement.querySelector<HTMLSpanElement>('.settings-slider-value');

        if (autonomySlider && autonomyValue) {
            const value = message.autonomyLevel === 'fully_autonomous' ? 2 : 1;
            autonomySlider.value = String(value);
            autonomyValue.textContent = value === 2 ? 'Fully Autonomous' : 'Plan Approval';
        }

        if (message.defaultAgent) {
            const orchestrationSlider = conversation.viewElement.querySelector<HTMLInputElement>('.orchestration-slider');
            const orchestrationValue = conversation.viewElement.querySelector<HTMLSpanElement>('.orchestration-slider-value');

            if (orchestrationSlider && orchestrationValue) {
                const agentToValue: Record<string, number> = { 'one_shot': 1, 'tycode': 2, 'coordinator': 3 };
                const valueToLabel: Record<number, string> = { 1: 'None', 2: 'Auto', 3: 'Required' };
                const value = agentToValue[message.defaultAgent] ?? 2;
                orchestrationSlider.value = String(value);
                orchestrationValue.textContent = valueToLabel[value] ?? 'Auto';
            }
        }

        if (message.reasoningEffort) {
            const reasoningSlider = conversation.viewElement.querySelector<HTMLInputElement>('.reasoning-slider');
            const reasoningValue = conversation.viewElement.querySelector<HTMLSpanElement>('.reasoning-slider-value');
            if (reasoningSlider && reasoningValue) {
                const effortToValue: Record<string, number> = { 'Off': 0, 'Low': 1, 'Medium': 2, 'High': 3, 'Max': 4 };
                const value = effortToValue[message.reasoningEffort] ?? 3;
                reasoningSlider.value = String(value);
                reasoningValue.textContent = message.reasoningEffort;
            }
        }

        if (message.profile) {
            const profileSelect = conversation.viewElement.querySelector<HTMLSelectElement>('.profile-select');
            if (profileSelect) {
                profileSelect.value = message.profile;
            }
            conversation.selectedProfile = message.profile;
        }
    }

    function handleTaskUpdate(message: TaskUpdateMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        if (!conversation.taskListState) {
            conversation.taskListState = {
                title: message.taskList.title,
                tasks: [],
                isExpanded: false
            };
        }

        conversation.taskListState.title = message.taskList.title;
        conversation.taskListState.tasks = message.taskList.tasks;
        renderTaskList(conversation);
    }

    function renderTaskList(conversation: ConversationState): void {
        const taskListContainer = conversation.viewElement.querySelector<HTMLDivElement>('.task-list-container');
        if (!taskListContainer || !conversation.taskListState) return;

        const { tasks, isExpanded, title } = conversation.taskListState;

        if (tasks.length === 0) {
            taskListContainer.style.display = 'none';
            return;
        }

        taskListContainer.style.display = 'block';

        const completedCount = tasks.filter(t => t.status === 'completed').length;
        const totalCount = tasks.length;

        const getStatusIcon = (status: string): string => {
            switch (status) {
                case 'completed': return '✓';
                case 'in_progress': return '⟳';
                case 'failed': return '✗';
                case 'pending': return '•';
                default: return '•';
            }
        };

        const getStatusClass = (status: string): string => {
            switch (status) {
                case 'completed': return 'task-completed';
                case 'in_progress': return 'task-in-progress';
                case 'failed': return 'task-failed';
                case 'pending': return 'task-pending';
                default: return 'task-pending';
            }
        };

        let taskToDisplay = tasks.find(t => t.status === 'in_progress');
        if (!taskToDisplay) {
            taskToDisplay = tasks.find(t => t.status === 'pending');
        }

        let tasksHtml = '';
        if (isExpanded) {
            tasksHtml = tasks.map(task => `
                <div class="task-item ${getStatusClass(task.status)}">
                    <span class="task-status-icon">${getStatusIcon(task.status)}</span>
                    <span class="task-text">Task ${task.id}: ${escapeHtml(task.description)}</span>
                </div>
            `).join('');
        } else if (taskToDisplay) {
            tasksHtml = `
                <div class="task-item ${getStatusClass(taskToDisplay.status)}">
                    <span class="task-status-icon">${getStatusIcon(taskToDisplay.status)}</span>
                    <span class="task-text">Task ${taskToDisplay.id}: ${escapeHtml(taskToDisplay.description)}</span>
                </div>
            `;
        } else {
            const allCompleted = tasks.every(t => t.status === 'completed');
            if (allCompleted) {
                tasksHtml = `
                    <div class="task-item task-completed">
                        <span class="task-status-icon">✓</span>
                        <span class="task-text">All tasks completed!</span>
                    </div>
                `;
            }
        }

        const expandIcon = isExpanded ? '▼' : '▶';
        const progressText = `${completedCount}/${totalCount} tasks completed`;
        const taskListTitle = title || 'Tasks';

        taskListContainer.innerHTML = `
            <div class="task-list-header">
                <div class="task-list-title">
                    <span class="task-list-expand-icon">${expandIcon}</span>
                    <span class="task-list-heading">${escapeHtml(taskListTitle)}</span>
                    <span class="task-list-progress">${progressText}</span>
                </div>
            </div>
            <div class="task-list-items">
                ${tasksHtml}
            </div>
        `;

        const header = taskListContainer.querySelector<HTMLDivElement>('.task-list-header');
        if (header) {
            header.addEventListener('click', () => toggleTaskList(conversation));
        }
    }

    function toggleTaskList(conversation: ConversationState): void {
        if (!conversation.taskListState) return;

        conversation.taskListState.isExpanded = !conversation.taskListState.isExpanded;
        renderTaskList(conversation);
    }

    function handleAddImageData(message: AddImageDataMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        if (!conversation.pendingImages) {
            conversation.pendingImages = [];
        }

        conversation.pendingImages.push({
            media_type: message.media_type,
            data: message.data,
            name: message.name
        });

        renderThumbnails(message.conversationId);
    }

    function setupDocumentDropHandlers(): void {
        document.addEventListener('dragenter', (e: DragEvent) => {
            e.preventDefault();
            e.stopPropagation();
        });

        document.addEventListener('dragover', (e: DragEvent) => {
            e.preventDefault();
            e.stopPropagation();
            if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
        });

        document.addEventListener('drop', (e: DragEvent) => {
            e.preventDefault();
            e.stopPropagation();

            if (!context.activeConversationId) return;
            const conversationId = context.activeConversationId;

            const files = e.dataTransfer?.files;
            if (files && files.length > 0) {
                for (let i = 0; i < files.length; i++) {
                    const file = files[i];
                    if (!file.type.startsWith('image/')) continue;
                    readImageFile(conversationId, file);
                }
                return;
            }

            const uriList = e.dataTransfer?.getData('text/uri-list');
            if (uriList) {
                const uris = uriList.split(/\r?\n/).filter(u => u && !u.startsWith('#'));
                for (const uri of uris) {
                    context.vscode.postMessage({
                        type: 'imageDropped',
                        conversationId,
                        uri
                    });
                }
            }
        });
    }

    function registerGlobalListeners(): void {
        setupDocumentDropHandlers();

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
            <span class="tab-title" title="Right-click to rename">${escapeHtml(title)}</span>
            <input class="tab-title-input" type="text" value="${escapeHtml(title)}" style="display: none;">
            <button class="tab-close" title="Close">×</button>
        `;

        const tabTitle = tab.querySelector<HTMLSpanElement>('.tab-title');
        const tabInput = tab.querySelector<HTMLInputElement>('.tab-title-input');
        const tabCloseButton = tab.querySelector<HTMLButtonElement>('.tab-close');
        if (!tabTitle || !tabInput || !tabCloseButton) {
            throw new Error('Tab template is missing expected elements');
        }

        tab.addEventListener('click', (e: MouseEvent) => {
            const target = e.target as HTMLElement;
            if (!target.classList.contains('tab-close') && !tab.classList.contains('editing')) {
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
            <div class="task-list-container" style="display: none;"></div>
            <div class="messages-wrapper">
                <div class="messages"></div>
                <button class="scroll-to-bottom" style="display: none;" title="Scroll to bottom">↓</button>
            </div>
            <div class="typing-indicator" style="display: none;">
                <span></span>
                <span></span>
                <span></span>
            </div>
            <div class="input-container">
                <div class="image-thumbnails" style="display: none;"></div>
                <div class="input-row">
                    <textarea class="message-input" placeholder="Ask me anything about your code..." rows="3"></textarea>
                    <div class="input-buttons">
                        <button class="attach-image-button" title="Attach image">📎</button>
                        <button class="send-button">Send</button>
                        <button class="cancel-button" style="display: none;">Cancel</button>
                    </div>
                </div>
            </div>
            <div class="settings-panel">
                <div class="settings-toggle">
                    <span class="settings-toggle-icon">▲</span>
                    <span class="settings-toggle-text">Session Settings</span>
                </div>
                <div class="settings-content">
                    <div class="settings-grid">
                        <div class="settings-item">
                            <label class="settings-label" data-tooltip="Controls whether the agent asks for approval before executing plans or runs fully autonomous">Autonomy</label>
                            <div class="settings-control">
                                <div class="settings-slider-container">
                                    <input type="range" class="settings-slider autonomy-slider" min="1" max="2" value="1">
                                    <span class="settings-slider-value">Plan Approval</span>
                                </div>
                            </div>
                        </div>
                        <div class="settings-item">
                            <label class="settings-label" data-tooltip="Controls task delegation: None runs single-agent, Auto breaks tasks into sub-agents as needed, Required always delegates">Orchestration</label>
                            <div class="settings-control">
                                <div class="settings-slider-container">
                                    <input type="range" class="settings-slider orchestration-slider" min="1" max="3" value="2">
                                    <span class="orchestration-slider-value">Auto</span>
                                </div>
                            </div>
                        </div>
                        <div class="settings-item">
                            <label class="settings-label" data-tooltip="How deeply the AI thinks before responding. Higher = better quality but slower and more expensive. Lower = faster and cheaper">Reasoning</label>
                            <div class="settings-control">
                                <div class="settings-slider-container">
                                    <input type="range" class="settings-slider reasoning-slider" min="0" max="4" value="3">
                                    <span class="reasoning-slider-value">High</span>
                                </div>
                            </div>
                        </div>
                        <div class="settings-item">
                            <label class="settings-label">Profile</label>
                            <div class="settings-control">
                                <select id="profile-select-${id}" class="settings-select profile-select">
                                    <option value="loading">Loading...</option>
                                </select>
                                <button class="settings-refresh-btn refresh-profiles" title="Refresh profiles">↻</button>
                                <button class="settings-refresh-btn save-session-defaults" data-tooltip="Save current settings as defaults for this profile">💾</button>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        `;

        const messageInput = conversationView.querySelector<HTMLTextAreaElement>('.message-input');
        const sendButton = conversationView.querySelector<HTMLButtonElement>('.send-button');
        const cancelButton = conversationView.querySelector<HTMLButtonElement>('.cancel-button');
        const profileSelect = conversationView.querySelector<HTMLSelectElement>('.profile-select');
        const refreshProfilesBtn = conversationView.querySelector<HTMLButtonElement>('.refresh-profiles');

        if (!messageInput || !sendButton || !cancelButton) {
            throw new Error('Conversation view missing expected controls');
        }

        sendButton.addEventListener('click', () => sendMessage(id, messageInput));

        cancelButton.addEventListener('click', () => {
            const pendingMessage = messageInput.value.trim();

            const conv = context.store.get(id);
            if (conv?.streamingElement) {
                conv.streamingElement.classList.remove('streaming');
                conv.streamingElement = undefined;
                conv.streamingText = undefined;
                conv.streamingReasoningText = undefined;
                conv.streamingAutoScroll = undefined;
            }

            const sBtn = conversationView.querySelector<HTMLButtonElement>('.send-button');
            const cBtn = conversationView.querySelector<HTMLButtonElement>('.cancel-button');
            if (sBtn && cBtn) {
                sBtn.style.display = 'block';
                cBtn.style.display = 'none';
            }
            if (conv) {
                conv.isProcessing = false;
            }

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

        if (profileSelect) {
            profileSelect.addEventListener('change', (e: Event) => {
                const target = e.target as HTMLSelectElement;
                const selectedProfile = target.value;
                context.vscode.postMessage({
                    type: 'switchProfile',
                    conversationId: id,
                    profile: selectedProfile
                });
            });
        }

        if (refreshProfilesBtn) {
            refreshProfilesBtn.addEventListener('click', () => {
                context.vscode.postMessage({
                    type: 'refreshProfiles',
                    conversationId: id
                });
            });
        }

        const saveDefaultsBtn = conversationView.querySelector<HTMLButtonElement>('.save-session-defaults');
        if (saveDefaultsBtn) {
            saveDefaultsBtn.addEventListener('click', () => {
                const aSlider = conversationView.querySelector<HTMLInputElement>('.autonomy-slider');
                const rSlider = conversationView.querySelector<HTMLInputElement>('.reasoning-slider');
                const oSlider = conversationView.querySelector<HTMLInputElement>('.orchestration-slider');

                const autonomyLevel = aSlider && parseInt(aSlider.value, 10) === 2
                    ? 'fully_autonomous' as const : 'plan_approval_required' as const;

                const reasoningLabels = ['Off', 'Low', 'Medium', 'High', 'Max'] as const;
                const reasoningEffort = rSlider
                    ? reasoningLabels[parseInt(rSlider.value, 10)] || 'High'
                    : 'High';

                const orchestrationAgents = ['one_shot', 'tycode', 'coordinator'];
                const defaultAgent = oSlider
                    ? orchestrationAgents[parseInt(oSlider.value, 10) - 1] || 'tycode'
                    : 'tycode';

                context.vscode.postMessage({
                    type: 'saveSessionDefaults',
                    conversationId: id,
                    autonomyLevel,
                    reasoningEffort,
                    defaultAgent
                });
            });
        }

        const settingsPanel = conversationView.querySelector<HTMLDivElement>('.settings-panel');
        const settingsToggle = conversationView.querySelector<HTMLDivElement>('.settings-toggle');
        const autonomySlider = conversationView.querySelector<HTMLInputElement>('.autonomy-slider');
        const autonomyValue = conversationView.querySelector<HTMLSpanElement>('.settings-slider-value');
        const orchestrationSlider = conversationView.querySelector<HTMLInputElement>('.orchestration-slider');
        const orchestrationValue = conversationView.querySelector<HTMLSpanElement>('.orchestration-slider-value');

        const scrollBtn = conversationView.querySelector<HTMLButtonElement>('.scroll-to-bottom');
        const msgsContainer = conversationView.querySelector<HTMLDivElement>('.messages');
        if (msgsContainer && scrollBtn) {
            msgsContainer.addEventListener('scroll', () => {
                scrollBtn.style.display = isNearBottom(msgsContainer) ? 'none' : 'flex';
            });
            scrollBtn.addEventListener('click', () => {
                scrollToBottom(msgsContainer);
            });
        }

        if (settingsToggle && settingsPanel) {
            settingsToggle.addEventListener('click', () => {
                settingsPanel.classList.toggle('expanded');
            });
        }

        if (autonomySlider && autonomyValue) {
            const autonomyLabels = ['Plan Approval', 'Fully Autonomous'];
            autonomySlider.addEventListener('input', () => {
                const value = parseInt(autonomySlider.value, 10);
                autonomyValue.textContent = autonomyLabels[value - 1] || 'Plan Approval';
                context.vscode.postMessage({
                    type: 'setAutonomyLevel',
                    conversationId: id,
                    autonomyLevel: value === 2 ? 'fully_autonomous' : 'plan_approval_required'
                });
            });
        }

        if (orchestrationSlider && orchestrationValue) {
            const orchestrationLabels = ['None', 'Auto', 'Required'];
            const orchestrationAgents = ['one_shot', 'tycode', 'coordinator'];
            orchestrationSlider.addEventListener('input', () => {
                const value = parseInt(orchestrationSlider.value, 10);
                orchestrationValue.textContent = orchestrationLabels[value - 1] || 'Auto';
                const agentName = orchestrationAgents[value - 1] || 'tycode';
                context.vscode.postMessage({
                    type: 'sendMessage',
                    conversationId: id,
                    message: `/agent ${agentName}`
                });
            });
        }

        const reasoningSlider = conversationView.querySelector<HTMLInputElement>('.reasoning-slider');
        const reasoningValue = conversationView.querySelector<HTMLSpanElement>('.reasoning-slider-value');
        if (reasoningSlider && reasoningValue) {
            const reasoningLabels = ['Off', 'Low', 'Medium', 'High', 'Max'] as const;
            reasoningSlider.addEventListener('input', () => {
                const value = parseInt(reasoningSlider.value, 10);
                const label = reasoningLabels[value] || 'High';
                reasoningValue.textContent = label;
                context.vscode.postMessage({
                    type: 'setReasoningEffort',
                    conversationId: id,
                    reasoningEffort: label
                });
            });
        }

        const inputContainer = conversationView.querySelector<HTMLDivElement>('.input-container');
        if (inputContainer) {
            inputContainer.addEventListener('dragover', (e: DragEvent) => {
                e.preventDefault();
                e.stopPropagation();
                if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
                inputContainer.classList.add('drag-over');
            });

            inputContainer.addEventListener('dragleave', (e: DragEvent) => {
                e.preventDefault();
                e.stopPropagation();
                inputContainer.classList.remove('drag-over');
            });

            inputContainer.addEventListener('drop', (e: DragEvent) => {
                e.preventDefault();
                e.stopPropagation();
                inputContainer.classList.remove('drag-over');

                const files = e.dataTransfer?.files;
                if (files && files.length > 0) {
                    for (let i = 0; i < files.length; i++) {
                        const file = files[i];
                        if (!file.type.startsWith('image/')) continue;
                        readImageFile(id, file);
                    }
                    return;
                }

                const uriList = e.dataTransfer?.getData('text/uri-list');
                if (uriList) {
                    const uris = uriList.split(/\r?\n/).filter(u => u && !u.startsWith('#'));
                    for (const uri of uris) {
                        context.vscode.postMessage({
                            type: 'imageDropped',
                            conversationId: id,
                            uri
                        });
                    }
                }
            });
        }

        if (messageInput) {
            messageInput.addEventListener('paste', (e: ClipboardEvent) => {
                const items = e.clipboardData?.items;
                if (!items) return;

                for (let i = 0; i < items.length; i++) {
                    const item = items[i];
                    if (!item.type.startsWith('image/')) continue;

                    const file = item.getAsFile();
                    if (!file) continue;
                    readImageFile(id, file);
                }
            });
        }

        const attachButton = conversationView.querySelector<HTMLButtonElement>('.attach-image-button');
        if (attachButton) {
            attachButton.addEventListener('click', () => {
                const fileInput = document.createElement('input');
                fileInput.type = 'file';
                fileInput.accept = 'image/png,image/jpeg,image/gif,image/webp';
                fileInput.multiple = true;
                fileInput.addEventListener('change', () => {
                    if (!fileInput.files) return;
                    for (let i = 0; i < fileInput.files.length; i++) {
                        readImageFile(id, fileInput.files[i]);
                    }
                });
                fileInput.click();
            });
        }

        context.dom.conversationsContainer.appendChild(conversationView);

        const state: ConversationState = {
            title,
            messages: [],
            tabElement: tab,
            viewElement: conversationView,
            selectedProfile: null,
            pendingToolUpdates: new Map<string, PendingToolUpdate>()
        };

        context.store.set(id, state);

        context.vscode.postMessage({
            type: 'getProfiles',
            conversationId: id
        });

        context.vscode.postMessage({
            type: 'getSettings',
            conversationId: id
        });
    }

    function sendMessage(conversationId: string, inputElement: HTMLTextAreaElement): void {
        const conversation = context.store.get(conversationId);
        const message = inputElement.value.trim();
        if (!message && (!conversation?.pendingImages || conversation.pendingImages.length === 0)) return;

        inputElement.value = '';
        inputElement.style.height = 'auto';

        const images = conversation?.pendingImages?.map(({ media_type, data }) => ({ media_type, data }));

        const outbound: any = {
            type: 'sendMessage',
            conversationId,
            message: message || ' '
        };
        if (images && images.length > 0) {
            outbound.images = images;
        }
        context.vscode.postMessage(outbound);

        if (conversation) {
            conversation.pendingImages = undefined;
            const thumbnailArea = conversation.viewElement.querySelector<HTMLDivElement>('.image-thumbnails');
            if (thumbnailArea) {
                thumbnailArea.innerHTML = '';
                thumbnailArea.style.display = 'none';
            }
        }
    }

    function readImageFile(conversationId: string, file: File): void {
        const conversation = context.store.get(conversationId);
        if (!conversation) return;

        const validTypes = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];
        if (!validTypes.includes(file.type)) return;

        // 20MB limit
        if (file.size > 20 * 1024 * 1024) return;

        const reader = new FileReader();
        reader.onload = () => {
            const result = reader.result as string;
            const base64 = result.split(',')[1];
            if (!base64) return;

            if (!conversation.pendingImages) {
                conversation.pendingImages = [];
            }

            conversation.pendingImages.push({
                media_type: file.type,
                data: base64,
                name: file.name
            });

            renderThumbnails(conversationId);
        };
        reader.readAsDataURL(file);
    }

    function renderThumbnails(conversationId: string): void {
        const conversation = context.store.get(conversationId);
        if (!conversation) return;

        const thumbnailArea = conversation.viewElement.querySelector<HTMLDivElement>('.image-thumbnails');
        if (!thumbnailArea) return;

        const images = conversation.pendingImages;
        if (!images || images.length === 0) {
            thumbnailArea.style.display = 'none';
            thumbnailArea.innerHTML = '';
            return;
        }

        thumbnailArea.style.display = 'flex';
        thumbnailArea.innerHTML = '';

        images.forEach((img, index) => {
            const thumb = document.createElement('div');
            thumb.className = 'image-thumbnail';
            thumb.innerHTML = `
                <img src="data:${escapeHtml(img.media_type)};base64,${img.data}" alt="${escapeHtml(img.name || 'image')}" />
                <button class="thumbnail-remove" data-index="${index}" title="Remove">×</button>
            `;

            const removeBtn = thumb.querySelector<HTMLButtonElement>('.thumbnail-remove');
            if (removeBtn) {
                removeBtn.addEventListener('click', () => {
                    if (conversation.pendingImages) {
                        conversation.pendingImages.splice(index, 1);
                        if (conversation.pendingImages.length === 0) {
                            conversation.pendingImages = undefined;
                        }
                    }
                    renderThumbnails(conversationId);
                });
            }

            thumbnailArea.appendChild(thumb);
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
            const modelInfo = model ? `<div class="model-info">${model}</div>` : '';

        let agentInfo = '';
        if (chatMessage.sender && typeof chatMessage.sender === 'object' && 'Assistant' in chatMessage.sender) {
            const assistant = chatMessage.sender.Assistant as Record<string, unknown>;
            const agentType = assistant?.agent;
            if (agentType && typeof agentType === 'string') {
                agentInfo = `<div class="agent-info">${agentType}</div>`;
            }
        }

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
            let taskListNote = '';
            const toolCallMetadata: Array<{ elementId: string; toolCallId: string; command?: string }>
                = [];
            if (toolCalls && toolCalls.length > 0) {
                const taskListTools = toolCalls.filter((tc: any) => tc.name === 'manage_task_list');
                const otherTools = toolCalls.filter((tc: any) => tc.name !== 'manage_task_list');

                if (taskListTools.length > 0) {
                    taskListNote = '<div class="task-list-note">Task list updated</div>';
                }

                const toolCallsHtml = otherTools.map((toolCall: any) => {
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

                    const previewHtml = generateToolPreview(toolCall);

                    return `
                        <div class="tool-call-item" data-tool-name="${toolCall.name}" data-tool-call-id="${toolCallId}" data-conversation-id="${conversationId}" id="${toolId}">
                            <div class="tool-header">
                                <span class="tool-status-icon">⏳</span>
                                <span class="tool-name">${toolCall.name}</span>
                                <span class="tool-status-text">Pending</span>
                                <span class="tool-debug-toggle">▶</span>
                            </div>
                            ${previewHtml}
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

                if (otherTools.length > 0) {
                    toolCallsSection = `
                        <div class="embedded-tool-calls">
                            ${toolCallsHtml}
                        </div>
                    `;
                }
            }

            messageDiv.innerHTML = `
                ${modelInfo}
                ${agentInfo}
                ${tokenInfo}
                ${reasoningSection}
                <div class="message-content">${renderContent(content)}</div>
                ${toolCallsSection}
                <div class="message-footer">${taskListNote}</div>
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
            let html = renderContent(content);

            const images = chatMessage.images || chatMessage.pendingImages;
            if (images && images.length > 0) {
                html += '<div class="message-images">';
                for (const img of images) {
                    html += `<img class="message-image" src="data:${escapeHtml(img.media_type)};base64,${img.data}" alt="Attached image" />`;
                }
                html += '</div>';
            }

            messageDiv.innerHTML = html;
        }

        addCodeActions(messageDiv, context.vscode);

        if (role === 'assistant') {
            addMessageCopyButton(messageDiv, content, context.vscode);
        }

        const wasNearBottom = isNearBottom(messagesContainer);
        messagesContainer.appendChild(messageDiv);
        if (wasNearBottom) {
            scrollToBottom(messagesContainer);
        }

        if (conversationId !== context.activeConversationId) {
            conversation.tabElement.classList.add('tab-unread');
        }

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
                tab.classList.remove('tab-unread');
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
    ): DocumentFragment {
        const fragment = document.createDocumentFragment();

        if (!toolResult) {
            return fragment;
        }

        if (typeof toolResult !== 'object' || !toolResult.kind) {
            const pre = document.createElement('pre');
            pre.textContent = JSON.stringify(toolResult, null, 2);
            fragment.appendChild(pre);
            return fragment;
        }

        switch (toolResult.kind) {
            case 'ModifyFile': {
                const fileResultDiv = document.createElement('div');
                fileResultDiv.className = 'tool-file-result';

                const successMessage = document.createElement('span');
                successMessage.className = 'tool-success-message';
                const filePathDisplay = filePath || 'file';
                successMessage.textContent = `✓ Modified: ${filePathDisplay}`;
                fileResultDiv.appendChild(successMessage);

                if (diffId) {
                    const diffButton = document.createElement('button');
                    diffButton.className = 'view-diff-button';
                    diffButton.setAttribute('data-diff-id', diffId);
                    diffButton.textContent = '📝 View Diff';
                    fileResultDiv.appendChild(diffButton);
                }

                fragment.appendChild(fileResultDiv);

                const detailDiv = document.createElement('div');
                detailDiv.className = 'tool-detail';
                detailDiv.textContent = `+${toolResult.lines_added} -${toolResult.lines_removed} lines`;
                fragment.appendChild(detailDiv);
                break;
            }

            case 'RunCommand': {
                const exitStatus = toolResult.exit_code === 0 ? '✓' : '⚠';
                const command = toolContext?.command?.trim();

                if (command) {
                    const commandDiv = document.createElement('div');
                    commandDiv.className = 'tool-command-highlight';
                    const code = document.createElement('code');
                    code.textContent = command;
                    commandDiv.appendChild(code);
                    fragment.appendChild(commandDiv);
                }

                const successDiv = document.createElement('div');
                successDiv.className = 'tool-success-message';
                successDiv.textContent = `${exitStatus} Exit code: ${toolResult.exit_code}`;
                fragment.appendChild(successDiv);

                if (toolResult.stdout) {
                    const details = document.createElement('details');
                    const summary = document.createElement('summary');
                    summary.textContent = 'Output';
                    details.appendChild(summary);
                    const pre = document.createElement('pre');
                    pre.textContent = toolResult.stdout;
                    details.appendChild(pre);
                    fragment.appendChild(details);
                }

                if (toolResult.stderr) {
                    const details = document.createElement('details');
                    const summary = document.createElement('summary');
                    summary.textContent = 'Errors';
                    details.appendChild(summary);
                    const pre = document.createElement('pre');
                    pre.textContent = toolResult.stderr;
                    details.appendChild(pre);
                    fragment.appendChild(details);
                }
                break;
            }

            case 'ReadFiles': {
                const fileCount = toolResult.files?.length || 0;
                const successDiv = document.createElement('div');
                successDiv.className = 'tool-success-message';
                successDiv.textContent = `✓ Read ${fileCount} file${fileCount === 1 ? '' : 's'}`;
                fragment.appendChild(successDiv);

                if (toolResult.files && toolResult.files.length > 0) {
                    for (const file of toolResult.files) {
                        const size = formatBytes(file.bytes);
                        const detailDiv = document.createElement('div');
                        detailDiv.className = 'tool-detail';
                        const code = document.createElement('code');
                        code.textContent = file.path;
                        detailDiv.appendChild(code);
                        detailDiv.appendChild(document.createTextNode(` — ${size}`));
                        fragment.appendChild(detailDiv);
                    }
                }
                break;
            }

            case 'Error': {
                const errorDiv = document.createElement('div');
                errorDiv.className = 'tool-error-message';
                errorDiv.textContent = toolResult.short_message;
                fragment.appendChild(errorDiv);
                break;
            }

            case 'SearchTypes': {
                const types = toolResult.types || [];
                const successDiv = document.createElement('div');
                successDiv.className = 'tool-success-message';
                successDiv.textContent = `✓ Found ${types.length} type${types.length === 1 ? '' : 's'}`;
                fragment.appendChild(successDiv);

                for (const typePath of types) {
                    const detailDiv = document.createElement('div');
                    detailDiv.className = 'tool-detail';
                    const code = document.createElement('code');
                    code.textContent = typePath;
                    detailDiv.appendChild(code);
                    fragment.appendChild(detailDiv);
                }
                break;
            }

            case 'GetTypeDocs': {
                const docs = toolResult.documentation || '';
                const successDiv = document.createElement('div');
                successDiv.className = 'tool-success-message';
                successDiv.textContent = `✓ Retrieved documentation`;
                fragment.appendChild(successDiv);

                if (docs) {
                    const details = document.createElement('details');
                    const summary = document.createElement('summary');
                    summary.textContent = 'Documentation';
                    details.appendChild(summary);
                    const pre = document.createElement('pre');
                    pre.textContent = docs;
                    details.appendChild(pre);
                    fragment.appendChild(details);
                }
                break;
            }

            case 'Other': {
                const result = toolResult.result;
                if (!result) {
                    return fragment;
                }

                if (result.content) {
                    const lines = result.content.split('\n').length;
                    const successDiv = document.createElement('div');
                    successDiv.className = 'tool-success-message';
                    successDiv.textContent = `✓ Read ${lines} lines`;
                    fragment.appendChild(successDiv);
                } else if (Array.isArray(result.entries)) {
                    const successDiv = document.createElement('div');
                    successDiv.className = 'tool-success-message';
                    successDiv.textContent = `✓ Found ${result.entries.length} entries`;
                    fragment.appendChild(successDiv);
                } else {
                    const pre = document.createElement('pre');
                    pre.textContent = JSON.stringify(result, null, 2);
                    fragment.appendChild(pre);
                }
                break;
            }

            default: {
                const pre = document.createElement('pre');
                pre.textContent = JSON.stringify(toolResult, null, 2);
                fragment.appendChild(pre);
                break;
            }
        }

        return fragment;
    }

    function handleStreamStart(message: StreamStartMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
        if (!messagesContainer) return;

        const messageDiv = document.createElement('div');
        messageDiv.className = 'message assistant streaming';
        messageDiv.setAttribute('data-stream-id', message.messageId);

        const modelInfo = message.model ? `<div class="model-info">${escapeHtml(message.model)}</div>` : '';
        const agentInfo = `<div class="agent-info">${escapeHtml(message.agent)}</div>`;
        messageDiv.innerHTML = `${modelInfo}${agentInfo}<div class="message-content"></div>`;

        const wasNearBottom = isNearBottom(messagesContainer);
        messagesContainer.appendChild(messageDiv);
        if (wasNearBottom) {
            scrollToBottom(messagesContainer);
        }

        conversation.streamingElement = messageDiv;
        conversation.streamingText = '';
        conversation.streamingAutoScroll = wasNearBottom;

        const sendButton = conversation.viewElement.querySelector<HTMLButtonElement>('.send-button');
        const cancelButton = conversation.viewElement.querySelector<HTMLButtonElement>('.cancel-button');
        if (sendButton && cancelButton) {
            sendButton.style.display = 'none';
            cancelButton.style.display = 'block';
            conversation.isProcessing = true;
        }
    }

    function handleStreamDelta(message: StreamDeltaMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const streamEl = conversation.streamingElement;
        if (!streamEl) return;

        conversation.streamingText = (conversation.streamingText || '') + message.text;

        const contentDiv = streamEl.querySelector<HTMLDivElement>('.message-content');
        if (!contentDiv) return;

        contentDiv.innerHTML = renderContent(conversation.streamingText);

        if (conversation.streamingAutoScroll) {
            const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
            if (messagesContainer) {
                scrollToBottom(messagesContainer);
            }
        }
    }

    function handleStreamReasoningDelta(message: StreamReasoningDeltaMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const streamEl = conversation.streamingElement;
        if (!streamEl) return;

        conversation.streamingReasoningText = (conversation.streamingReasoningText || '') + message.text;

        let reasoningDiv = streamEl.querySelector<HTMLDivElement>('.embedded-reasoning');
        if (!reasoningDiv) {
            reasoningDiv = document.createElement('div');
            reasoningDiv.className = 'embedded-reasoning';
            reasoningDiv.innerHTML = `
                <div class="reasoning-header">
                    💭 Reasoning
                </div>
                <div class="reasoning-content expanded"></div>
            `;
            const contentDiv = streamEl.querySelector<HTMLDivElement>('.message-content');
            if (contentDiv) {
                streamEl.insertBefore(reasoningDiv, contentDiv);
            }
        }

        const reasoningContent = reasoningDiv.querySelector<HTMLDivElement>('.reasoning-content');
        if (reasoningContent) {
            reasoningContent.innerHTML = renderContent(conversation.streamingReasoningText);
        }

        if (conversation.streamingAutoScroll) {
            const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
            if (messagesContainer) {
                scrollToBottom(messagesContainer);
            }
        }
    }

    function handleStreamEnd(message: StreamEndMessage): void {
        const conversation = context.store.get(message.conversationId);
        if (!conversation) return;

        const streamEl = conversation.streamingElement;
        conversation.streamingElement = undefined;
        conversation.streamingText = undefined;

        if (!streamEl) {
            displayMessage(message.conversationId, message.message);
            return;
        }

        streamEl.classList.remove('streaming');

        const chatMessage = message.message;

        let content: string;
        let reasoning: string | undefined;
        let toolCalls: any[];
        let tokenUsage: any;

        if (chatMessage.sender) {
            content = chatMessage.content;
            reasoning = chatMessage.reasoning?.text;
            toolCalls = chatMessage.tool_calls || [];
            tokenUsage = chatMessage.token_usage;
        } else {
            content = chatMessage.content;
            reasoning = chatMessage.reasoning;
            toolCalls = chatMessage.toolCalls || [];
            tokenUsage = chatMessage.tokenUsage;
        }

        const contentDiv = streamEl.querySelector<HTMLDivElement>('.message-content');

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

            const tokenDiv = document.createElement('div');
            tokenDiv.className = 'token-info';
            tokenDiv.textContent = `📊 Tokens: ${inputPart}/${outputPart}`;

            const insertBeforeEl = streamEl.querySelector<HTMLDivElement>('.embedded-reasoning') || contentDiv;
            if (insertBeforeEl) {
                streamEl.insertBefore(tokenDiv, insertBeforeEl);
            }
        }

        const alreadyStreamedReasoning = !!conversation.streamingReasoningText;
        conversation.streamingReasoningText = undefined;

        if (reasoning && !alreadyStreamedReasoning) {
            const reasoningId = `reasoning-${Date.now()}`;
            const isLong = reasoning.length > 120;
            const truncated = isLong ? `${reasoning.substring(0, 120)}...` : reasoning;

            const reasoningDiv = document.createElement('div');
            reasoningDiv.className = 'embedded-reasoning';
            reasoningDiv.innerHTML = `
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
            `;

            reasoningToggleState.set(reasoningId, isLong);

            if (contentDiv) {
                streamEl.insertBefore(reasoningDiv, contentDiv);
            }

            const header = reasoningDiv.querySelector('.reasoning-header-clickable');
            header?.addEventListener('click', () => toggleReasoning(reasoningId));
        }

        if (reasoning && alreadyStreamedReasoning) {
            const existingReasoningDiv = streamEl.querySelector<HTMLDivElement>('.embedded-reasoning');
            if (existingReasoningDiv) {
                const reasoningId = `reasoning-${Date.now()}`;
                const isLong = reasoning.length > 120;
                const truncated = isLong ? `${reasoning.substring(0, 120)}...` : reasoning;

                existingReasoningDiv.innerHTML = `
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
                `;

                reasoningToggleState.set(reasoningId, isLong);
                const header = existingReasoningDiv.querySelector('.reasoning-header-clickable');
                header?.addEventListener('click', () => toggleReasoning(reasoningId));
            }
        }

        if (contentDiv) {
            contentDiv.innerHTML = renderContent(content);
        }

        const toolCallMetadata: Array<{ elementId: string; toolCallId: string; command?: string }> = [];
        let taskListNote = '';

        if (toolCalls && toolCalls.length > 0) {
            const taskListTools = toolCalls.filter((tc: any) => tc.name === 'manage_task_list');
            const otherTools = toolCalls.filter((tc: any) => tc.name !== 'manage_task_list');

            if (taskListTools.length > 0) {
                taskListNote = '<div class="task-list-note">Task list updated</div>';
            }

            if (otherTools.length > 0) {
                const toolCallsDiv = document.createElement('div');
                toolCallsDiv.className = 'embedded-tool-calls';

                for (const toolCall of otherTools) {
                    const toolId = `tool-${message.conversationId}-${Date.now()}-${toolCall.name}`;
                    const toolCallId = toolCall.id ?? toolCall.tool_call_id ?? toolId;
                    const runCommand = toolCall?.arguments && typeof toolCall.arguments === 'object'
                        ? (toolCall.arguments as Record<string, unknown>).command : undefined;
                    const commandString = typeof runCommand === 'string' ? runCommand : undefined;
                    toolCallMetadata.push({ elementId: toolId, toolCallId, command: commandString });

                    const initialRequestHtml = toolCall.arguments
                        ? `<strong>Request:</strong><pre>${escapeHtml(JSON.stringify(toolCall.arguments, null, 2))}</pre>`
                        : '';
                    const previewHtml = generateToolPreview(toolCall);

                    const toolItemDiv = document.createElement('div');
                    toolItemDiv.className = 'tool-call-item';
                    toolItemDiv.setAttribute('data-tool-name', toolCall.name);
                    toolItemDiv.setAttribute('data-tool-call-id', toolCallId);
                    toolItemDiv.setAttribute('data-conversation-id', message.conversationId);
                    toolItemDiv.id = toolId;
                    toolItemDiv.innerHTML = `
                        <div class="tool-header">
                            <span class="tool-status-icon">⏳</span>
                            <span class="tool-name">${toolCall.name}</span>
                            <span class="tool-status-text">Pending</span>
                            <span class="tool-debug-toggle">▶</span>
                        </div>
                        ${previewHtml}
                        <div class="tool-result" style="display: none;"></div>
                        <div class="tool-debug-section">
                            <div class="tool-debug-content">
                                <div class="tool-debug-request">${initialRequestHtml}</div>
                                <div class="tool-debug-response"></div>
                            </div>
                        </div>
                    `;
                    toolCallsDiv.appendChild(toolItemDiv);
                }

                streamEl.appendChild(toolCallsDiv);
            }
        }

        const footerDiv = document.createElement('div');
        footerDiv.className = 'message-footer';
        footerDiv.innerHTML = taskListNote;
        streamEl.appendChild(footerDiv);

        if (toolCallMetadata.length > 0) {
            for (const meta of toolCallMetadata) {
                const toolElement = streamEl.querySelector<HTMLElement>(`#${meta.elementId}`);
                if (toolElement) {
                    if (meta.command) {
                        toolElement.setAttribute('data-run-command', meta.command);
                    }
                    hydrateToolItem(conversation, toolElement, meta.toolCallId);
                }
            }
        }

        addCodeActions(streamEl, context.vscode);
        addMessageCopyButton(streamEl, content, context.vscode);

        if (conversation.streamingAutoScroll) {
            const messagesContainer = conversation.viewElement.querySelector<HTMLDivElement>('.messages');
            if (messagesContainer) {
                scrollToBottom(messagesContainer);
            }
        }
        conversation.streamingAutoScroll = undefined;

        conversation.messages.push(chatMessage);
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
        handleProfileConfig,
        handleProfileSwitched,
        handleSettingsUpdate,
        handleTaskUpdate,
        handleSessionsListUpdate,
        handleStreamStart,
        handleStreamDelta,
        handleStreamReasoningDelta,
        handleStreamEnd,
        handleAddImageData,
        registerGlobalListeners
    };

    function handleSessionsListUpdate(sessions: any[]): void {
        renderSessionsList(sessions);
    }

    function renderSessionsList(sessions: any[]): void {
        renderWelcomeSessionsList(sessions);
        renderHistoryDropdownSessions(sessions);
    }

    function generateSessionItemHtml(session: any): string {
        const date = new Date(session.last_modified);
        const formattedDate = date.toLocaleDateString() + ' ' + date.toLocaleTimeString();
        return `
            <div class="session-item" data-session-id="${escapeHtml(session.id)}">
                <div class="session-title">${escapeHtml(session.title)}</div>
                <div class="session-date">${formattedDate}</div>
            </div>
        `;
    }

    function attachSessionClickListeners(container: Element, onAfterClick?: () => void): void {
        container.querySelectorAll('.session-item').forEach(item => {
            item.addEventListener('click', () => {
                const sessionId = (item as HTMLElement).getAttribute('data-session-id');
                if (!sessionId) return;

                context.vscode.postMessage({
                    type: 'resumeSession',
                    sessionId
                });

                if (onAfterClick) onAfterClick();
            });
        });
    }

    function renderWelcomeSessionsList(sessions: any[]): void {
        const sessionsList = document.getElementById('sessions-list');
        if (!sessionsList) return;

        if (!sessions || sessions.length === 0) {
            sessionsList.style.display = 'none';
            return;
        }

        sessionsList.style.display = 'block';
        const sessionsHtml = sessions.map(generateSessionItemHtml).join('');

        sessionsList.innerHTML = `
            <div class="sessions-header">Previous Sessions</div>
            <div class="sessions-items">
                ${sessionsHtml}
            </div>
        `;

        attachSessionClickListeners(sessionsList);
    }

    function renderHistoryDropdownSessions(sessions: any[]): void {
        const dropdownSessionsList = document.getElementById('dropdown-sessions-list');
        if (!dropdownSessionsList) return;

        if (!sessions || sessions.length === 0) {
            dropdownSessionsList.innerHTML = '<div class="no-sessions">No previous sessions</div>';
            return;
        }

        dropdownSessionsList.innerHTML = sessions.map(generateSessionItemHtml).join('');

        attachSessionClickListeners(dropdownSessionsList, () => {
            const dropdown = document.getElementById('new-tab-dropdown');
            if (dropdown) dropdown.style.display = 'none';
        });
    }
}
