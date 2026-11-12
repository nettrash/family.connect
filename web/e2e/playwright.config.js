import { defineConfig } from '@playwright/test';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const baseURL = process.env.FC_E2E_URL ?? 'http://127.0.0.1:58081';
const target = new URL(baseURL);
// These tests create accounts and a family. Never run their fixtures on a live server.
if (!['127.0.0.1', 'localhost', '[::1]'].includes(target.hostname)) {
    throw new Error('FC_E2E_URL must point to a local test server');
}
const root = process.env.FC_E2E_OUTPUT ?? fileURLToPath(new URL('../../output/playwright/reaction-search', import.meta.url));

export default defineConfig({
    testDir: '.',
    testMatch: 'reaction-search.spec.js',
    outputDir: path.join(root, 'results'),
    timeout: 120_000,
    globalTimeout: 180_000,
    fullyParallel: false,
    workers: 1,
    retries: 0,
    forbidOnly: true,
    failOnFlakyTests: true,
    reporter: [
        ['list'],
        ['json', { outputFile: path.join(root, 'report.json') }],
        ['junit', { outputFile: path.join(root, 'junit.xml') }],
        ['html', { outputFolder: path.join(root, 'html'), open: 'never' }],
    ],
    use: {
        baseURL,
        browserName: 'chromium',
        locale: 'en',
        viewport: { width: 1280, height: 720 },
        trace: 'retain-on-failure',
        screenshot: 'only-on-failure',
        video: 'retain-on-failure',
    },
});
