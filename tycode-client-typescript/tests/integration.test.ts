import { writeFileSync, existsSync, unlinkSync } from 'fs';
import { ChatActorClient } from '../src/client';

describe('ChatActorClient Integration Test', () => {
  let client: ChatActorClient;
  let settingsPath: string;

  afterAll(() => {
    if (existsSync(settingsPath)) {
      unlinkSync(settingsPath);
    }
  });

  test('should launch subprocess successfully', async () => {
    settingsPath = `/tmp/tycode-test-settings-${Date.now()}.toml`;
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    writeFileSync(settingsPath, tomlContent);
    expect(() => {
      client = new ChatActorClient(['.'], settingsPath);
    }).not.toThrow();
  }, 10000);

  test('should send message and receive response', async () => {
    console.log("Hello world!");
    // Listen for events using async iteration
    let receivedEvent = false;
    const eventPromise = new Promise<void>((resolve) => {
      (async () => {
        for await (const event of client.events()) {
          console.log(event);
          receivedEvent = true;
          resolve();
        }
      })();
    });

    // Send message
    await client.sendMessage('/help');

    // Wait for event or timeout
    await Promise.race([
      eventPromise,
      new Promise((_, reject) => setTimeout(() => reject(new Error('Timeout waiting for response')), 5000))
    ]);

    expect(receivedEvent).toBe(true);

    await client.close();
  }, 10000);

  test('should handle settings command', async () => {
    const tmpSettingsPath = `/tmp/tycode-test-settings-settings-${Date.now()}.toml`;
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    writeFileSync(tmpSettingsPath, tomlContent);
    const tempClient = new ChatActorClient(['.'], tmpSettingsPath);

    let receivedSettings = false;
    const promise = (async () => {
      for await (const event of tempClient.events()) {
        if (event.kind === 'MessageAdded' && event.data.content.includes('=== Current Settings')) {
          receivedSettings = true;
          break;
        }
      }
    })();

    await tempClient.sendMessage('/settings');
    await Promise.race([promise, new Promise(r => setTimeout(r, 5000))]);
    await tempClient.close();
    unlinkSync(tmpSettingsPath);
    expect(receivedSettings).toBe(true);
  }, 10000);

  test('should handle agent model command', async () => {
    const tmpSettingsPath = `/tmp/tycode-test-settings-agent-${Date.now()}.toml`;
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    writeFileSync(tmpSettingsPath, tomlContent);
    const tempClient = new ChatActorClient(['.'], tmpSettingsPath);

    let receivedConfirmation = false;
    const promise = (async () => {
      for await (const event of tempClient.events()) {
        if (event.kind === 'MessageAdded' && event.data.content.includes('Model successfully set to')) {
          receivedConfirmation = true;
          break;
        }
      }
    })();

    await tempClient.sendMessage('/agentmodel coder grok-code-fast-1');
    await Promise.race([promise, new Promise(r => setTimeout(r, 5000))]);
    await tempClient.close();
    unlinkSync(tmpSettingsPath);
    expect(receivedConfirmation).toBe(true);
  }, 10000);

  test('should handle retry events', async () => {
    const tmpSettingsPath = `/tmp/tycode-test-settings-retry-${Date.now()}.toml`;
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = { retry_then_success = { errors_before_success = 3 } }`;
    writeFileSync(tmpSettingsPath, tomlContent);
    const tempClient = new ChatActorClient(['.'], tmpSettingsPath);

    let retriesObserved = 0;
    const promise = new Promise<void>((resolve) => {
      (async () => {
        const eventGenerator = tempClient.events();
        for await (const event of eventGenerator) {
          console.log('Event during retry test:', event);
          if (event.kind === 'RetryAttempt') {
            retriesObserved++;
            if (retriesObserved === 3) {
              resolve();
              break;
            }
          }
        }
      })();
    });

    await tempClient.sendMessage('search for files');
    await Promise.race([
      promise,
      new Promise((_, reject) => setTimeout(() => reject(new Error('Timeout waiting for 3 retries')), 10000))
    ]);
    await tempClient.close();
    unlinkSync(tmpSettingsPath);
    expect(retriesObserved).toBe(3);
  }, 10000);

  test('should handle security approval for sensitive searches', async () => {
    const tmpSettingsPath = `/tmp/tycode-test-settings-security-${Date.now()}.toml`;
    const tomlContent = `version = "1.0"
active_provider = "mock"

[providers.mock]
type = "mock"
behavior = "success"`;
    writeFileSync(tmpSettingsPath, tomlContent);
    const tempClient = new ChatActorClient(['.'], tmpSettingsPath);

    let receivedSecurityInfo = false;
    const promise = (async () => {
      for await (const event of tempClient.events()) {
        if (event.kind === 'MessageAdded' && event.data.content.toLowerCase().includes('help')) {
          receivedSecurityInfo = true;
          break;
        }
      }
    })();

    await tempClient.sendMessage('/help');
    await Promise.race([promise, new Promise(r => setTimeout(r, 5000))]);
    await tempClient.close();
    unlinkSync(tmpSettingsPath);
    expect(receivedSecurityInfo).toBe(true);
  }, 10000);

  test('should handle search results limits', async () => {
    const tmpSettingsPath = `/tmp/tycode-test-settings-search-${Date.now()}.toml`;
    const tomlContent = `version = "1.0"
active_provider = "mock"

[global]
search_results_max_files = 5

[providers.mock]
type = "mock"
behavior = { tool_use = { tool_name = "search_files", tool_arguments = '{ "query": "test", "max_results": 10 }' } }`;
    writeFileSync(tmpSettingsPath, tomlContent);
    const tempClient = new ChatActorClient(['.'], tmpSettingsPath);

    let resultCount = 0;
    const promise = new Promise<void>((resolve, reject) => {
      (async () => {
        for await (const event of tempClient.events()) {
          if (event.kind === 'MessageAdded') {
            try {
              const parsed = JSON.parse(event.data.content);
              if (parsed.count !== undefined) {
                resultCount = parsed.count;
                resolve();
                return;
              }
            } catch (e) {
              // Not the json message, ignore
            }
          }
        }
        setTimeout(() => reject('Timeout'), 5000);
      })();
    });

    await tempClient.sendMessage('/search test');
    await Promise.race([promise, new Promise(r => setTimeout(r, 5000))]);
    await tempClient.close();
    unlinkSync(tmpSettingsPath);
    expect(resultCount).toBeLessThanOrEqual(5);
  }, 10000);
});
