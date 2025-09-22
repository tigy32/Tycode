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
    
    // Save settings button
    document.getElementById('saveSettingsBtn').addEventListener('click', saveSettings);
    
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
            providerInfo = 'Profile: ' + config.profile + ', Region: ' + (config.region || 'us-west-2');
            providerTypeLabel = 'AWS Bedrock';
        } else if (config.type === 'openrouter') {
            providerInfo = 'API Key: ' + (config.api_key ? config.api_key.substring(0, 12) + '...' : 'Not set');
            if (config.base_url) {
                providerInfo += ', Base URL: ' + config.base_url;
            }
            providerTypeLabel = 'OpenRouter';
        } else {
            providerTypeLabel = config.type || 'Unknown';
        }
        
        item.innerHTML = '<div class="provider-header">' +
            '<div class="provider-name">' +
            '<input type="radio" name="activeProvider" value="' + name + '" ' +
            (isActive ? 'checked' : '') + '>' +
            '<span>' + name + ' (' + providerTypeLabel + ')</span>' +
            '</div>' +
            '<div class="provider-actions">' +
            '<button class="edit-btn" data-provider="' + name + '">Edit</button>' +
            '<button class="danger delete-btn" data-provider="' + name + '">Delete</button>' +
            '</div>' +
            '</div>' +
            '<div class="provider-details">' + providerInfo + '</div>';
        
        list.appendChild(item);
    }
}

function setActiveProvider(name) {
    settings.active_provider = name;
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
            '<input type="text" id="awsProfile" value="' + (config.profile || 'default') + '" placeholder="default">' +
            '<div class="help-text">AWS profile name from ~/.aws/credentials</div>' +
            '</div>' +
            '<div class="form-group">' +
            '<label for="awsRegion">AWS Region</label>' +
            '<input type="text" id="awsRegion" value="' + (config.region || 'us-west-2') + '" placeholder="us-west-2">' +
            '<div class="help-text">AWS region (e.g., us-west-2, us-east-1)</div>' +
            '</div>';
    } else if (type === 'openrouter') {
        fieldsDiv.innerHTML = '<div class="form-group">' +
            '<label for="apiKey">API Key</label>' +
            '<input type="text" id="apiKey" value="' + (config.api_key || '') + '" placeholder="sk-or-v1-...">' +
            '<div class="help-text">Your OpenRouter API key</div>' +
            '</div>' +
            '<div class="form-group">' +
            '<label for="baseUrl">Base URL (Optional)</label>' +
            '<input type="text" id="baseUrl" value="' + (config.base_url || '') + '" placeholder="https://openrouter.ai/api/v1">' +
            '<div class="help-text">Custom base URL (leave empty for default)</div>' +
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
    }
    
    settings.providers[name] = config;
    
    // If this is the first provider, make it active
    if (Object.keys(settings.providers).length === 1) {
        settings.active_provider = name;
    }
    
    closeModal();
    renderProviders();
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

// Initial load
vscode.postMessage({ type: 'getSettings' });