const vscode = acquireVsCodeApi();
let settings = {
    active_provider: 'default',
    providers: {}
};
let editingProvider = null;
let deletingProvider = null;

// Listen for messages from extension
window.addEventListener('message', event => {
    const message = event.data;
    switch (message.type) {
        case 'loadSettings':
            settings = message.settings;
            renderProviders();
            break;
    }
});

// Set up event listeners when DOM is loaded
document.addEventListener('DOMContentLoaded', function() {
    // Add provider button
    document.getElementById('addProviderBtn').addEventListener('click', showAddProviderModal);
    
    // Provider type dropdown in modal
    document.getElementById('providerType').addEventListener('change', function() {
        updateProviderFields(this.value);
    });
    
    // Modal buttons
    document.getElementById('closeModalBtn').addEventListener('click', closeModal);
    document.getElementById('saveProviderBtn').addEventListener('click', saveProvider);
    document.getElementById('cancelDeleteBtn').addEventListener('click', cancelDelete);
    document.getElementById('confirmDeleteBtn').addEventListener('click', confirmDelete);
    

    
    // Event delegation for dynamically generated provider list
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
});

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
        
        if (envText) {
            config.env = {};
            const envLines = envText.split('\n');
            for (const line of envLines) {
                const trimmed = line.trim();
                if (trimmed.length === 0) {
                    continue;
                }
                const equalsIndex = trimmed.indexOf('=');
                if (equalsIndex === -1) {
                    vscode.postMessage({
                        type: 'error',
                        message: 'Invalid environment variable format: ' + trimmed + '. Use KEY=VALUE format.'
                    });
                    return;
                }
                const key = trimmed.substring(0, equalsIndex).trim();
                const value = trimmed.substring(equalsIndex + 1).trim();
                if (key.length === 0) {
                    vscode.postMessage({
                        type: 'error',
                        message: 'Environment variable key cannot be empty: ' + trimmed
                    });
                    return;
                }
                config.env[key] = value;
            }
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
    
    // Show confirmation modal
    deletingProvider = name;
    document.getElementById('deleteProviderName').textContent = name;
    document.getElementById('deleteConfirmModal').style.display = 'block';
}

function confirmDelete() {
    if (deletingProvider) {
        delete settings.providers[deletingProvider];
        renderProviders();
        saveSettings();
        deletingProvider = null;
    }
    document.getElementById('deleteConfirmModal').style.display = 'none';
}

function cancelDelete() {
    deletingProvider = null;
    document.getElementById('deleteConfirmModal').style.display = 'none';
}

function saveSettings() {
    vscode.postMessage({
        type: 'saveSettings',
        settings: settings
    });
}

// Utility function to escape HTML
function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// Initial load
vscode.postMessage({ type: 'getSettings' });