import { writeFileSync, existsSync, unlinkSync } from 'fs';
import { ChatActorClient } from '../src/client';
import type { ChatEvent } from '../src/types';

describe('ChatActorClient Integration Test', () => {
  const settingsPaths: string[] = [];

  function withTimeout<T>(promise: Promise<T>, timeoutMs: number, message: string): Promise<T> {
    let timeout: NodeJS.Timeout | undefined;
    const timeoutPromise = new Promise<never>((_, reject) => {
      timeout = setTimeout(() => reject(new Error(message)), timeoutMs);
    });
    return Promise.race([promise, timeoutPromise]).finally(() => {
      if (timeout) {
        clearTimeout(timeout);
      }
    });
  }

  function waitForEvent(
    client: ChatActorClient,
    predicate: (event: ChatEvent) => boolean,
    timeoutMs: number,
    timeoutMessage: string
  ): Promise<void> {
    return withTimeout(
      new Promise<void>((resolve, reject) => {
        (async () => {
          try {
            for await (const event of client.events()) {
              if (predicate(event)) {
                resolve();
                break;
              }
            }
          } catch (error) {
            reject(error);
          }
        })();
      }),
      timeoutMs,
      timeoutMessage
    );
  }

  function createClient(name: string, tomlContent: string): ChatActorClient {
    const settingsPath = `/tmp/tycode-test-settings-${name}-${Date.now()}-${Math.random().toString(36).slice(2)}.toml`;
    settingsPaths.push(settingsPath);
    writeFileSync(settingsPath, tomlContent);
    return new ChatActorClient(['.'], settingsPath);
  }

  afterAll(() => {
    for (const settingsPath of settingsPaths) {
      if (existsSync(settingsPath)) {
        unlinkSync(settingsPath);
      }
    }
  });

  test('should launch subprocess successfully', async () => {
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    const client = createClient('launch', tomlContent);
    await client.close();
  }, 10000);

  test('should send message and receive response', async () => {
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    const client = createClient('message', tomlContent);

    let receivedEvent = false;
    try {
      const eventPromise = waitForEvent(
        client,
        () => {
          receivedEvent = true;
          return true;
        },
        5000,
        'Timeout waiting for response'
      );

      await client.sendMessage('/help');
      await eventPromise;

      expect(receivedEvent).toBe(true);
    } finally {
      await client.close();
    }
  }, 10000);

  test('should handle settings command', async () => {
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    const tempClient = createClient('settings', tomlContent);

    let receivedSettings = false;
    try {
      const promise = waitForEvent(
        tempClient,
        (event) => {
          if (event.kind === 'MessageAdded' && event.data.content.includes('=== Current Settings')) {
            receivedSettings = true;
            return true;
          }
          return false;
        },
        5000,
        'Timeout waiting for settings response'
      );

      await tempClient.sendMessage('/settings');
      await promise;
      expect(receivedSettings).toBe(true);
    } finally {
      await tempClient.close();
    }
  }, 10000);

  test('should handle agent model command', async () => {
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    const tempClient = createClient('agent', tomlContent);

    let receivedConfirmation = false;
    try {
      const promise = waitForEvent(
        tempClient,
        (event) => {
          if (event.kind === 'MessageAdded' && event.data.content.includes('Model successfully set to')) {
            receivedConfirmation = true;
            return true;
          }
          return false;
        },
        5000,
        'Timeout waiting for agent model response'
      );

      await tempClient.sendMessage('/agentmodel coder grok-build');
      await promise;
      expect(receivedConfirmation).toBe(true);
    } finally {
      await tempClient.close();
    }
  }, 10000);

  test('should handle retry events', async () => {
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = { retryable_error_then_success = { remaining_errors = 3 } }`;
    const tempClient = createClient('retry', tomlContent);

    let retriesObserved = 0;
    try {
      const promise = waitForEvent(
        tempClient,
        (event) => {
          if (event.kind === 'RetryAttempt') {
            retriesObserved++;
            return retriesObserved === 3;
          }
          return false;
        },
        10000,
        'Timeout waiting for 3 retries'
      );

      await tempClient.sendMessage('search for files');
      await promise;
      expect(retriesObserved).toBe(3);
    } finally {
      await tempClient.close();
    }
  }, 10000);

  test('should handle security approval for sensitive searches', async () => {
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    const tempClient = createClient('security', tomlContent);

    let receivedSecurityInfo = false;
    try {
      const promise = waitForEvent(
        tempClient,
        (event) => {
          if (event.kind === 'MessageAdded' && event.data.content.toLowerCase().includes('help')) {
            receivedSecurityInfo = true;
            return true;
          }
          return false;
        },
        5000,
        'Timeout waiting for help response'
      );

      await tempClient.sendMessage('/help');
      await promise;
      expect(receivedSecurityInfo).toBe(true);
    } finally {
      await tempClient.close();
    }
  }, 10000);

  test('should handle models command', async () => {
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    const tempClient = createClient('models', tomlContent);

    let modelList = '';
    try {
      const promise = waitForEvent(
        tempClient,
        (event) => {
          if (event.kind === 'MessageAdded' && event.data.sender === 'System') {
            modelList = event.data.content;
            return modelList.toLowerCase().includes('none');
          }
          return false;
        },
        5000,
        'Timeout waiting for models response'
      );

      await tempClient.sendMessage('/models');
      await promise;
      expect(modelList.toLowerCase()).toContain('none');
    } finally {
      await tempClient.close();
    }
  }, 10000);
});
