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
        
        // Load and send available profiles
        try {
            const profiles = await this.client.listProfiles();
            this.panel.webview.postMessage({
                type: 'loadProfiles',
                profiles: profiles
            });
        } catch (error) {
            console.error('Failed to load profiles:', error);
        }

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
                    case 'switchProfile':
                        await this.switchProfile(message.profile);
                        break;
                    case 'saveProfile':
                        await this.saveProfileAs(message.profile);
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

    private async switchProfile(profileName: string): Promise<void> {
        try {
            await this.client.switchProfile(profileName);
            const settings = await this.loadSettings();
            this.panel?.webview.postMessage({
                type: 'loadSettings',
                settings: settings
            });
            
            // Reload profiles list after switching
            const profiles = await this.client.listProfiles();
            this.panel?.webview.postMessage({
                type: 'loadProfiles',
                profiles: profiles
            });
            
            vscode.window.showInformationMessage(`Switched to profile: ${profileName}`);
        } catch (error) {
            console.error('Failed to switch profile:', error);
            vscode.window.showErrorMessage(`Failed to switch profile: ${error}`);
        }
    }

    private async saveProfileAs(profileName: string): Promise<void> {
        try {
            await this.client.saveProfileAs(profileName);
            
            // Reload profiles list after saving
            const profiles = await this.client.listProfiles();
            this.panel?.webview.postMessage({
                type: 'loadProfiles',
                profiles: profiles
            });
            
            vscode.window.showInformationMessage(`Profile saved: ${profileName}`);
        } catch (error) {
            console.error('Failed to save profile:', error);
            vscode.window.showErrorMessage(`Failed to save profile: ${error}`);
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
    
    <!-- Profile Management Section -->
    <div class="section">
        <div class="section-title">Profile Management</div>
        <div class="profile-controls">
            <div class="form-group" style="flex: 1;">
                <label for="currentProfile">Current Profile</label>
                <select id="currentProfile">
                    <!-- Profiles will be dynamically populated -->
                </select>
            </div>
            <div class="form-group" style="flex: 1;">
                <label for="newProfileName">Save As New Profile</label>
                <div style="display: flex; gap: 10px;">
                    <input type="text" id="newProfileName" placeholder="Profile name">
                    <button id="saveProfileBtn">Save</button>
                </div>
            </div>
        </div>
    </div>
    
    <!-- Tab Layout -->
    <div class="settings-layout">
        <nav class="settings-nav">
            <button class="nav-item active" data-tab="general">General</button>
            <button class="nav-item" data-tab="providers">Providers</button>
            <button class="nav-item" data-tab="memory">Memory</button>
            <button class="nav-item" data-tab="mcp">MCP Servers</button>
            <button class="nav-item" data-tab="agents">Agent Models</button>
            <button class="nav-item" data-tab="advanced">Advanced</button>
        </nav>
        
        <div class="settings-content">
            <!-- General Tab -->
            <div class="tab-panel active" id="tab-general">
                <div class="tab-title">General Settings</div>
                <div class="settings-grid">
                    <div class="form-group">
                        <label for="securityMode">Security Mode</label>
                        <select id="securityMode">
                            <option value="read_only">Read Only</option>
                            <option value="auto">Auto</option>
                            <option value="all">All</option>
                        </select>
                        <div class="help-text">Controls which tools the AI can use: Read Only (read files only), Auto (read + write, requires approval for dangerous operations), All (unrestricted access)</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="modelQuality">Model Quality (Cost Limit)</label>
                        <select id="modelQuality">
                            <option value="free">Free</option>
                            <option value="low">Low</option>
                            <option value="medium">Medium</option>
                            <option value="high">High</option>
                            <option value="unlimited">Unlimited</option>
                        </select>
                        <div class="help-text">Limits the maximum cost/quality of AI models used: Free (smallest models), Low/Medium/High (progressively larger models), Unlimited (all models including most expensive)</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="reviewLevel">Review Level</label>
                        <select id="reviewLevel">
                            <option value="None">None</option>
                            <option value="Task">Task</option>
                        </select>
                        <div class="help-text">None (no review), Task (AI reviews code changes line-by-line before committing to check style compliance and potential issues)</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="defaultAgent">Default Agent</label>
                        <input type="text" id="defaultAgent" placeholder="e.g., one_shot">
                        <div class="help-text">Which agent handles new conversations by default. Common agents: one_shot (single-pass implementation), coder (iterative development), recon (codebase exploration). Leave empty for system default.</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="communicationTone">Communication Tone</label>
                        <select id="communicationTone">
                            <option value="concise_and_logical">Concise and Logical</option>
                            <option value="warm_and_flowy">Warm and Flowy</option>
                            <option value="cat">Cat</option>
                            <option value="meme">Meme</option>
                        </select>
                        <div class="help-text">Sets the AI's communication style: Concise and Logical (terse, vulcan-like), Warm and Flowy (friendly, encouraging), Cat (feline personality with cat mannerisms)</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="autonomyLevel">Autonomy Level</label>
                        <select id="autonomyLevel">
                            <option value="PlanApprovalRequired">Plan Approval Required</option>
                            <option value="FullyAutonomous">Fully Autonomous</option>
                        </select>
                        <div class="help-text">Plan Approval Required (agent presents a plan and waits for approval before implementing), Fully Autonomous (agent proceeds directly with implementation)</div>
                    </div>
                </div>
            </div>
            
            <!-- Providers Tab -->
            <div class="tab-panel" id="tab-providers">
                <div class="tab-title">Provider Configurations</div>
                <div class="provider-list" id="providerList">
                    <!-- Providers will be dynamically added here -->
                </div>
                <button class="add-provider-btn" id="addProviderBtn">+ Add Provider</button>
            </div>
            
            <!-- Memory Tab -->
            <div class="tab-panel" id="tab-memory">
                <div class="tab-title">Memory Settings</div>
                <div class="help-text" style="margin-bottom: 20px;">Configure the memory agent that maintains context across conversations</div>
                <div class="settings-grid">
                    <div class="form-group">
                        <label for="memoryEnabled">Memory Agent</label>
                        <select id="memoryEnabled">
                            <option value="false">Disabled</option>
                            <option value="true">Enabled</option>
                        </select>
                        <div class="help-text">Enable the memory agent to persist learned context and patterns across conversations. When enabled, the agent will maintain notes about your codebase and preferences.</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="memorySummarizerCost">Summarizer Cost Level</label>
                        <select id="memorySummarizerCost">
                            <option value="low">Low</option>
                            <option value="medium">Medium</option>
                            <option value="high">High</option>
                        </select>
                        <div class="help-text">Model cost tier for memory summarization. Higher cost uses more capable models for better summaries.</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="memoryRecorderCost">Recorder Cost Level</label>
                        <select id="memoryRecorderCost">
                            <option value="low">Low</option>
                            <option value="medium">Medium</option>
                            <option value="high">High</option>
                        </select>
                        <div class="help-text">Model cost tier for memory recording. Higher cost uses more capable models for extracting and storing context.</div>
                    </div>
                </div>
            </div>
            
            <!-- MCP Servers Tab -->
            <div class="tab-panel" id="tab-mcp">
                <div class="tab-title">MCP Servers</div>
                <div class="help-text" style="margin-bottom: 15px;">Model Context Protocol servers provide additional tools and capabilities</div>
                <div class="mcp-list" id="mcpList">
                    <!-- MCP servers will be dynamically added here -->
                </div>
                <button class="add-mcp-btn" id="addMcpBtn">+ Add MCP Server</button>
            </div>
            
            <!-- Agent Models Tab -->
            <div class="tab-panel" id="tab-agents">
                <div class="tab-title">Agent Model Overrides</div>
                <div class="help-text" style="margin-bottom: 15px;">Configure specific models for individual agents (overrides global settings)</div>
                <div class="agent-models-list" id="agentModelsList">
                    <!-- Agent models will be dynamically added here -->
                </div>
                <button class="add-agent-model-btn" id="addAgentModelBtn">+ Add Agent Model</button>
            </div>
            
            <!-- Advanced Tab -->
            <div class="tab-panel" id="tab-advanced">
                <div class="tab-title">Advanced Configuration</div>
                <div class="settings-grid">
                    <div class="form-group">
                        <label for="fileModificationApi">File Modification API</label>
                        <select id="fileModificationApi">
                            <option value="Default">Default</option>
                            <option value="Patch">Patch</option>
                            <option value="FindReplace">Find & Replace</option>
                        </select>
                        <div class="help-text">How the AI applies file edits: Default (direct modifications), Patch (unified diff format), Find & Replace (search and replace blocks). Choose based on model capabilities.</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="runBuildTestOutputMode">Build Test Output Mode</label>
                        <select id="runBuildTestOutputMode">
                            <option value="ToolResponse">Tool Response</option>
                            <option value="Context">Context</option>
                        </select>
                        <div class="help-text">Tool Response (output sent directly to AI for processing), Context (output added to conversation context for visibility). Context mode uses more tokens but provides transparency.</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="autoContextBytes">Auto Context Bytes</label>
                        <input type="number" id="autoContextBytes" min="0" placeholder="80000">
                        <div class="help-text">Maximum size (in bytes) for automatically including directory structure in conversation context. Larger values provide more context but use more tokens. Default: 80,000 bytes (~80KB).</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="enableTypeAnalyzer">Enable Type Analyzer</label>
                        <select id="enableTypeAnalyzer">
                            <option value="false">Disabled</option>
                            <option value="true">Enabled</option>
                        </select>
                        <div class="help-text">Enable type analyzer tools (search_types, get_type_docs) for Rust projects</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="spawnContextMode">Spawn Context Mode</label>
                        <select id="spawnContextMode">
                            <option value="Fork">Fork</option>
                            <option value="Fresh">Fresh</option>
                        </select>
                        <div class="help-text">Fork copies parent conversation to sub-agents. Fresh starts sub-agents with empty context.</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="xmlToolMode">XML Tool Mode</label>
                        <select id="xmlToolMode">
                            <option value="false">Disabled</option>
                            <option value="true">Enabled</option>
                        </select>
                        <div class="help-text">Enable XML-based tool calling instead of native tool use</div>
                    </div>
                    
                    <div class="form-group">
                        <label for="disableCustomSteering">Custom Steering</label>
                        <select id="disableCustomSteering">
                            <option value="false">Enabled</option>
                            <option value="true">Disabled</option>
                        </select>
                        <div class="help-text">When disabled, only built-in steering documents are used. Custom .tycode documents and external agent configs (Cursor, Cline, Roo, Kiro) are ignored.</div>
                    </div>
                </div>
            </div>
        </div>
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
                    <option value="claude_code">Claude Code</option>
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
    
    <!-- Add/Edit MCP Server Modal -->
    <div id="mcpModal" class="modal">
        <div class="modal-content">
            <div class="modal-header" id="mcpModalTitle">Add MCP Server</div>
            <div class="form-group">
                <label for="mcpName">Name</label>
                <input type="text" id="mcpName" placeholder="e.g., database, filesystem">
                <div class="help-text">A unique name for this MCP server</div>
            </div>
            <div class="form-group">
                <label for="mcpCommand">Command</label>
                <input type="text" id="mcpCommand" placeholder="e.g., npx, python">
                <div class="help-text">The command to start the MCP server</div>
            </div>
            <div class="form-group">
                <label for="mcpArgs">Arguments (Optional)</label>
                <textarea id="mcpArgs" rows="3" placeholder="One argument per line" style="width: 100%; resize: vertical; font-family: monospace;"></textarea>
                <div class="help-text">Command arguments, one per line</div>
            </div>
            <div class="form-group">
                <label for="mcpEnv">Environment Variables (Optional)</label>
                <textarea id="mcpEnv" rows="3" placeholder="KEY=VALUE\nANOTHER_KEY=value" style="width: 100%; resize: vertical; font-family: monospace;"></textarea>
                <div class="help-text">Environment variables in KEY=VALUE format, one per line</div>
            </div>
            <div class="modal-footer">
                <button id="closeMcpModalBtn">Cancel</button>
                <button class="primary" id="saveMcpBtn">Save</button>
            </div>
        </div>
    </div>
    
    <!-- Add/Edit Agent Model Modal -->
    <div id="agentModelModal" class="modal">
        <div class="modal-content">
            <div class="modal-header" id="agentModelModalTitle">Add Agent Model</div>
            <div class="form-group">
                <label for="agentName">Agent Name</label>
                <input type="text" id="agentName" placeholder="e.g., coder, recon">
                <div class="help-text">The agent to configure</div>
            </div>
            <div class="form-group">
                <label for="agentModelName">Model Name</label>
                <input type="text" id="agentModelName" placeholder="e.g., claude-3-5-sonnet-20241022">
                <div class="help-text">The model identifier</div>
            </div>
            <div class="form-group">
                <label for="agentTemperature">Temperature (Optional)</label>
                <input type="number" id="agentTemperature" min="0" max="2" step="0.1" placeholder="1.0">
                <div class="help-text">Controls randomness (0.0 - 2.0)</div>
            </div>
            <div class="form-group">
                <label for="agentMaxTokens">Max Tokens (Optional)</label>
                <input type="number" id="agentMaxTokens" min="1" placeholder="4096">
                <div class="help-text">Maximum tokens in response</div>
            </div>
            <div class="form-group">
                <label for="agentTopP">Top P (Optional)</label>
                <input type="number" id="agentTopP" min="0" max="1" step="0.01" placeholder="0.95">
                <div class="help-text">Nucleus sampling threshold (0.0 - 1.0)</div>
            </div>
            <div class="form-group">
                <label for="agentReasoningBudget">Reasoning Budget (Optional)</label>
                <select id="agentReasoningBudget">
                    <option value="">Not set</option>
                    <option value="off">Off</option>
                    <option value="low">Low</option>
                    <option value="high">High</option>
                </select>
                <div class="help-text">Extended thinking budget for reasoning models</div>
            </div>
            <div class="modal-footer">
                <button id="closeAgentModelModalBtn">Cancel</button>
                <button class="primary" id="saveAgentModelBtn">Save</button>
            </div>
        </div>
    </div>
    
    <!-- Delete Confirmation Modal -->
    <div id="deleteConfirmModal" class="modal">
        <div class="modal-content" style="max-width: 400px;">
            <div class="modal-header">Confirm Delete</div>
            <div style="margin: 20px 0;">
                Are you sure you want to delete "<span id="deleteItemName"></span>"?
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
