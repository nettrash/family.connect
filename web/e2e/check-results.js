import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Read Playwright's structured report, never its human-readable console output.
export function checkResults(root) {
    const report = JSON.parse(readFileSync(path.join(root, 'report.json'), 'utf8'));
    assert.deepEqual(report.errors, [], 'runner errors');
    for (const key of ['unexpected', 'skipped', 'flaky']) assert.equal(report.stats[key], 0, key);
    assert.equal(report.stats.expected, 1, 'one completed scenario');
    const specs = suites => suites.flatMap(suite => [...(suite.specs ?? []), ...specs(suite.suites ?? [])]);
    const tests = specs(report.suites);
    assert.equal(tests.length, 1, 'one discovered scenario');
    assert.equal(tests[0].title, 'reaction_search_two_accounts_e2e');
    assert.equal(tests[0].tests.length, 1, 'one browser project');
    const results = tests[0].tests[0].results;
    assert.equal(results.length, 1, 'no retries');
    assert.equal(results[0].status, 'passed');
    assert.equal(results[0].retry, 0);
    const resultsDir = path.join(root, 'results');
    const summaries = readdirSync(resultsDir, { recursive: true }).filter(name => path.basename(name) === 'summary.json');
    assert.equal(summaries.length, 1, 'one domain evidence report');
    const summaryFile = path.join(resultsDir, summaries[0]);
    const summary = JSON.parse(readFileSync(summaryFile, 'utf8'));
    assert.equal(summary.test, 'reaction_search_two_accounts_e2e');
    assert.equal(summary.status, 'passed');
    assert.deepEqual(summary.languageResults.map(item => item.locale).sort(),
        ['en', 'de', 'es', 'fr', 'ja', 'ru', 'sr', 'sr-Latn', 'zh-Hans'].sort());
    assert.ok(summary.reactionEvents >= 3, 'set/replace/remove delivery');
    assert.equal(summary.searchApiRequests, 0, 'search stays local');
    for (const name of ['03-search-heart.png', '08-search-mobile.png', '09-short-mobile-viewport.png']) {
        assert.ok(statSync(path.join(path.dirname(summaryFile), name)).size > 0, name);
    }
    return summary;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const root = process.env.FC_E2E_OUTPUT ?? fileURLToPath(new URL('../../output/playwright/reaction-search', import.meta.url));
    checkResults(root);
    console.log('reaction_search_two_accounts_e2e: 1 passed; 9 locales; set/replace/remove delivered; screenshots present');
}
