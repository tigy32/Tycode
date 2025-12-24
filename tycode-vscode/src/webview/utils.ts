import { marked } from 'marked';
import { VsCodeApi } from './types.js';

marked.setOptions({
    gfm: true,
    breaks: true
});

export function escapeHtml(text: string): string {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

export function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) {
        return 'unknown size';
    }

    if (bytes < 1024) {
        return `${bytes} B`;
    }

    const units = ['KB', 'MB', 'GB', 'TB'];
    let value = bytes / 1024;
    let unitIndex = 0;

    while (value >= 1024 && unitIndex < units.length - 1) {
        value /= 1024;
        unitIndex += 1;
    }

    const precision = value < 10 ? 1 : 0;
    return `${value.toFixed(precision)} ${units[unitIndex]}`;
}

export function renderContent(content: string): string {
    const result = marked.parse(content, { async: false }) as string;

    // Container div enables absolute positioning of copy action buttons
    return result.replace(
        /<pre><code([^>]*)>/g,
        '<div class="code-block-container"><pre><code$1>'
    ).replace(
        /<\/code><\/pre>/g,
        '</code></pre></div>'
    );
}

export function addCodeActions(messageDiv: HTMLElement, vscode: VsCodeApi): void {
    const codeBlocks = messageDiv.querySelectorAll<HTMLElement>('.code-block-container');

    codeBlocks.forEach(block => {
        const actionsDiv = document.createElement('div');
        actionsDiv.className = 'code-actions';

        const copyButton = document.createElement('button');
        copyButton.className = 'code-action-button';
        copyButton.textContent = 'Copy';
        copyButton.onclick = () => {
            const codeElement = block.querySelector('code');
            const code = codeElement?.textContent || '';
            vscode.postMessage({
                type: 'copyCode',
                code
            });
        };

        actionsDiv.appendChild(copyButton);
        block.appendChild(actionsDiv);
    });
}

export function addMessageCopyButton(messageDiv: HTMLElement, rawContent: string, vscode: VsCodeApi): void {
    const copyButtonContainer = document.createElement('div');
    copyButtonContainer.className = 'message-copy-container';

    const copyButton = document.createElement('button');
    copyButton.className = 'message-copy-button';
    copyButton.innerHTML = '⧉';
    copyButton.title = 'Copy message';
    copyButton.onclick = () => {
        vscode.postMessage({
            type: 'copyCode',
            code: rawContent
        });
    };

    copyButtonContainer.appendChild(copyButton);

    const footer = messageDiv.querySelector('.message-footer');
    if (footer) {
        footer.insertBefore(copyButtonContainer, footer.firstChild);
    } else {
        messageDiv.appendChild(copyButtonContainer);
    }
}

export function getRoleFromSender(sender: any): string {
    if (sender === 'User') {
        return 'user';
    }
    if (sender === 'System') {
        return 'system';
    }
    if (sender === 'Warning') {
        return 'warning';
    }
    if (sender === 'Error') {
        return 'error';
    }
    if (typeof sender === 'object' && sender !== null && 'Assistant' in sender) {
        return 'assistant';
    }
    console.error('Unknown sender type:', sender);
    return 'system';
}
