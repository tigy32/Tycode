import { initializeDomElements, ConversationStore, WebviewContext } from './context.js';
import { createConversationController } from './conversationController.js';
import { registerUiEventListeners } from './uiListeners.js';
import { routeMessage } from './messageRouter.js';
import { VsCodeApi, WebviewMessageInbound } from './types.js';

declare function acquireVsCodeApi(): VsCodeApi;

(function bootstrap() {
    const vscode = acquireVsCodeApi();
    const dom = initializeDomElements();
    const store = new ConversationStore();
    const context = new WebviewContext(vscode, dom, store);

    registerUiEventListeners(context);

    const controller = createConversationController(context);
    controller.registerGlobalListeners();

    window.addEventListener('message', (event: MessageEvent<WebviewMessageInbound>) => {
        routeMessage(controller, event.data);
    });

    vscode.postMessage({ type: 'requestSessionsList' });
})();
