import * as vscode from 'vscode';
import * as path from 'path';
import { ChatActorClient } from '../lib/client';

export class SettingsProvider {
    private panel: vscode.WebviewPanel | undefined;

    constructor(
        private context: vscode.ExtensionContext,
        private client: ChatActorClient
    ) {}

    public async show() {
        if (this.panel) {
            this.panel.reveal();
            return;
        }

        this.panel = vscode.window.createWebviewPanel(
            'tycodeSettings',
            'TyCode Settings',
            vscode.ViewColumn.One,
            {
                enableScripts: true,
                retainContextWhenHidden: true,
                localResourceRoots: [this.context.extensionUri]
            }
        );

        this.panel.webview.html = this.getWebviewContent();

        // Load current settings and send to webview
        const settings = await this.loadSettings();
        this.panel.webview.postMessage({
            type: 'loadSettings',
            settings: settings
        });

        // Handle messages from the webview
        this.panel.webview.onDidReceiveMessage(
            async message => {
                switch (message.type) {
                    case 'saveSettings':
                        await this.saveSettings(message.settings);
                        vscode.window.showInformationMessage('Settings saved successfully');
                        break;
                    case 'getSettings':
                        const currentSettings = await this.loadSettings();
                        this.panel?.webview.postMessage({
                            type: 'loadSettings',
                            settings: currentSettings
                        });
                        break;
                    case 'error':
                        vscode.window.showErrorMessage(message.message);
                        break;
                }
            },
            undefined,
            this.context.subscriptions
        );

        this.panel.onDidDispose(() => {
            this.panel = undefined;
        });
    }

    private async loadSettings(): Promise<any> {
        try {
            // Create a promise that will resolve when we get settings
            const settingsPromise = new Promise<any>((resolve, reject) => {
                const timeout = setTimeout(() => {
                    reject(new Error('Settings loading timeout'));
                }, 10000);
                
                // Start consuming events in the background
                this.consumeSettingsEvents(resolve, reject, timeout);
            });
            
            // Send the settings request
            await this.client.getSettings();
            
            // Wait for the settings to come back through events
            const settings = await settingsPromise;
            return settings;
            
        } catch (error) {
            console.error('[SettingsProvider] Error loading settings:', error);
            vscode.window.showErrorMessage(`Failed to load settings: ${error}`);
            
            // Return empty settings to allow webview to render blank form, user can configure providers
            return {
                active_provider: '',
                providers: {}
            };
        }
    }
    
    private async consumeSettingsEvents(resolve: (value: any) => void, reject: (error: Error) => void, timeout: NodeJS.Timeout): Promise<void> {
        try {
            for await (const event of this.client.events()) {
                if (event.kind === 'Settings') {
                    clearTimeout(timeout);
                    resolve(event.data);
                    return;
                }

                if (event.kind === 'Error') {
                    clearTimeout(timeout);
                    reject(new Error(event.data));
                    return;
                }
            }
        } catch (error) {
            clearTimeout(timeout);
            reject(error as Error);
        }
    }

    private async saveSettings(settings: any): Promise<void> {
        try {
            await this.client.saveSettings(settings);
            // Note: Settings reload needs to be handled through event stream
            console.warn('[SettingsProvider] Settings reload needs event stream integration');
        } catch (error) {
            console.error('Failed to save settings:', error);
            vscode.window.showErrorMessage(`Failed to save settings: ${error}`);
            throw error;
        }
    }

    private getSettingsCssUri(): vscode.Uri {
        return this.panel!.webview.asWebviewUri(
            vscode.Uri.joinPath(this.context.extensionUri, 'out', 'webview', 'settings.css')
        );
    }

    private getSettingsJsUri(): vscode.Uri {
        return this.panel!.webview.asWebviewUri(
            vscode.Uri.joinPath(this.context.extensionUri, 'out', 'webview', 'settings.js')
        );
    }

    private getWebviewContent(): string {
        const cssUri = this.getSettingsCssUri();
        const jsUri = this.getSettingsJsUri();
        const nonce = this.getNonce();
        
        return `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${this.panel!.webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
    <title>TyCode Settings</title>
    <link rel="stylesheet" href="${cssUri}">
</head>
<body>
    <h1>TyCode Settings</h1>
    
    <div class="section">
        <div class="section-title">Provider Configurations</div>
        <div class="provider-list" id="providerList">
            <!-- Providers will be dynamically added here -->
        </div>
        <button class="add-provider-btn" id="addProviderBtn">+ Add Provider</button>
    </div>
    
    <div class="actions">
        <button class="primary" id="saveSettingsBtn">Save Settings</button>
    </div>
    
    <!-- Add/Edit Provider Modal -->
    <div id="providerModal" class="modal">
        <div class="modal-content">
            <div class="modal-header" id="modalTitle">Add Provider</div>
            <div class="form-group">
                <label for="providerName">Name</label>
                <input type="text" id="providerName" placeholder="e.g., personal, work, dev">
                <div class="help-text">A unique name for this provider configuration</div>
            </div>
            <div class="form-group">
                <label for="providerType">Type</label>
                <select id="providerType">
                    <option value="bedrock">AWS Bedrock</option>
                    <option value="openrouter">OpenRouter</option>
                </select>
            </div>
            <div id="providerFields">
                <!-- Dynamic fields based on provider type -->
            </div>
            <div class="modal-footer">
                <button id="closeModalBtn">Cancel</button>
                <button class="primary" id="saveProviderBtn">Save</button>
            </div>
        </div>
    </div>
    
    <!-- Delete Confirmation Modal -->
    <div id="deleteConfirmModal" class="modal">
        <div class="modal-content" style="max-width: 400px;">
            <div class="modal-header">Confirm Delete</div>
            <div style="margin: 20px 0;">
                Are you sure you want to delete the provider "<span id="deleteProviderName"></span>"?
            </div>
            <div class="modal-footer">
                <button id="cancelDeleteBtn">Cancel</button>
                <button class="danger" id="confirmDeleteBtn">Delete</button>
            </div>
        </div>
    </div>
    
    <script nonce="${nonce}" src="${jsUri}"></script>
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

    public dispose() {
        if (this.panel) {
            this.panel.dispose();
        }
    }
}
