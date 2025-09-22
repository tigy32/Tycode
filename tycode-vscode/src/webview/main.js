(function () {
    const vscode = acquireVsCodeApi();

    // State management
    let conversations = new Map(); // id -> { title, messages, element }
    let activeConversationId = null;
    let currentRetryElements = new Map(); // conversationId -> retry element

    // DOM elements
    const welcomeScreen = document.getElementById('welcome-screen');
    const tabBar = document.getElementById('tab-bar');
    const tabsContainer = document.getElementById('tabs');
    const conversationsContainer = document.getElementById('conversations-container');
    const newTabButton = document.getElementById('new-tab-button');
    const welcomeNewChatButton = document.getElementById('welcome-new-chat');
    const welcomeSettingsButton = document.getElementById('welcome-settings');

    // Event listeners
    newTabButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'newChat' });
    });

    welcomeNewChatButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'newChat' });
    });

    welcomeSettingsButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'openSettings' });
    });

    // Handle messages from extension
    window.addEventListener('message', event => {
        const message = event.data;

        switch (message.type) {
            case 'initialState':
                handleInitialState(message);
                break;
            case 'conversationCreated':
                handleConversationCreated(message);
                break;
            case 'conversationMessage':
                handleConversationMessage(message);
                break;
            case 'activeConversationChanged':
                handleActiveConversationChanged(message);
                break;
            case 'conversationClosed':
                handleConversationClosed(message);
                break;
            case 'conversationTitleChanged':
                handleConversationTitleChanged(message);
                break;
            case 'conversationCleared':
                handleConversationCleared(message);
                break;
            case 'showTyping':
                handleShowTyping(message);
                break;
            case 'conversationDisconnected':
                handleConversationDisconnected(message);
                break;
            case 'toolResult':
                handleToolResult(message);
                break;
            case 'providerConfig':
                handleProviderConfig(message);
                break;
            case 'providerSwitched':
                handleProviderSwitched(message);
                break;
            case 'retryAttempt':
                handleRetryAttempt(message);
                break;
            case 'toolRequest':
                handleToolRequest(message);
                break;
        }
    });

    function handleProviderConfig(message) {
        const { conversationId, providers, selectedProvider } = message;
        const conversation = conversations.get(conversationId);
        if (!conversation) return;

        const providerSelect = conversation.viewElement.querySelector('.provider-select');
        if (!providerSelect) return;

        // Clear existing options
        providerSelect.innerHTML = '';

        // Add provider options
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
            // Default option if no providers
            const option = document.createElement('option');
            option.value = 'default';
            option.textContent = 'default';
            option.selected = true;
            providerSelect.appendChild(option);
        }

        // Store selected provider
        conversation.selectedProvider = selectedProvider;
    }

    function handleProviderSwitched(message) {
        const { conversationId, newProvider } = message;
        const conversation = conversations.get(conversationId);
        if (!conversation) return;

        const providerSelect = conversation.viewElement.querySelector('.provider-select');
        if (providerSelect && newProvider) {
            providerSelect.value = newProvider;
        }

        conversation.selectedProvider = newProvider;
    }

    function handleInitialState(message) {
        // Clear existing state
        conversations.clear();
        tabsContainer.innerHTML = '';
        conversationsContainer.innerHTML = '';

        // Load conversations
        if (message.conversations && message.conversations.length > 0) {
            for (const conv of message.conversations) {
                createConversationUI(conv.id, conv.title);

                // Load existing messages
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

    function handleConversationCreated(message) {
        createConversationUI(message.id, message.title);
        setActiveConversation(message.id);
        showConversations();
    }

    function createConversationUI(id, title) {
        // Create tab
        const tab = document.createElement('div');
        tab.className = 'tab';
        tab.dataset.conversationId = id;
        tab.innerHTML = `
            <span class="tab-title">${escapeHtml(title)}</span>
            <input class="tab-title-input" type="text" value="${escapeHtml(title)}" style="display: none;">
            <button class="tab-close" title="Close">×</button>
        `;

        const tabTitle = tab.querySelector('.tab-title');
        const tabInput = tab.querySelector('.tab-title-input');
        let isEditing = false;

        // Double-click to edit title
        tabTitle.addEventListener('dblclick', (e) => {
            e.stopPropagation();
            startEditingTitle(id, tab, tabTitle, tabInput);
        });

        // Click to switch tabs (only if not editing)
        tab.addEventListener('click', (e) => {
            if (!e.target.classList.contains('tab-close') && !isEditing) {
                vscode.postMessage({ type: 'switchTab', conversationId: id });
            }
        });

        // Handle input events
        tabInput.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') {
                e.preventDefault();
                saveTabTitle(id, tab, tabTitle, tabInput);
            } else if (e.key === 'Escape') {
                e.preventDefault();
                cancelEditingTitle(id, tab, tabTitle, tabInput);
            }
        });

        tabInput.addEventListener('blur', () => {
            if (tabInput.style.display !== 'none') {
                saveTabTitle(id, tab, tabTitle, tabInput);
            }
        });

        // Right-click context menu
        tab.addEventListener('contextmenu', (e) => {
            e.preventDefault();
            showTabContextMenu(e, id, tab, tabTitle, tabInput);
        });

        tab.querySelector('.tab-close').addEventListener('click', (e) => {
            e.stopPropagation();
            vscode.postMessage({ type: 'closeTab', conversationId: id });
        });

        tabsContainer.appendChild(tab);

        // Create conversation view
        const conversationView = document.createElement('div');
        conversationView.className = 'conversation-view';
        conversationView.dataset.conversationId = id;
        conversationView.style.display = 'none';
        // Clear any previous content and rebuild with provider selector
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

        // Set up event listeners for this conversation
        const messageInput = conversationView.querySelector('.message-input');
        const sendButton = conversationView.querySelector('.send-button');
        const cancelButton = conversationView.querySelector('.cancel-button');
        const clearButton = conversationView.querySelector('.clear-chat');
        const providerSelect = conversationView.querySelector('.provider-select');
        const refreshProvidersBtn = conversationView.querySelector('.refresh-providers');

        sendButton.addEventListener('click', () => sendMessage(id, messageInput));

        // Add cancel button handler with smart auto-send
        cancelButton.addEventListener('click', () => {
            // Get any pending text in the input
            const pendingMessage = messageInput.value.trim();

            // Send cancel command with conversationId
            vscode.postMessage({
                type: 'cancel',
                conversationId: id
            });

            // If there's pending text, send it after a short delay
            if (pendingMessage) {
                // Clear the input first
                messageInput.value = '';
                messageInput.style.height = 'auto';

                // Wait a brief moment for cancel to process, then send the new message
                setTimeout(() => {
                    // Send to extension
                    vscode.postMessage({
                        type: 'sendMessage',
                        conversationId: id,
                        message: pendingMessage
                    });
                }, 100);
            }
        });

        messageInput.addEventListener('keydown', (e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                const conversation = conversations.get(id);
                // If processing, Enter key triggers cancel with auto-send
                if (conversation && conversation.isProcessing) {
                    cancelButton.click();
                } else {
                    sendMessage(id, messageInput);
                }
            }
        });

        // Auto-resize textarea
        messageInput.addEventListener('input', () => {
            messageInput.style.height = 'auto';
            messageInput.style.height = messageInput.scrollHeight + 'px';
        });

        clearButton.addEventListener('click', () => {
            const messagesContainer = conversationView.querySelector('.messages');
            messagesContainer.innerHTML = '';
            vscode.postMessage({ type: 'clearChat', conversationId: id });
        });

        // Handle provider selection change for this conversation
        if (providerSelect) {
            providerSelect.addEventListener('change', (e) => {
                const selectedProvider = e.target.value;
                vscode.postMessage({
                    type: 'switchProvider',
                    conversationId: id,
                    provider: selectedProvider
                });
            });

            // Remove any focus event that might reload providers
            // We only want explicit refresh button clicks to reload
        }

        // Handle refresh providers button
        if (refreshProvidersBtn) {
            refreshProvidersBtn.addEventListener('click', () => {
                vscode.postMessage({
                    type: 'refreshProviders',  // Different message type for refresh
                    conversationId: id
                });
            });
        }

        conversationsContainer.appendChild(conversationView);

        // Store in map
        conversations.set(id, {
            title,
            messages: [],
            tabElement: tab,
            viewElement: conversationView,
            selectedProvider: null
        });

        // Request provider list for this conversation (initial load - no reload)
        vscode.postMessage({
            type: 'getProviders',
            conversationId: id
        });
    }

    function sendMessage(conversationId, inputElement) {
        const message = inputElement.value.trim();
        if (!message) return;

        // Clear input
        inputElement.value = '';
        inputElement.style.height = 'auto';

        // Send to extension
        vscode.postMessage({
            type: 'sendMessage',
            conversationId,
            message
        });
    }

    function handleConversationMessage(message) {
        displayMessage(message.conversationId, message.message);
    }

    function displayMessage(conversationId, chatMessage) {
        const conversation = conversations.get(conversationId);
        if (!conversation) return;

        const messagesContainer = conversation.viewElement.querySelector('.messages');

        // Handle ChatMessage structure directly
        let role, content, reasoning, toolCalls, model, isComplete, tokenUsage;

        if (chatMessage.sender) {
            // This is a ChatMessage from Rust
            role = getRoleFromSender(chatMessage.sender);
            content = chatMessage.content;
            reasoning = chatMessage.reasoning?.text;
            toolCalls = chatMessage.tool_calls || [];
            model = chatMessage.model_info?.model;
            isComplete = true; // MessageAdded events are complete
            tokenUsage = chatMessage.token_usage;
        } else {
            // Legacy message format or error message
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
            // Include model info if available
            const modelInfo = model ? `<div class="model-info">Model: ${model}</div>` : '';
            const completionInfo = isComplete !== undefined ?
                `<div class="completion-info">${isComplete ? '✅ Complete' : '⏳ Pending AI response'}</div>` : '';

            // Build token usage info if available
            let tokenInfo = '';
            if (tokenUsage) {
                tokenInfo = `<div class="token-info">📊 Tokens: ${tokenUsage.input_tokens} in, ${tokenUsage.output_tokens} out (${tokenUsage.total_tokens} total)</div>`;
            }

            // Build the reasoning section if present
            let reasoningSection = '';
            if (reasoning) {
                const reasoningId = 'reasoning-' + Date.now();
                const isLong = reasoning.length > 120;
                const truncated = isLong ? reasoning.substring(0, 120) + '...' : reasoning;

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

                // Store expansion state
                window[`toggleReasoning_${reasoningId}`] = isLong;
            }

            // Build tool calls section if present
            let toolCallsSection = '';
            if (toolCalls && toolCalls.length > 0) {
                const toolCallsHtml = toolCalls.map(toolCall => {
                    const toolId = `tool-${conversationId}-${Date.now()}-${toolCall.name}`;
                    return `
                        <div class="tool-call-item" data-tool-name="${toolCall.name}" data-conversation-id="${conversationId}" id="${toolId}">
                            <div class="tool-header">
                                <span class="tool-status-icon">⏳</span>
                                <span class="tool-name">${toolCall.name}</span>
                                <span class="tool-status-text">Executing...</span>
                            </div>
                            ${formatToolDetails(toolCall)}
                            <div class="tool-result" style="display: none;"></div>
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
                ${toolCallsSection}
                ${completionInfo}
            `;

            // Add click event listener for reasoning after adding to DOM
            if (reasoning) {
                setTimeout(() => {
                    const header = messageDiv.querySelector('.reasoning-header-clickable');
                    if (header) {
                        header.addEventListener('click', function () {
                            const id = this.getAttribute('data-reasoning-id');
                            toggleReasoning(id);
                        });
                    }
                }, 0);
            }
        } else {
            messageDiv.innerHTML = renderContent(content);
        }

        // Add code action buttons
        addCodeActions(messageDiv);

        messagesContainer.appendChild(messageDiv);
        messagesContainer.scrollTop = messagesContainer.scrollHeight;

        // Store message
        conversation.messages.push(chatMessage);
    }

    function toggleReasoning(reasoningId) {
        const content = document.getElementById(reasoningId);
        const toggle = document.getElementById(reasoningId + '-toggle');
        const truncated = content.querySelector('.reasoning-truncated');
        const full = content.querySelector('.reasoning-full');

        // Only toggle if there's a full version
        if (window[`toggleReasoning_${reasoningId}`]) {
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
    }

    function handleActiveConversationChanged(message) {
        setActiveConversation(message.id);
    }

    function setActiveConversation(id) {
        activeConversationId = id;

        // Update tab styling
        document.querySelectorAll('.tab').forEach(tab => {
            if (tab.dataset.conversationId === id) {
                tab.classList.add('active');
            } else {
                tab.classList.remove('active');
            }
        });

        // Show/hide conversation views
        document.querySelectorAll('.conversation-view').forEach(view => {
            if (view.dataset.conversationId === id) {
                view.style.display = 'flex';
                // Focus input
                const input = view.querySelector('.message-input');
                if (input) input.focus();
            } else {
                view.style.display = 'none';
            }
        });
    }

    function handleConversationClosed(message) {
        const conversation = conversations.get(message.id);
        if (conversation) {
            conversation.tabElement.remove();
            conversation.viewElement.remove();
            conversations.delete(message.id);
        }

        if (conversations.size === 0) {
            showWelcomeScreen();
        }
    }

    function handleConversationTitleChanged(message) {
        const conversation = conversations.get(message.id);
        if (conversation) {
            conversation.title = message.title;

            // Update tab elements
            const titleElement = conversation.tabElement.querySelector('.tab-title');
            const inputElement = conversation.tabElement.querySelector('.tab-title-input');
            if (titleElement) titleElement.textContent = message.title;
            if (inputElement) inputElement.value = message.title;

            // Update header
            conversation.viewElement.querySelector('.chat-header h3').textContent = message.title;
        }
    }

    function handleConversationCleared(message) {
        const conversation = conversations.get(message.conversationId);
        if (conversation) {
            conversation.messages = [];
        }
    }

    function handleShowTyping(message) {
        const conversation = conversations.get(message.conversationId);
        if (conversation) {
            const typingIndicator = conversation.viewElement.querySelector('.typing-indicator');
            typingIndicator.style.display = message.show ? 'flex' : 'none';
            if (message.show) {
                const messagesContainer = conversation.viewElement.querySelector('.messages');
                messagesContainer.scrollTop = messagesContainer.scrollHeight;
            }

            // Swap send/cancel buttons
            const sendButton = conversation.viewElement.querySelector('.send-button');
            const cancelButton = conversation.viewElement.querySelector('.cancel-button');

            if (sendButton && cancelButton) {
                if (message.show) {
                    // Show cancel, hide send
                    sendButton.style.display = 'none';
                    cancelButton.style.display = 'block';
                    conversation.isProcessing = true;
                } else {
                    // Show send, hide cancel
                    sendButton.style.display = 'block';
                    cancelButton.style.display = 'none';
                    conversation.isProcessing = false;

                    // Clear retry status when processing completes
                    const retryElement = currentRetryElements.get(message.conversationId);
                    if (retryElement) {
                        retryElement.remove();
                        currentRetryElements.delete(message.conversationId);
                    }
                }
            }
        }
    }

    function handleRetryAttempt(message) {
        const { conversationId, attempt, maxRetries, error, backoffMs } = message;
        const conversation = conversations.get(conversationId);
        if (!conversation) return;

        const messagesContainer = conversation.viewElement.querySelector('.messages');
        if (!messagesContainer) return;

        // Get or create retry status element
        let retryElement = currentRetryElements.get(conversationId);
        if (!retryElement) {
            retryElement = document.createElement('div');
            retryElement.className = 'message system retry-status';
            messagesContainer.appendChild(retryElement);
            currentRetryElements.set(conversationId, retryElement);
        }

        // Format error message
        const errorMsg = error ? error.substring(0, 100) : 'Request failed';
        const nextAttemptIn = Math.ceil(backoffMs / 1000);

        // Update retry status display
        retryElement.innerHTML = `
            <div class="retry-info">
                <span class="retry-icon">🔄</span>
                <span class="retry-text">
                    [Request failed - retrying (attempt ${attempt}/${maxRetries})]
                    <br>
                    <span class="retry-error">${escapeHtml(errorMsg)}</span>
                    <br>
                    <span class="retry-countdown">Next attempt in ${nextAttemptIn}s...</span>
                </span>
            </div>
        `;

        // Scroll to show the retry status
        messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }

    function handleConversationDisconnected(message) {
        const conversation = conversations.get(message.id);
        if (conversation) {
            displayMessage(message.id, {
                role: 'error',
                content: 'Connection to backend lost. Please close this tab and start a new chat.'
            });
        }
    }

    function handleToolRequest(message) {
        console.log('Tool request received:', message);

        const { conversationId, toolName, arguments: toolArgs, toolType, diffId } = message;
        const conversation = conversations.get(conversationId);
        if (!conversation) {
            console.warn('No conversation found for:', conversationId);
            return;
        }

        const messagesContainer = conversation.viewElement.querySelector('.messages');
        if (!messagesContainer) {
            console.warn('No messages container found for conversation:', conversationId);
            return;
        }

        // Find the most recent tool-call-item for this tool in the conversation
        const toolItems = conversation.viewElement.querySelectorAll(`.tool-call-item[data-tool-name="${toolName}"]`);
        if (toolItems.length === 0) {
            console.warn('No tool item found for:', toolName, 'in conversation:', conversationId);
            return;
        }

        const toolItem = toolItems[toolItems.length - 1]; // Get the most recent

        // Update status icon and text to 'Requested'
        const statusIcon = toolItem.querySelector('.tool-status-icon');
        const statusText = toolItem.querySelector('.tool-status-text');
        statusIcon.textContent = '🔧';
        statusText.textContent = 'Requested';

        // Add arguments to tool-details if present (but not for ModifyFile with diffId, where we show view diff button instead)
        if (toolArgs && Object.keys(toolArgs).length > 0 && !(toolType && 'ModifyFile' in toolType && diffId)) {
            let detailsDiv = toolItem.querySelector('.tool-details');
            if (!detailsDiv) {
                detailsDiv = document.createElement('div');
                detailsDiv.className = 'tool-details';
                toolItem.appendChild(detailsDiv);
            }
            detailsDiv.innerHTML = `<pre>${escapeHtml(JSON.stringify(toolArgs, null, 2))}</pre>`;
        }

        // Add View Diff button for ModifyFile requests
        if (toolType && 'ModifyFile' in toolType && diffId) {
            let actionsDiv = toolItem.querySelector('.tool-request-actions');
            if (!actionsDiv) {
                actionsDiv = document.createElement('div');
                actionsDiv.className = 'tool-request-actions';
                toolItem.appendChild(actionsDiv);
            }
            actionsDiv.innerHTML = `<button class="view-diff-button" data-diff-id="${diffId}">📝 View Diff</button>`;
        }

        // Scroll to show the update
        messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }

    function handleToolResult(message) {
        console.log('Tool result received:', message);

        const { conversationId, toolName, success, result, error, diffId } = message;

        // Find the most recent tool call item with this name in the specified conversation
        const conversationView = document.querySelector(`.conversation-view[data-conversation-id="${conversationId}"]`);
        if (!conversationView) {
            console.warn('No conversation view found for:', conversationId);
            return;
        }

        const toolItems = conversationView.querySelectorAll(`.tool-call-item[data-tool-name="${toolName}"]`);
        if (toolItems.length === 0) {
            console.warn('No tool item found for:', toolName, 'in conversation:', conversationId);
            return;
        }

        // Get the last one (most recent)
        const toolItem = toolItems[toolItems.length - 1];

        // Update status icon and text
        const statusIcon = toolItem.querySelector('.tool-status-icon');
        const statusText = toolItem.querySelector('.tool-status-text');
        const resultDiv = toolItem.querySelector('.tool-result');

        if (success) {
            statusIcon.textContent = '✅';
            statusText.textContent = 'Success';
            toolItem.classList.add('tool-success');
        } else {
            statusIcon.textContent = '❌';
            statusText.textContent = 'Failed';
            toolItem.classList.add('tool-error');
        }

        // Display result if available
        if (result || error) {
            resultDiv.style.display = 'block';

            // Format the result based on tool type
            let resultContent = '';
            if (error) {
                resultContent = `<div class="tool-error-message">${escapeHtml(error)}</div>`;
            } else if (result) {
                // Special formatting for different tool types
                if (toolName === 'write_file' || toolName === 'modify_file') {
                    // File modification tools  
                    if (result.path) {
                        resultContent = `<div class="tool-success-message">✓ Modified: ${escapeHtml(result.path)}</div>`;
                        if (result.changes_applied !== undefined) {
                            resultContent += `<div class="tool-detail">Changes applied: ${result.changes_applied}</div>`;
                        }
                        // Add View Diff button if diffId is available
                        if (diffId) {
                            console.log('[Main] Adding diff button with ID:', diffId);
                            resultContent += `<button class="view-diff-button" data-diff-id="${diffId}">📝 View Diff</button>`;
                        } else {
                            console.log('[Main] No diffId available for file modification');
                        }
                    } else {
                        resultContent = `<div class="tool-success-message">✓ File operation completed</div>`;
                    }
                } else if (toolName === 'delete_file') {
                    if (result.path) {
                        resultContent = `<div class="tool-success-message">✓ Deleted: ${escapeHtml(result.path)}</div>`;
                    }
                } else if (toolName === 'read_file') {
                    if (result.content) {
                        const lines = result.content.split('\n').length;
                        resultContent = `<div class="tool-success-message">✓ Read ${lines} lines</div>`;
                    }
                } else if (toolName === 'list_files') {
                    if (result.files && Array.isArray(result.files)) {
                        resultContent = `<div class="tool-success-message">✓ Found ${result.files.length} files</div>`;
                    }
                } else if (toolName === 'run_build_test') {
                    if (result.code !== undefined) {
                        const exitStatus = result.exit_code === 0 ? '✓' : '⚠';
                        resultContent = `<div class="tool-success-message">${exitStatus} Exit code: ${result.exit_code}</div>`;
                        if (result.out) {
                            resultContent += `<details><summary>Output</summary><pre>${escapeHtml(result.stdout)}</pre></details>`;
                        }
                        if (result.err) {
                            resultContent += `<details><summary>Errors</summary><pre>${escapeHtml(result.stderr)}</pre></details>`;
                        }
                    }
                } else {
                    // Generic result display
                    resultContent = `<pre>${escapeHtml(JSON.stringify(result, null, 2))}</pre>`;
                }
            }

            resultDiv.innerHTML = resultContent;
        }

        // Scroll to show the update
        const messagesContainer = conversationView.querySelector('.messages');
        if (messagesContainer) {
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }
    }

    function showWelcomeScreen() {
        welcomeScreen.style.display = 'flex';
        tabBar.style.display = 'none';
        conversationsContainer.style.display = 'none';
    }

    function showConversations() {
        welcomeScreen.style.display = 'none';
        tabBar.style.display = 'flex';
        conversationsContainer.style.display = 'flex';
    }

    function renderContent(content) {
        // Escape HTML first
        let rendered = escapeHtml(content);

        // Render code blocks with syntax highlighting hint
        rendered = rendered.replace(/```(\w+)?\n([\s\S]*?)```/g, (match, lang, code) => {
            return `<div class="code-block-container">
                <pre><code class="language-${lang || 'plaintext'}">${escapeHtml(code.trim())}</code></pre>
            </div>`;
        });

        // Render inline code
        rendered = rendered.replace(/`([^`]+)`/g, '<code>$1</code>');

        // Render markdown headers (h1-h6)
        rendered = rendered.replace(/^######\s+(.+)$/gm, '<h6>$1</h6>');
        rendered = rendered.replace(/^#####\s+(.+)$/gm, '<h5>$1</h5>');
        rendered = rendered.replace(/^####\s+(.+)$/gm, '<h4>$1</h4>');
        rendered = rendered.replace(/^###\s+(.+)$/gm, '<h3>$1</h3>');
        rendered = rendered.replace(/^##\s+(.+)$/gm, '<h2>$1</h2>');
        rendered = rendered.replace(/^#\s+(.+)$/gm, '<h1>$1</h1>');

        // Render links
        rendered = rendered.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank">$1</a>');

        // Render bold
        rendered = rendered.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');

        // Render italic
        rendered = rendered.replace(/\*([^*]+)\*/g, '<em>$1</em>');

        // Preserve line breaks
        rendered = rendered.replace(/\n/g, '<br>');

        // Clean up excessive spacing
        rendered = rendered.replace(/(<h[1-6]>.*?)<br>(.*?<\/h[1-6]>)/g, '$1 $2');
        rendered = rendered.replace(/(<\/h[1-6]>)<br>/g, '$1');
        rendered = rendered.replace(/(<br>){2,}/g, '<br>');
        rendered = rendered.replace(/<br>(<h[1-6]>)/g, '$1');

        return rendered;
    }

    function escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }

    function formatToolDetails(toolCall) {
        if (!toolCall.arguments) return '';

        // For all tools, show the raw JSON arguments as fallback
        return `<div class="tool-details"><pre>${escapeHtml(JSON.stringify(toolCall.arguments, null, 2))}</pre></div>`;
    }

    function addCodeActions(messageDiv) {
        const codeBlocks = messageDiv.querySelectorAll('.code-block-container');

        codeBlocks.forEach(block => {
            const actionsDiv = document.createElement('div');
            actionsDiv.className = 'code-actions';

            // Copy button
            const copyButton = document.createElement('button');
            copyButton.className = 'code-action-button';
            copyButton.textContent = 'Copy';
            copyButton.onclick = () => {
                const code = block.querySelector('code').textContent;
                vscode.postMessage({
                    type: 'copyCode',
                    code: code
                });
            };

            // Insert button
            const insertButton = document.createElement('button');
            insertButton.className = 'code-action-button';
            insertButton.textContent = 'Insert';
            insertButton.onclick = () => {
                const code = block.querySelector('code').textContent;
                vscode.postMessage({
                    type: 'insertCode',
                    code: code
                });
            };

            actionsDiv.appendChild(copyButton);
            actionsDiv.appendChild(insertButton);
            block.appendChild(actionsDiv);
        });
    }

    function startEditingTitle(conversationId, tab, titleElement, inputElement) {
        // Show input, hide title
        titleElement.style.display = 'none';
        inputElement.style.display = 'block';
        inputElement.value = titleElement.textContent;
        inputElement.select();
        inputElement.focus();

        // Mark tab as editing
        tab.classList.add('editing');
    }

    function saveTabTitle(conversationId, tab, titleElement, inputElement) {
        const newTitle = inputElement.value.trim();

        // Don't save empty titles
        if (!newTitle) {
            cancelEditingTitle(conversationId, tab, titleElement, inputElement);
            return;
        }

        // Only send message if title actually changed
        if (newTitle !== titleElement.textContent) {
            vscode.postMessage({
                type: 'renameTab',
                conversationId: conversationId,
                title: newTitle
            });

            // Update the displayed title immediately for responsiveness
            titleElement.textContent = newTitle;
            inputElement.value = newTitle;
        }

        // Hide input, show title
        inputElement.style.display = 'none';
        titleElement.style.display = 'block';
        tab.classList.remove('editing');
    }

    function cancelEditingTitle(conversationId, tab, titleElement, inputElement) {
        // Restore original value
        inputElement.value = titleElement.textContent;

        // Hide input, show title
        inputElement.style.display = 'none';
        titleElement.style.display = 'block';
        tab.classList.remove('editing');
    }

    function showTabContextMenu(event, conversationId, tab, titleElement, inputElement) {
        // Remove any existing context menu
        const existingMenu = document.querySelector('.tab-context-menu');
        if (existingMenu) {
            existingMenu.remove();
        }

        // Create context menu
        const menu = document.createElement('div');
        menu.className = 'tab-context-menu';
        menu.style.position = 'fixed';
        menu.style.left = event.clientX + 'px';
        menu.style.top = event.clientY + 'px';
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

        // Handle menu item clicks
        menu.addEventListener('click', (e) => {
            const item = e.target.closest('.context-menu-item');
            if (item) {
                const action = item.dataset.action;
                if (action === 'rename') {
                    startEditingTitle(conversationId, tab, titleElement, inputElement);
                } else if (action === 'close') {
                    vscode.postMessage({ type: 'closeTab', conversationId: conversationId });
                }
                menu.remove();
            }
        });

        // Close menu when clicking outside
        setTimeout(() => {
            document.addEventListener('click', function closeMenu(e) {
                if (!menu.contains(e.target)) {
                    menu.remove();
                    document.removeEventListener('click', closeMenu);
                }
            });
        }, 0);

        document.body.appendChild(menu);
    }

    function getRoleFromSender(sender) {
        if (sender === 'User') {
            return 'user';
        }
        if (sender === 'System') {
            return 'system';
        }
        if (sender === 'Error') {
            return 'error';
        }
        if (typeof sender === 'object' && sender !== null && 'Assistant' in sender) {
            return 'assistant';
        }
        // exhaustiveness check - if we get here, it's an unknown sender type
        console.error('Unknown sender type:', sender);
        return 'system'; // fallback
    }

    // Handle View Diff button clicks using event delegation
    document.addEventListener('click', (e) => {
        if (e.target && e.target.classList && e.target.classList.contains('view-diff-button')) {
            const diffId = e.target.getAttribute('data-diff-id');
            console.log('[Main] View diff clicked, diffId:', diffId);
            if (diffId) {
                const message = {
                    type: 'viewDiff',
                    diffId: diffId
                };
                console.log('[Main] Sending message:', message);
                console.log('[Main] vscode object exists?', typeof vscode !== 'undefined');
                try {
                    vscode.postMessage(message);
                    console.log('[Main] Message sent successfully');
                } catch (error) {
                    console.error('[Main] Error sending message:', error);
                }
            }
        }
    });
})();