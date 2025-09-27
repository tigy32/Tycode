import { WebviewContext } from './context.js';

export function registerUiEventListeners(context: WebviewContext): void {
    const { dom, vscode } = context;

    dom.newTabButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'newChat' });
    });

    dom.welcomeNewChatButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'newChat' });
    });

    dom.welcomeSettingsButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'openSettings' });
    });
}
