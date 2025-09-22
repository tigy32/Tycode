import * as path from 'path';
import { glob } from 'glob';

export function run(): Promise<void> {
    // Mocha exports itself as default in CommonJS
    const Mocha = require('mocha');
    const mocha = new Mocha({
        ui: 'tdd',
        color: true,
        timeout: 10000
    });

    const testsRoot = path.resolve(__dirname, '..');

    return new Promise(async (resolve, reject) => {
        try {
            // Exclude e2e tests from Mocha - they use Playwright
            const files = await glob('**/**.test.js', { 
                cwd: testsRoot,
                ignore: '**/e2e/**'
            });
            
            files.forEach((f: string) => mocha.addFile(path.resolve(testsRoot, f)));

            mocha.run((failures: number) => {
                if (failures > 0) {
                    reject(new Error(`${failures} tests failed.`));
                } else {
                    resolve();
                }
            });
        } catch (err) {
            console.error(err);
            reject(err);
        }
    });
}