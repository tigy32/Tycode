import { spawn, type ChildProcess } from 'child_process';
import { platform, arch } from 'os';
import { join } from 'path';
import { existsSync } from 'fs';
import { ChatEvent, ChatActorMessage } from './types';

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
    this.subprocess = spawn(binaryPath, args, { stdio: ['pipe', 'pipe', 'pipe'] });
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
        const event: ChatEvent = JSON.parse(line);
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