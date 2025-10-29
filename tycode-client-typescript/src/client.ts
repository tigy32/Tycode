import { spawn, type ChildProcess } from 'child_process';
import { platform, arch } from 'os';
import { join } from 'path';
import { existsSync } from 'fs';
import { ChatEvent, ChatActorMessage, SessionMetadata, SessionData, ChatEventTag } from './types';

class ChatActorClient {
  private subprocess: ChildProcess | null = null;
  private rl: any = null;
  private eventQueue: ChatEvent[] = [];
  private eventResolvers: Array<(event: ChatEvent) => void> = [];

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
    return join(__dirname, '../bin', `${plat}-${architecture}`, 'tycode-subprocess');
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

  getSettings(): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = 'GetSettings';
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

  saveSettings(settings: any): Promise<void> {
    if (!this.subprocess) throw new Error('No subprocess');
    const msg: ChatActorMessage = { SaveSettings: { settings } };
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
        reject(new Error(`Timeout waiting for ${eventKind} event`));
      }, timeoutMs);

      const checkEvent = async () => {
        while (true) {
          const shouldStop = await this.processEventForWait<T>(eventKind, timeout, resolve, reject);
          if (shouldStop) return;
        }
      };

      checkEvent().catch(reject);
    });
  }

  async listSessions(): Promise<SessionMetadata[]> {
    if (!this.subprocess) throw new Error('No subprocess');
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
    const result = await this.waitForEvent<{ sessions: SessionMetadata[] }>('SessionsList');
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
