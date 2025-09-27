import { ConversationState, VsCodeApi } from './types.js';

export interface DomElements {
    welcomeScreen: HTMLDivElement;
    tabBar: HTMLDivElement;
    tabsContainer: HTMLDivElement;
    conversationsContainer: HTMLDivElement;
    newTabButton: HTMLButtonElement | null;
    welcomeNewChatButton: HTMLButtonElement | null;
    welcomeSettingsButton: HTMLButtonElement | null;
}

export class ConversationStore {
    private readonly conversations = new Map<string, ConversationState>();

    public set(id: string, state: ConversationState): void {
        this.conversations.set(id, state);
    }

    public get(id: string): ConversationState | undefined {
        return this.conversations.get(id);
    }

    public delete(id: string): void {
        this.conversations.delete(id);
    }

    public clear(): void {
        this.conversations.clear();
    }

    public size(): number {
        return this.conversations.size;
    }

    public values(): IterableIterator<ConversationState> {
        return this.conversations.values();
    }

    public entries(): IterableIterator<[string, ConversationState]> {
        return this.conversations.entries();
    }
}

export class WebviewContext {
    public activeConversationId: string | null = null;
    public readonly retryElements = new Map<string, HTMLElement>();

    constructor(
        public readonly vscode: VsCodeApi,
        public readonly dom: DomElements,
        public readonly store: ConversationStore
    ) {}
}

export function initializeDomElements(): DomElements {
    const welcomeScreen = document.getElementById('welcome-screen') as HTMLDivElement | null;
    const tabBar = document.getElementById('tab-bar') as HTMLDivElement | null;
    const tabsContainer = document.getElementById('tabs') as HTMLDivElement | null;
    const conversationsContainer = document.getElementById('conversations-container') as HTMLDivElement | null;
    const newTabButton = document.getElementById('new-tab-button') as HTMLButtonElement | null;
    const welcomeNewChatButton = document.getElementById('welcome-new-chat') as HTMLButtonElement | null;
    const welcomeSettingsButton = document.getElementById('welcome-settings') as HTMLButtonElement | null;

    if (!welcomeScreen || !tabBar || !tabsContainer || !conversationsContainer) {
        throw new Error('TyCode webview is missing required root elements.');
    }

    return {
        welcomeScreen,
        tabBar,
        tabsContainer,
        conversationsContainer,
        newTabButton,
        welcomeNewChatButton,
        welcomeSettingsButton
    };
}
