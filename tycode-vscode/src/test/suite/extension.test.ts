import * as assert from 'assert';
import * as vscode from 'vscode';

suite('Extension Test Suite', () => {
    vscode.window.showInformationMessage('Start all tests.');

    test('Extension should be present', () => {
        assert.ok(vscode.extensions.getExtension('tycode.tycode'));
    });

    test('Extension should activate', async () => {
        const extension = vscode.extensions.getExtension('tycode.tycode');
        assert.ok(extension);
        
        await extension.activate();
        assert.ok(extension.isActive);
    });

    test('Commands should be registered', async () => {
        const commands = await vscode.commands.getCommands();
        
        assert.ok(commands.includes('tycode.openChat'));
        assert.ok(commands.includes('tycode.askAboutSelection'));
        assert.ok(commands.includes('tycode.applyChanges'));
        assert.ok(commands.includes('tycode.openSettings'));
    });

    test('Chat view should be registered', () => {
        const allViews = vscode.window.tabGroups.all
            .flatMap(group => group.tabs)
            .map(tab => tab.label);
        
        // View registration is checked by extension activation
        const extension = vscode.extensions.getExtension('tycode.tycode');
        assert.ok(extension?.packageJSON?.contributes?.views);
        
        const sidebarViews = extension.packageJSON.contributes.views['tycode-sidebar'];
        assert.ok(sidebarViews);
        assert.strictEqual(sidebarViews[0].id, 'tycode.chatView');
    });
});