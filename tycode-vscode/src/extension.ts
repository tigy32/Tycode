import * as vscode from 'vscode';
import { MainProvider } from './mainProvider';
import { SettingsProvider } from './settingsProvider';
import { ChatActorClient } from '../lib/client';

let mainProvider: MainProvider;
let settingsProvider: SettingsProvider;
let settingsClient: ChatActorClient;

export async function activate(context: vscode.ExtensionContext) {
    console.log('Tycode extension is activating...');

    // Create providers
    mainProvider = new MainProvider(context);

    // Register webview provider
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(
            'tycode.chatView',
            mainProvider,
            {
                webviewOptions: {
                    retainContextWhenHidden: true
                }
            }
        )
    );

    // Register commands
    context.subscriptions.push(
        vscode.commands.registerCommand('tycode.openChat', () => {
            mainProvider.openChat();
        })
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('tycode.askAboutSelection', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) {
                vscode.window.showWarningMessage('No active editor');
                return;
            }

            const selection = editor.document.getText(editor.selection);
            if (!selection) {
                vscode.window.showWarningMessage('No text selected');
                return;
            }

            // Open chat and send the selection
            mainProvider.openChat();
            await mainProvider.sendMessageToActiveChat(
                `Can you explain this code?\n\n\`\`\`${editor.document.languageId}\n${selection}\n\`\`\``
            );
        })
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('tycode.applyChanges', async (changes: string) => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) {
                vscode.window.showWarningMessage('No active editor');
                return;
            }

            await editor.edit(editBuilder => {
                const document = editor.document;
                const fullRange = new vscode.Range(
                    document.positionAt(0),
                    document.positionAt(document.getText().length)
                );
                editBuilder.replace(fullRange, changes);
            });
        })
    );

    // Register settings command
    context.subscriptions.push(
        vscode.commands.registerCommand('tycode.openSettings', async () => {
            // Create a settings client on demand
            if (!settingsClient) {
                const workspaceRoots = vscode.workspace.workspaceFolders?.map(f => f.uri.fsPath) || [];
                // Use default settings path (~/.tycode/settings.toml)
                settingsClient = new ChatActorClient(workspaceRoots);
                settingsProvider = new SettingsProvider(context, settingsClient);
            }
            settingsProvider.show();
        })
    );

    console.log('TyCode extension is now active!');
}

export function deactivate() {
    if (mainProvider) {
        mainProvider.dispose();
    }
    if (settingsProvider) {
        settingsProvider.dispose();
    }
    if (settingsClient) {
        settingsClient.close();
    }
}
