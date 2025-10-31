import { VsCodeApi } from './types.js';

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
    let rendered = escapeHtml(content);

    rendered = rendered.replace(/```(\w+)?\n([\s\S]*?)```/g, (_match, lang, code) => {
        return `<div class="code-block-container">
                <pre><code class="language-${lang || 'plaintext'}">${code.trim()}</code></pre>
            </div>`;
    });

    rendered = rendered.replace(/`([^`]+)`/g, '<code>$1</code>');

    rendered = rendered.replace(/^######\s+(.+)$/gm, '<h6>$1</h6>');
    rendered = rendered.replace(/^#####\s+(.+)$/gm, '<h5>$1</h5>');
    rendered = rendered.replace(/^####\s+(.+)$/gm, '<h4>$1</h4>');
    rendered = rendered.replace(/^###\s+(.+)$/gm, '<h3>$1</h3>');
    rendered = rendered.replace(/^##\s+(.+)$/gm, '<h2>$1</h2>');
    rendered = rendered.replace(/^#\s+(.+)$/gm, '<h1>$1</h1>');

    rendered = rendered.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank">$1</a>');
    rendered = rendered.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    rendered = rendered.replace(/\*([^*]+)\*/g, '<em>$1</em>');

    rendered = rendered.replace(/\n/g, '<br>');

    rendered = rendered.replace(/(<h[1-6]>.*?)<br>(.*?<\/h[1-6]>)/g, '$1 $2');
    rendered = rendered.replace(/(<\/h[1-6]>)<br>/g, '$1');
    rendered = rendered.replace(/(<br>){2,}/g, '<br>');
    rendered = rendered.replace(/<br>(<h[1-6]>)/g, '$1');

    return rendered;
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

        const insertButton = document.createElement('button');
        insertButton.className = 'code-action-button';
        insertButton.textContent = 'Insert';
        insertButton.onclick = () => {
            const codeElement = block.querySelector('code');
            const code = codeElement?.textContent || '';
            vscode.postMessage({
                type: 'insertCode',
                code
            });
        };

        actionsDiv.appendChild(copyButton);
        actionsDiv.appendChild(insertButton);
        block.appendChild(actionsDiv);
    });
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
