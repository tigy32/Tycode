import { spawn, type ChildProcess } from 'child_process';
import { platform, arch } from 'os';
import { join } from 'path';
import { existsSync } from 'fs';
import { ChatEvent, ChatActorMessage, ImageData, SessionMetadata, SessionData, ChatEventTag, ModuleSchemaInfo } from './types';

class ChatActorClient {
  private subprocess: ChildProcess | null = null;
  private rl: any = null;
  private eventQueue: ChatEvent[] = [];
  private eventResolvers: Array<(event: ChatEvent) => void> = [];
  private pendingEventWaiters: Map<ChatEventTag, Array<{resolve: (value: any) => void, reject: (reason: any) => void, timeout: NodeJS.Timeout}>> = new Map();

  constructor(workspaceRoots: string[], settingsPath?: string) {
    const binaryPath = this.getBinaryPath();
    if (!existsSync(binaryPath)) {
      throw new Error(`Binary not found at ${binaryPath}. Run npm run build:rust-binary to build it.`);
    }
    const args = [
      '--workspace-roots',
      JSON.stringify(workspaceRoots),
    ];
    if (settingsPath) {
      args.push('--settings-path', settingsPath);
    }
    this.subprocess = spawn(binaryPath, args, { stdio: ['pipe', 'pipe', 'pipe'], env: process.env });
    this.subprocess.on('error', (error) => {
      console.error('Subprocess error:', error);
    });
    this.subprocess.on('exit', (code) => {
      console.log('Subprocess exited with code', code);
    });

    // Capture and log stderr output
    if (this.subprocess.stderr) {
      this.subprocess.stderr.on('data', (data: Buffer) => {
        console.log('Subprocess stderr:', data.toString());
      });
    }

    // Set up event listening immediately after spawning, using queue for async consumption
    const readline = require('readline');
    this.rl = readline.createInterface({
      input: this.subprocess.stdout!,
      terminal: false
    });
    this.rl.on('line', (line: string) => {
      try {
        const event = JSON.parse(line) as ChatEvent;
        
        // Priority routing: check if there's a waiter for this specific event type
        const waiters = this.pendingEventWaiters.get(event.kind);
        if (waiters && waiters.length > 0) {
          const waiter = waiters.shift()!;
          clearTimeout(waiter.timeout);
          if (event.kind === 'Error') {
            waiter.reject(new Error(event.data || 'Unknown error'));
          } else if ('data' in event) {
            waiter.resolve(event.data);
          } else {
            waiter.resolve(undefined);
          }
          return;
        }
        
        // General queue for events without specific waiters
        if (this.eventResolvers.length > 0) {
          const resolve = this.eventResolvers.shift()!;
          resolve(event);
        } else {
          this.eventQueue.push(event);
        }
      } catch (e) {
        console.error('Invalid JSON in event stream:', line);
      }
    });
  }

  private getBinaryPath(): string {
    const plat = platform();
    const architecture = arch();
    const binaryName = plat === 'win32' ? 'tycode-subprocess.exe' : 'tycode-subprocess';
    return join(__dirname, '../bin', `${plat}-${architecture}`, binaryName);
  }

  sendMessage(message: string): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { UserInput: message };
    const data = JSON.stringify(msg) + '\n';
    return new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  sendMessageWithImages(text: string, images: ImageData[]): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { UserInputWithImages: { text, images } };
    const data = JSON.stringify(msg) + '\n';
    return new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  changeProvider(provider: string): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { ChangeProvider: provider };
    const data = JSON.stringify(msg) + '\n';
    return new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  async getSettings(): Promise<any> {
    if (!this.subprocess) throw new Error('No subprocess');
    
    // Register waiter BEFORE sending message to avoid race condition
    const resultPromise = this.waitForEvent<any>('Settings');
    
    const msg: ChatActorMessage = 'GetSettings';
    const data = JSON.stringify(msg) + '\n';
    await new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
    return resultPromise;
  }

  saveSettings(settings: any, persist: boolean = true): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { SaveSettings: { settings, persist } };
    const data = JSON.stringify(msg) + '\n';
    return new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  switchProfile(profileName: string): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { SwitchProfile: { profile_name: profileName } };
    const data = JSON.stringify(msg) + '\n';
    return new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  saveProfileAs(profileName: string): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { SaveProfile: { profile_name: profileName } };
    const data = JSON.stringify(msg) + '\n';
    return new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  cancel(): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const data = 'CANCEL\n';
    return new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  private async getNextEvent(): Promise<ChatEvent> {
    if (this.eventQueue.length > 0) {
      return this.eventQueue.shift()!;
    }
    return new Promise<ChatEvent>((resolve) => {
      this.eventResolvers.push(resolve);
    });
  }

  private async processEventForWait<T>(
    eventKind: ChatEventTag,
    timeout: NodeJS.Timeout,
    resolve: (value: T) => void,
    reject: (reason: any) => void
  ): Promise<boolean> {
    const event = await this.getNextEvent();

    if (event.kind === 'Error') {
      clearTimeout(timeout);
      reject(new Error(event.data || 'Unknown error'));
      return true;
    }

    if (event.kind !== eventKind) return false;

    clearTimeout(timeout);
    if (!('data' in event)) {
      reject(new Error(`Event ${eventKind} does not have data`));
      return true;
    }
    resolve(event.data as T);
    return true;
  }

  private waitForEvent<T>(eventKind: ChatEventTag, timeoutMs: number = 10000): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timeout = setTimeout(() => {
        // Remove from pending waiters on timeout
        const waiters = this.pendingEventWaiters.get(eventKind);
        if (waiters) {
          const idx = waiters.findIndex(w => w.resolve === resolve);
          if (idx >= 0) waiters.splice(idx, 1);
        }
        reject(new Error(`Timeout waiting for ${eventKind} event`));
      }, timeoutMs);

      // Register as a waiter for this specific event type
      if (!this.pendingEventWaiters.has(eventKind)) {
        this.pendingEventWaiters.set(eventKind, []);
      }
      this.pendingEventWaiters.get(eventKind)!.push({ resolve, reject, timeout });
    });
  }

  async listProfiles(): Promise<string[]> {
    if (!this.subprocess) throw new Error('No subprocess');
    
    // Register waiter BEFORE sending message to avoid race condition
    const resultPromise = this.waitForEvent<{ profiles: string[] }>('ProfilesList');
    
    const msg: ChatActorMessage = 'ListProfiles';
    const data = JSON.stringify(msg) + '\n';
    await new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
    const result = await resultPromise;
    return result.profiles;
  }

  async listSessions(): Promise<SessionMetadata[]> {
    if (!this.subprocess) throw new Error('No subprocess');
    
    // Register waiter BEFORE sending message to avoid race condition
    const resultPromise = this.waitForEvent<{ sessions: SessionMetadata[] }>('SessionsList');
    
    const msg: ChatActorMessage = 'ListSessions';
    const data = JSON.stringify(msg) + '\n';
    await new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
    const result = await resultPromise;
    return result.sessions;
  }

  async resumeSession(sessionId: string): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { ResumeSession: { session_id: sessionId } };
    const data = JSON.stringify(msg) + '\n';
    await new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
  }

  async getModuleSchemas(): Promise<ModuleSchemaInfo[]> {
    if (!this.subprocess) throw new Error('No subprocess');
    
    const resultPromise = this.waitForEvent<{ schemas: ModuleSchemaInfo[] }>('ModuleSchemas');
    
    const msg: ChatActorMessage = 'GetModuleSchemas';
    const data = JSON.stringify(msg) + '\n';
    await new Promise<void>((resolve, reject) => {
      const written = this.subprocess!.stdin!.write(data);
      if (written) {
        resolve();
      } else {
        this.subprocess!.stdin!.once('drain', resolve);
      }
    });
    const result = await resultPromise;
    return result.schemas;
  }

  async *events(): AsyncGenerator<ChatEvent, void, unknown> {
    while (true) {
      yield new Promise<ChatEvent>((resolve) => {
        if (this.eventQueue.length > 0) {
          resolve(this.eventQueue.shift()!);
        } else {
          this.eventResolvers.push(resolve);
        }
      });
    }
  }

  async close(): Promise<void> {
    if (this.rl) {
      this.rl.close();
      this.rl = null;
    }
    if (this.subprocess) {
      this.subprocess.kill();
      // Await subprocess to fully exit to prevent lingering stdout/stderr
      await new Promise<void>((resolve) => {
        this.subprocess!.on('exit', () => resolve());
      });
      this.subprocess = null;
    }
  }
}

export { ChatActorClient };
