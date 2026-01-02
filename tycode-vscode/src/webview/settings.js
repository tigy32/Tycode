const vscode = acquireVsCodeApi();
let settings = {
    active_provider: 'default',
    providers: {},
    security: { mode: 'auto' },
    model_quality: null,
    review_level: 'None',
    file_modification_api: 'Default',
    run_build_test_output_mode: 'ToolResponse',
    command_execution_mode: 'Direct',
    default_agent: '',
    auto_context_bytes: 80000,
    mcp_servers: {},
    agent_models: {},
    enable_type_analyzer: false,
    spawn_context_mode: 'Fork',
    xml_tool_mode: false,
    disable_custom_steering: false,
    communication_tone: 'concise_and_logical',
    autonomy_level: 'plan_approval_required',
    memory: { enabled: false, summarizer_cost: 'high', recorder_cost: 'high', context_message_count: 0 }
};
let activeTab = 'general';
let editingProvider = null;
let deletingProvider = null;
let editingMcp = null;
let deletingMcp = null;
let editingAgentModel = null;
let deletingAgentModel = null;
let deleteType = 'provider';
let availableProfiles = ['default'];
let currentProfile = 'default';

window.addEventListener('message', event => {
    const message = event.data;
    switch (message.type) {
        case 'loadSettings':
            settings = message.settings || settings;
            renderAll();
            break;
        case 'loadProfiles':
            availableProfiles = message.profiles || ['default'];
            renderProfiles();
            break;
    }
});

document.addEventListener('DOMContentLoaded', function() {
    document.querySelectorAll('.nav-item').forEach(function(navItem) {
        navItem.addEventListener('click', function() {
            switchTab(this.dataset.tab);
        });
    });
    
    document.getElementById('memoryEnabled').addEventListener('change', updateMemorySettings);
    document.getElementById('memorySummarizerCost').addEventListener('change', updateMemorySettings);
    document.getElementById('memoryRecorderCost').addEventListener('change', updateMemorySettings);
    document.getElementById('memoryContextMessageCount').addEventListener('input', updateMemorySettings);
    
    document.getElementById('communicationTone').addEventListener('change', updateGeneralSettings);
    document.getElementById('autonomyLevel').addEventListener('change', updateGeneralSettings);
    document.getElementById('securityMode').addEventListener('change', updateGeneralSettings);
    document.getElementById('modelQuality').addEventListener('change', updateGeneralSettings);
    document.getElementById('reviewLevel').addEventListener('change', updateGeneralSettings);
    document.getElementById('fileModificationApi').addEventListener('change', updateGeneralSettings);
    document.getElementById('runBuildTestOutputMode').addEventListener('change', updateGeneralSettings);
    document.getElementById('defaultAgent').addEventListener('input', updateGeneralSettings);
    document.getElementById('autoContextBytes').addEventListener('input', updateGeneralSettings);
    document.getElementById('enableTypeAnalyzer').addEventListener('change', updateGeneralSettings);
    document.getElementById('commandExecutionMode').addEventListener('change', updateGeneralSettings);
    document.getElementById('spawnContextMode').addEventListener('change', updateGeneralSettings);
    document.getElementById('xmlToolMode').addEventListener('change', updateGeneralSettings);
    document.getElementById('disableCustomSteering').addEventListener('change', updateGeneralSettings);
    
    document.getElementById('currentProfile').addEventListener('change', switchProfile);
    document.getElementById('saveProfileBtn').addEventListener('click', saveProfile);
    
    document.getElementById('addProviderBtn').addEventListener('click', showAddProviderModal);
    
    document.getElementById('providerType').addEventListener('change', function() {
        updateProviderFields(this.value);
    });
    
    document.getElementById('closeModalBtn').addEventListener('click', closeModal);
    document.getElementById('saveProviderBtn').addEventListener('click', saveProvider);
    document.getElementById('closeMcpModalBtn').addEventListener('click', closeMcpModal);
    document.getElementById('saveMcpBtn').addEventListener('click', saveMcp);
    document.getElementById('closeAgentModelModalBtn').addEventListener('click', closeAgentModelModal);
    document.getElementById('saveAgentModelBtn').addEventListener('click', saveAgentModel);
    document.getElementById('cancelDeleteBtn').addEventListener('click', cancelDelete);
    document.getElementById('confirmDeleteBtn').addEventListener('click', confirmDelete);
    
    document.getElementById('addMcpBtn').addEventListener('click', showAddMcpModal);
    
    document.getElementById('addAgentModelBtn').addEventListener('click', showAddAgentModelModal);
    
    document.getElementById('providerList').addEventListener('change', function(e) {
        if (e.target.type === 'radio' && e.target.name === 'activeProvider') {
            setActiveProvider(e.target.value);
        }
    });
    
    document.getElementById('providerList').addEventListener('click', function(e) {
        if (e.target.classList.contains('edit-btn')) {
            editProvider(e.target.dataset.provider);
        } else if (e.target.classList.contains('delete-btn')) {
            deleteProvider(e.target.dataset.provider);
        }
    });
    
    document.getElementById('mcpList').addEventListener('click', function(e) {
        if (e.target.classList.contains('edit-btn')) {
            editMcp(e.target.dataset.mcp);
        } else if (e.target.classList.contains('delete-btn')) {
            deleteMcp(e.target.dataset.mcp);
        }
    });
    
    document.getElementById('agentModelsList').addEventListener('click', function(e) {
        if (e.target.classList.contains('edit-btn')) {
            editAgentModel(e.target.dataset.agent);
        } else if (e.target.classList.contains('delete-btn')) {
            deleteAgentModel(e.target.dataset.agent);
        }
    });
});

function switchTab(tabId) {
    activeTab = tabId;
    
    document.querySelectorAll('.nav-item').forEach(function(item) {
        item.classList.remove('active');
        if (item.dataset.tab === tabId) {
            item.classList.add('active');
        }
    });
    
    document.querySelectorAll('.tab-panel').forEach(function(panel) {
        panel.classList.remove('active');
    });
    document.getElementById('tab-' + tabId).classList.add('active');
}

function renderAll() {
    renderGeneralSettings();
    renderProfiles();
    renderProviders();
    renderMemorySettings();
    renderMcpServers();
    renderAgentModels();
}

function renderMemorySettings() {
    const memoryEnabled = settings.memory && settings.memory.enabled ? 'true' : 'false';
    document.getElementById('memoryEnabled').value = memoryEnabled;
    
    const summarizerCost = settings.memory && settings.memory.summarizer_cost ? settings.memory.summarizer_cost : 'high';
    document.getElementById('memorySummarizerCost').value = summarizerCost;
    
    const recorderCost = settings.memory && settings.memory.recorder_cost ? settings.memory.recorder_cost : 'high';
    document.getElementById('memoryRecorderCost').value = recorderCost;
    
    const contextMessageCount = settings.memory?.context_message_count ?? 0;
    document.getElementById('memoryContextMessageCount').value = contextMessageCount;
}

function updateMemorySettings() {
    if (!settings.memory) {
        settings.memory = {};
    }
    settings.memory.enabled = document.getElementById('memoryEnabled').value === 'true';
    settings.memory.summarizer_cost = document.getElementById('memorySummarizerCost').value;
    settings.memory.recorder_cost = document.getElementById('memoryRecorderCost').value;
    const contextMessageCount = parseInt(document.getElementById('memoryContextMessageCount').value);
    settings.memory.context_message_count = isNaN(contextMessageCount) ? 0 : contextMessageCount;
    saveSettings();
}

function renderGeneralSettings() {
    document.getElementById('communicationTone').value = settings.communication_tone || 'concise_and_logical';
    document.getElementById('autonomyLevel').value = settings.autonomy_level || 'plan_approval_required';
    
    const securityMode = settings.security && settings.security.mode ? settings.security.mode : 'auto';
    document.getElementById('securityMode').value = securityMode;
    
    const modelQuality = settings.model_quality ? settings.model_quality : '';
    document.getElementById('modelQuality').value = modelQuality;
    
    document.getElementById('reviewLevel').value = settings.review_level || 'None';
    document.getElementById('fileModificationApi').value = settings.file_modification_api || 'Default';
    document.getElementById('runBuildTestOutputMode').value = settings.run_build_test_output_mode || 'ToolResponse';
    document.getElementById('defaultAgent').value = settings.default_agent || '';
    document.getElementById('autoContextBytes').value = settings.auto_context_bytes || 80000;
    document.getElementById('enableTypeAnalyzer').value = settings.enable_type_analyzer ? 'true' : 'false';
    document.getElementById('commandExecutionMode').value = settings.command_execution_mode || 'Direct';
    document.getElementById('spawnContextMode').value = settings.spawn_context_mode || 'Fork';
    document.getElementById('xmlToolMode').value = settings.xml_tool_mode ? 'true' : 'false';
    document.getElementById('disableCustomSteering').value = settings.disable_custom_steering ? 'true' : 'false';
}

function updateGeneralSettings() {
    settings.communication_tone = document.getElementById('communicationTone').value;
    settings.autonomy_level = document.getElementById('autonomyLevel').value;
    
    if (!settings.security) {
        settings.security = {};
    }
    settings.security.mode = document.getElementById('securityMode').value;
    
    const modelQualityValue = document.getElementById('modelQuality').value;
    settings.model_quality = modelQualityValue === '' ? null : modelQualityValue;
    
    settings.review_level = document.getElementById('reviewLevel').value;
    settings.file_modification_api = document.getElementById('fileModificationApi').value;
    settings.run_build_test_output_mode = document.getElementById('runBuildTestOutputMode').value;
    settings.default_agent = document.getElementById('defaultAgent').value;
    const autoContextBytes = parseInt(document.getElementById('autoContextBytes').value);
    settings.auto_context_bytes = isNaN(autoContextBytes) ? 80000 : autoContextBytes;
    settings.enable_type_analyzer = document.getElementById('enableTypeAnalyzer').value === 'true';
    settings.command_execution_mode = document.getElementById('commandExecutionMode').value;
    settings.spawn_context_mode = document.getElementById('spawnContextMode').value;
    settings.xml_tool_mode = document.getElementById('xmlToolMode').value === 'true';
    settings.disable_custom_steering = document.getElementById('disableCustomSteering').value === 'true';
    saveSettings();
}

function renderProfiles() {
    const select = document.getElementById('currentProfile');
    select.innerHTML = '';
    
    for (const profile of availableProfiles) {
        const option = document.createElement('option');
        option.value = profile;
        option.textContent = profile;
        if (profile === currentProfile) {
            option.selected = true;
        }
        select.appendChild(option);
    }
}

function switchProfile() {
    const profileName = document.getElementById('currentProfile').value;
    currentProfile = profileName;
    vscode.postMessage({
        type: 'switchProfile',
        profile: profileName
    });
}

function saveProfile() {
    const profileName = document.getElementById('newProfileName').value.trim();
    if (!profileName) {
        vscode.postMessage({
            type: 'error',
            message: 'Profile name is required'
        });
        return;
    }
    vscode.postMessage({
        type: 'saveProfile',
        profile: profileName
    });
    document.getElementById('newProfileName').value = '';
}

function renderMcpServers() {
    const list = document.getElementById('mcpList');
    list.innerHTML = '';
    
    if (!settings.mcp_servers || Object.keys(settings.mcp_servers).length === 0) {
        list.innerHTML = '<div style="color: var(--vscode-descriptionForeground);">No MCP servers configured</div>';
        return;
    }
    
    for (const [name, config] of Object.entries(settings.mcp_servers)) {
        const item = document.createElement('div');
        item.className = 'mcp-item';
        
        let mcpInfo = 'Command: ' + escapeHtml(config.command || '');
        if (config.args && config.args.length > 0) {
            mcpInfo += ', Args: ' + config.args.length;
        }
        if (config.env && Object.keys(config.env).length > 0) {
            mcpInfo += ', Env vars: ' + Object.keys(config.env).length;
        }
        
        item.innerHTML = '<div class="mcp-header">' +
            '<div class="mcp-name">' + escapeHtml(name) + '</div>' +
            '<div class="mcp-actions">' +
            '<button class="edit-btn" data-mcp="' + escapeHtml(name) + '">Edit</button>' +
            '<button class="danger delete-btn" data-mcp="' + escapeHtml(name) + '">Delete</button>' +
            '</div>' +
            '</div>' +
            '<div class="mcp-details">' + escapeHtml(mcpInfo) + '</div>';
        
        list.appendChild(item);
    }
}

function showAddMcpModal() {
    editingMcp = null;
    document.getElementById('mcpModalTitle').textContent = 'Add MCP Server';
    document.getElementById('mcpName').value = '';
    document.getElementById('mcpName').disabled = false;
    document.getElementById('mcpCommand').value = '';
    document.getElementById('mcpArgs').value = '';
    document.getElementById('mcpEnv').value = '';
    document.getElementById('mcpModal').style.display = 'block';
}

function editMcp(name) {
    editingMcp = name;
    const config = settings.mcp_servers[name];
    document.getElementById('mcpModalTitle').textContent = 'Edit MCP Server';
    document.getElementById('mcpName').value = name;
    document.getElementById('mcpName').disabled = true;
    document.getElementById('mcpCommand').value = config.command || '';
    document.getElementById('mcpArgs').value = config.args ? config.args.join('\n') : '';
    document.getElementById('mcpEnv').value = config.env ? Object.entries(config.env).map(([k, v]) => k + '=' + v).join('\n') : '';
    document.getElementById('mcpModal').style.display = 'block';
}

function closeMcpModal() {
    document.getElementById('mcpModal').style.display = 'none';
}

function saveMcp() {
    const name = document.getElementById('mcpName').value.trim();
    const command = document.getElementById('mcpCommand').value.trim();
    const argsText = document.getElementById('mcpArgs').value.trim();
    const envText = document.getElementById('mcpEnv').value.trim();
    
    if (!name) {
        vscode.postMessage({ type: 'error', message: 'MCP server name is required' });
        return;
    }
    
    if (!command) {
        vscode.postMessage({ type: 'error', message: 'Command is required' });
        return;
    }
    
    if (!editingMcp && settings.mcp_servers && settings.mcp_servers[name]) {
        vscode.postMessage({ type: 'error', message: 'MCP server with this name already exists' });
        return;
    }
    
    const config = { command };
    
    if (argsText) {
        config.args = argsText.split('\n').map(line => line.trim()).filter(line => line.length > 0);
    }
    
    const envResult = parseEnvironmentVariables(envText);
    if (!envResult.success) {
        vscode.postMessage({ type: 'error', message: envResult.message });
        return;
    }
    if (envResult.env) {
        config.env = envResult.env;
    }
    
    if (!settings.mcp_servers) {
        settings.mcp_servers = {};
    }
    settings.mcp_servers[name] = config;
    
    closeMcpModal();
    renderMcpServers();
    saveSettings();
}

function deleteMcp(name) {
    deletingMcp = name;
    deleteType = 'mcp';
    document.getElementById('deleteItemName').textContent = name;
    document.getElementById('deleteConfirmModal').style.display = 'block';
}

function renderAgentModels() {
    const list = document.getElementById('agentModelsList');
    list.innerHTML = '';
    
    if (!settings.agent_models || Object.keys(settings.agent_models).length === 0) {
        list.innerHTML = '<div style="color: var(--vscode-descriptionForeground);">No agent model overrides configured</div>';
        return;
    }
    
    for (const [agentName, config] of Object.entries(settings.agent_models)) {
        const item = document.createElement('div');
        item.className = 'agent-model-item';
        
        let modelInfo = 'Model: ' + escapeHtml(config.model || '');
        const params = [];
        if (config.temperature !== undefined) params.push('temp=' + config.temperature);
        if (config.max_tokens !== undefined) params.push('max_tokens=' + config.max_tokens);
        if (config.top_p !== undefined) params.push('top_p=' + config.top_p);
        if (config.reasoning_budget) params.push('reasoning=' + config.reasoning_budget);
        if (params.length > 0) {
            modelInfo += ' (' + params.join(', ') + ')';
        }
        
        item.innerHTML = '<div class="agent-model-header">' +
            '<div class="agent-model-name">' + escapeHtml(agentName) + '</div>' +
            '<div class="agent-model-actions">' +
            '<button class="edit-btn" data-agent="' + escapeHtml(agentName) + '">Edit</button>' +
            '<button class="danger delete-btn" data-agent="' + escapeHtml(agentName) + '">Delete</button>' +
            '</div>' +
            '</div>' +
            '<div class="agent-model-details">' + escapeHtml(modelInfo) + '</div>';
        
        list.appendChild(item);
    }
}

function showAddAgentModelModal() {
    editingAgentModel = null;
    document.getElementById('agentModelModalTitle').textContent = 'Add Agent Model';
    document.getElementById('agentName').value = '';
    document.getElementById('agentName').disabled = false;
    document.getElementById('agentModelName').value = '';
    document.getElementById('agentTemperature').value = '';
    document.getElementById('agentMaxTokens').value = '';
    document.getElementById('agentTopP').value = '';
    document.getElementById('agentReasoningBudget').value = '';
    document.getElementById('agentModelModal').style.display = 'block';
}

function editAgentModel(agentName) {
    editingAgentModel = agentName;
    const config = settings.agent_models[agentName];
    document.getElementById('agentModelModalTitle').textContent = 'Edit Agent Model';
    document.getElementById('agentName').value = agentName;
    document.getElementById('agentName').disabled = true;
    document.getElementById('agentModelName').value = config.model || '';
    document.getElementById('agentTemperature').value = config.temperature !== undefined ? config.temperature : '';
    document.getElementById('agentMaxTokens').value = config.max_tokens !== undefined ? config.max_tokens : '';
    document.getElementById('agentTopP').value = config.top_p !== undefined ? config.top_p : '';
    document.getElementById('agentReasoningBudget').value = config.reasoning_budget || '';
    document.getElementById('agentModelModal').style.display = 'block';
}

function closeAgentModelModal() {
    document.getElementById('agentModelModal').style.display = 'none';
}

function saveAgentModel() {
    const agentName = document.getElementById('agentName').value.trim();
    const modelName = document.getElementById('agentModelName').value.trim();
    
    if (!agentName) {
        vscode.postMessage({ type: 'error', message: 'Agent name is required' });
        return;
    }
    
    if (!modelName) {
        vscode.postMessage({ type: 'error', message: 'Model name is required' });
        return;
    }
    
    if (!editingAgentModel && settings.agent_models && settings.agent_models[agentName]) {
        vscode.postMessage({ type: 'error', message: 'Agent model override for this agent already exists' });
        return;
    }
    
    const config = { model: modelName };
    
    const temperature = parseFloat(document.getElementById('agentTemperature').value);
    if (!isNaN(temperature)) config.temperature = temperature;
    
    const maxTokens = parseInt(document.getElementById('agentMaxTokens').value);
    if (!isNaN(maxTokens)) config.max_tokens = maxTokens;
    
    const topP = parseFloat(document.getElementById('agentTopP').value);
    if (!isNaN(topP)) config.top_p = topP;
    
    const reasoningBudget = document.getElementById('agentReasoningBudget').value;
    if (reasoningBudget && reasoningBudget !== '') {
        config.reasoning_budget = reasoningBudget;
    }
    
    if (!settings.agent_models) {
        settings.agent_models = {};
    }
    settings.agent_models[agentName] = config;
    
    closeAgentModelModal();
    renderAgentModels();
    saveSettings();
}

function deleteAgentModel(agentName) {
    deletingAgentModel = agentName;
    deleteType = 'agent_model';
    document.getElementById('deleteItemName').textContent = agentName;
    document.getElementById('deleteConfirmModal').style.display = 'block';
}

function renderProviders() {
    const list = document.getElementById('providerList');
    list.innerHTML = '';
    
    if (!settings.providers || Object.keys(settings.providers).length === 0) {
        list.innerHTML = '<div style="color: var(--vscode-descriptionForeground);">No providers configured</div>';
        return;
    }
    
    for (const [name, config] of Object.entries(settings.providers)) {
        const isActive = name === settings.active_provider;
        const item = document.createElement('div');
        item.className = 'provider-item' + (isActive ? ' active' : '');
        
        let providerInfo = '';
        let providerTypeLabel = '';
        
        if (config.type === 'bedrock') {
            providerInfo = 'Profile: ' + escapeHtml(config.profile || '') + ', Region: ' + escapeHtml(config.region || 'us-west-2');
            providerTypeLabel = 'AWS Bedrock';
        } else if (config.type === 'openrouter') {
            providerInfo = 'API Key: ' + (config.api_key ? escapeHtml(config.api_key.substring(0, 12) + '...') : 'Not set');
            if (config.base_url) {
                providerInfo += ', Base URL: ' + escapeHtml(config.base_url);
            }
            providerTypeLabel = 'OpenRouter';
        } else if (config.type === 'claude_code') {
            providerInfo = 'Command: ' + escapeHtml(config.command || 'claude');
            if (config.extra_args && config.extra_args.length > 0) {
                providerInfo += ', Args: ' + config.extra_args.length;
            }
            if (config.env && Object.keys(config.env).length > 0) {
                providerInfo += ', Env vars: ' + Object.keys(config.env).length;
            }
            providerTypeLabel = 'Claude Code';
        } else {
            providerTypeLabel = escapeHtml(config.type || 'Unknown');
        }
        
        item.innerHTML = '<div class="provider-header">' +
            '<div class="provider-name">' +
            '<input type="radio" name="activeProvider" value="' + escapeHtml(name) + '" ' +
            (isActive ? 'checked' : '') + '>' +
            '<span>' + escapeHtml(name) + ' (' + escapeHtml(providerTypeLabel) + ')</span>' +
            '</div>' +
            '<div class="provider-actions">' +
            '<button class="edit-btn" data-provider="' + escapeHtml(name) + '">Edit</button>' +
            '<button class="danger delete-btn" data-provider="' + escapeHtml(name) + '">Delete</button>' +
            '</div>' +
            '</div>' +
            '<div class="provider-details">' + escapeHtml(providerInfo) + '</div>';
        
        list.appendChild(item);
    }
}

function setActiveProvider(name) {
    settings.active_provider = name;
    renderProviders();
    saveSettings();
}

function showAddProviderModal() {
    editingProvider = null;
    document.getElementById('modalTitle').textContent = 'Add Provider';
    document.getElementById('providerName').value = '';
    document.getElementById('providerName').disabled = false;
    document.getElementById('providerType').value = 'bedrock';
    updateProviderFields('bedrock');
    document.getElementById('providerModal').style.display = 'block';
}

function editProvider(name) {
    editingProvider = name;
    const config = settings.providers[name];
    document.getElementById('modalTitle').textContent = 'Edit Provider';
    document.getElementById('providerName').value = name;
    document.getElementById('providerName').disabled = true;
    document.getElementById('providerType').value = config.type;
    updateProviderFields(config.type, config);
    document.getElementById('providerModal').style.display = 'block';
}

function updateProviderFields(type, config = {}) {
    const fieldsDiv = document.getElementById('providerFields');
    
    if (type === 'bedrock') {
        fieldsDiv.innerHTML = '<div class="form-group">' +
            '<label for="awsProfile">AWS Profile</label>' +
            '<input type="text" id="awsProfile" value="' + escapeHtml(config.profile || 'default') + '" placeholder="default">' +
            '<div class="help-text">AWS profile name from ~/.aws/credentials</div>' +
            '</div>' +
            '<div class="form-group">' +
            '<label for="awsRegion">AWS Region</label>' +
            '<input type="text" id="awsRegion" value="' + escapeHtml(config.region || 'us-west-2') + '" placeholder="us-west-2">' +
            '<div class="help-text">AWS region (e.g., us-west-2, us-east-1)</div>' +
            '</div>';
    } else if (type === 'openrouter') {
        fieldsDiv.innerHTML = '<div class="form-group">' +
            '<label for="apiKey">API Key</label>' +
            '<input type="text" id="apiKey" value="' + escapeHtml(config.api_key || '') + '" placeholder="sk-or-v1-...">' +
            '<div class="help-text">Your OpenRouter API key</div>' +
            '</div>' +
            '<div class="form-group">' +
            '<label for="baseUrl">Base URL (Optional)</label>' +
            '<input type="text" id="baseUrl" value="' + escapeHtml(config.base_url || '') + '" placeholder="https://openrouter.ai/api/v1">' +
            '<div class="help-text">Custom base URL (leave empty for default)</div>' +
            '</div>';
    } else if (type === 'claude_code') {
        const extraArgsValue = config.extra_args ? config.extra_args.join('\n') : '';
        const envValue = config.env ? Object.entries(config.env).map(([k, v]) => k + '=' + v).join('\n') : '';
        fieldsDiv.innerHTML = '<div class="form-group">' +
            '<label for="claudeCommand">Command Path</label>' +
            '<input type="text" id="claudeCommand" value="' + escapeHtml(config.command || 'claude') + '" placeholder="claude">' +
            '<div class="help-text">Path to the Claude CLI executable (defaults to "claude")</div>' +
            '</div>' +
            '<div class="form-group">' +
            '<label for="claudeExtraArgs">Extra Arguments (Optional)</label>' +
            '<textarea id="claudeExtraArgs" rows="3" placeholder="One argument per line" style="width: 100%; resize: vertical; font-family: monospace;">' + escapeHtml(extraArgsValue) + '</textarea>' +
            '<div class="help-text">Additional command-line arguments, one per line</div>' +
            '</div>' +
            '<div class="form-group">' +
            '<label for="claudeEnv">Environment Variables (Optional)</label>' +
            '<textarea id="claudeEnv" rows="3" placeholder="KEY=VALUE\nANOTHER_KEY=value" style="width: 100%; resize: vertical; font-family: monospace;">' + escapeHtml(envValue) + '</textarea>' +
            '<div class="help-text">Environment variables in KEY=VALUE format, one per line</div>' +
            '</div>';
    }
}

function closeModal() {
    document.getElementById('providerModal').style.display = 'none';
}

function saveProvider() {
    const name = document.getElementById('providerName').value.trim();
    const type = document.getElementById('providerType').value;
    
    if (!name) {
        vscode.postMessage({
            type: 'error',
            message: 'Provider name is required'
        });
        return;
    }
    
    if (!editingProvider && settings.providers[name]) {
        vscode.postMessage({
            type: 'error',
            message: 'Provider with this name already exists'
        });
        return;
    }
    
    let config = { type };
    
    if (type === 'bedrock') {
        const profile = document.getElementById('awsProfile').value.trim() || 'default';
        const region = document.getElementById('awsRegion').value.trim() || 'us-west-2';
        config.profile = profile;
        config.region = region;
    } else if (type === 'openrouter') {
        const apiKey = document.getElementById('apiKey').value.trim();
        const baseUrl = document.getElementById('baseUrl').value.trim();
        
        if (!apiKey) {
            vscode.postMessage({
                type: 'error',
                message: 'API Key is required for OpenRouter providers'
            });
            return;
        }
        
        config.api_key = apiKey;
        if (baseUrl) {
            config.base_url = baseUrl;
        }
    } else if (type === 'claude_code') {
        const command = document.getElementById('claudeCommand').value.trim() || 'claude';
        const extraArgsText = document.getElementById('claudeExtraArgs').value.trim();
        const envText = document.getElementById('claudeEnv').value.trim();
        
        config.command = command;
        
        if (extraArgsText) {
            config.extra_args = extraArgsText.split('\n').map(line => line.trim()).filter(line => line.length > 0);
        }
        
        const envResult = parseEnvironmentVariables(envText);
        if (!envResult.success) {
            vscode.postMessage({ type: 'error', message: envResult.message });
            return;
        }
        if (envResult.env) {
            config.env = envResult.env;
        }
    }
    
    settings.providers[name] = config;
    
    // If this is the first provider, make it active
    if (Object.keys(settings.providers).length === 1) {
        settings.active_provider = name;
    }
    
    closeModal();
    renderProviders();
    saveSettings();
}

function deleteProvider(name) {
    if (name === settings.active_provider) {
        vscode.postMessage({
            type: 'error',
            message: 'Cannot delete the active provider'
        });
        return;
    }
    
    deletingProvider = name;
    deleteType = 'provider';
    document.getElementById('deleteItemName').textContent = name;
    document.getElementById('deleteConfirmModal').style.display = 'block';
}

function confirmDelete() {
    if (deleteType === 'provider' && deletingProvider) {
        delete settings.providers[deletingProvider];
        deletingProvider = null;
        renderProviders();
    } else if (deleteType === 'mcp' && deletingMcp) {
        delete settings.mcp_servers[deletingMcp];
        deletingMcp = null;
        renderMcpServers();
    } else if (deleteType === 'agent_model' && deletingAgentModel) {
        delete settings.agent_models[deletingAgentModel];
        deletingAgentModel = null;
        renderAgentModels();
    }
    saveSettings();
    document.getElementById('deleteConfirmModal').style.display = 'none';
}

function cancelDelete() {
    deletingProvider = null;
    deletingMcp = null;
    deletingAgentModel = null;
    deleteType = 'provider';
    document.getElementById('deleteConfirmModal').style.display = 'none';
}

function saveSettings() {
    vscode.postMessage({
        type: 'saveSettings',
        settings: settings
    });
}

function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

function parseEnvironmentVariables(envText) {
    if (!envText) {
        return { success: true, env: null };
    }
    
    const env = {};
    const lines = envText.split('\n');
    
    for (const line of lines) {
        const trimmed = line.trim();
        if (trimmed.length === 0) {
            continue;
        }
        const equalsIndex = trimmed.indexOf('=');
        if (equalsIndex === -1) {
            return { success: false, message: 'Invalid environment variable format: ' + trimmed + '. Use KEY=VALUE format.' };
        }
        const key = trimmed.substring(0, equalsIndex).trim();
        const value = trimmed.substring(equalsIndex + 1).trim();
        if (key.length === 0) {
            return { success: false, message: 'Environment variable key cannot be empty: ' + trimmed };
        }
        env[key] = value;
    }
    
    return { success: true, env: Object.keys(env).length > 0 ? env : null };
}

vscode.postMessage({ type: 'getSettings' });