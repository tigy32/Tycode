import { WebviewContext } from './context.js';

export function registerUiEventListeners(context: WebviewContext): void {
    const { dom, vscode } = context;

    // + button opens dropdown menu
    dom.newTabButton?.addEventListener('click', (e: MouseEvent) => {
        e.stopPropagation();
        if (!dom.newTabDropdown) return;

        const isVisible = dom.newTabDropdown.style.display !== 'none';
        if (isVisible) {
            dom.newTabDropdown.style.display = 'none';
            return;
        }
        dom.newTabDropdown.style.display = 'block';
        vscode.postMessage({ type: 'requestSessionsList' });
    });

    // "New Chat" option in dropdown
    dom.newChatOption?.addEventListener('click', () => {
        if (dom.newTabDropdown) {
            dom.newTabDropdown.style.display = 'none';
        }
        vscode.postMessage({ type: 'newChat' });
    });

    dom.welcomeNewChatButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'newChat' });
    });

    dom.welcomeSettingsButton?.addEventListener('click', () => {
        vscode.postMessage({ type: 'openSettings' });
    });

    // Close dropdown when clicking outside
    document.addEventListener('click', (e: MouseEvent) => {
        if (!dom.newTabDropdown || !dom.newTabButton) return;

        const target = e.target as Node;
        const isClickInsideDropdown = dom.newTabDropdown.contains(target);
        const isClickOnButton = dom.newTabButton.contains(target);

        if (!isClickInsideDropdown && !isClickOnButton) {
            dom.newTabDropdown.style.display = 'none';
        }
    });
}
