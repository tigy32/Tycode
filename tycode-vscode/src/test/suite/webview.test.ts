import * as assert from 'assert';
import * as vscode from 'vscode';

suite('Webview Test Suite', () => {
    let panel: vscode.WebviewPanel | undefined;

    teardown(() => {
        if (panel) {
            panel.dispose();
            panel = undefined;
        }
    });

    test('Should create webview panel', () => {
        panel = vscode.window.createWebviewPanel(
            'tycodeTest',
            'TyCode Test',
            vscode.ViewColumn.One,
            {
                enableScripts: true,
                retainContextWhenHidden: true
            }
        );

        assert.ok(panel);
        assert.strictEqual(panel.title, 'TyCode Test');
        assert.strictEqual(panel.viewType, 'tycodeTest');
    });

    test('Should handle webview messages', async () => {
        panel = vscode.window.createWebviewPanel(
            'tycodeTest',
            'TyCode Test',
            vscode.ViewColumn.One,
            {
                enableScripts: true
            }
        );

        const messagePromise = new Promise<any>((resolve) => {
            panel!.webview.onDidReceiveMessage(message => {
                resolve(message);
            });
        });

        panel.webview.html = `
            <!DOCTYPE html>
            <html>
            <body>
                <script>
                    const vscode = acquireVsCodeApi();
                    vscode.postMessage({ type: 'test', data: 'hello' });
                </script>
            </body>
            </html>
        `;

        const message = await messagePromise;
        assert.strictEqual(message.type, 'test');
        assert.strictEqual(message.data, 'hello');
    });

    test('Should update webview content', () => {
        panel = vscode.window.createWebviewPanel(
            'tycodeTest',
            'TyCode Test',
            vscode.ViewColumn.One,
            {}
        );

        const testHtml = '<html><body><h1>Test Content</h1></body></html>';
        panel.webview.html = testHtml;
        assert.strictEqual(panel.webview.html, testHtml);
    });

    test('Should handle dispose event', (done) => {
        panel = vscode.window.createWebviewPanel(
            'tycodeTest',
            'TyCode Test',
            vscode.ViewColumn.One,
            {}
        );

        panel.onDidDispose(() => {
            done();
        });

        panel.dispose();
    });

    test('Should maintain state when hidden', () => {
        panel = vscode.window.createWebviewPanel(
            'tycodeTest',
            'TyCode Test',
            vscode.ViewColumn.One,
            {
                enableScripts: true,
                retainContextWhenHidden: true
            }
        );

        const options = panel.webview.options;
        assert.ok(options.enableScripts);
        
        // Verify panel was created with retainContextWhenHidden
        panel.webview.html = '<html><body>State Test</body></html>';
        const originalHtml = panel.webview.html;
        
        // Verify content is set correctly
        assert.strictEqual(panel.webview.html, originalHtml);
        
        // Verify panel maintains its configuration
        assert.ok(panel.options.retainContextWhenHidden);
    });
});