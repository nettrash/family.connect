import { test, expect } from '@playwright/test';
import { writeFile } from 'node:fs/promises';

// Real REST/WebSocket traffic with disposable local fixtures. No camera is opened.
test('reaction_search_two_accounts_e2e', async ({ page, baseURL }, testInfo) => {
    const outputDir = testInfo.outputDir;
    const assert = (value, message) => expect(value, message).toBe(true);
    const origin = new URL(baseURL).origin;
    const suffix = Date.now().toString(36);
    const password = `ReactionE2E-${suffix}!`;
    const headers = token => ({ Authorization: `Bearer ${token}`, 'X-FC-Families': '1' });
    async function request(method, path, token, data) {
        const response = await page.request[method](`${origin}/api/v1${path}`, {
            headers: token ? headers(token) : {}, ...(data === undefined ? {} : { data }),
        });
        assert(response.ok(), `${method} ${path}: ${response.status()}`);
        return response.status() === 204 ? null : response.json();
    }
    const alice = await request('post', '/auth/register', null, {
        username: `reaction_${suffix}_a`, display_name: 'Reaction Alice', password,
    });
    const bob = await request('post', '/auth/register', null, {
        username: `reaction_${suffix}_b`, display_name: 'Reaction Bob', password,
    });
    const familyName = `Reaction E2E ${suffix}`;
    const { family } = await request('post', '/families', alice.token, { name: familyName });
    const joined = await request('post', '/families/join', bob.token, { invite_code: family.invite_code });
    if (joined.status === 'pending') {
        const { requests } = await request('get', `/families/${family.id}/join-requests`, alice.token);
        assert(requests.length === 1, 'one join request');
        await request('post', `/families/${family.id}/join-requests/${requests[0].id}/approve`, alice.token);
    }
    const message = `Reaction search evidence ${suffix}`;
    const chat = target => target.getByRole('button').filter({ hasText: familyName }).first();
    const article = target => target.locator('article').filter({ hasText: message });
    const picker = target => target.locator('.emoji-picker');
    const search = target => picker(target).locator('input[type=search]');
    async function openPicker(target) {
        await article(target).locator('button.more').click();
        await article(target).locator('[role=menuitem].link').click();
        await picker(target).waitFor({ state: 'visible' });
        assert(await search(target).isVisible(), 'search is visible');
    }
    async function signIn(target, token) {
        await target.addInitScript(({ origin, token }) => {
            if (location.origin === origin) sessionStorage.setItem('fc.token', token);
        }, { origin, token });
        await target.goto(origin);
        await chat(target).click();
    }
    await signIn(page, alice.token);
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill(message);
    await page.getByRole('textbox', { name: 'Message', exact: true }).press('Enter');
    await article(page).waitFor();
    await page.waitForFunction(text => {
        const article = [...document.querySelectorAll('article')].find(node => node.textContent.includes(text));
        return article && !article.classList.contains('is-pending');
    }, message);
    const second = await page.context().newPage();
    const frameTypes = [];
    second.on('websocket', socket => {
        if (new URL(socket.url()).pathname !== '/api/v1/ws') return;
        socket.on('framereceived', ({ payload }) => {
            try { frameTypes.push(JSON.parse(payload.toString()).type); } catch {}
        });
    });
    await signIn(second, bob.token);
    await article(second).waitFor();
    await page.bringToFront();
    await openPicker(page);
    assert(await search(page).evaluate(node => node === document.activeElement), 'search receives focus');
    const catalogueCount = await picker(page).locator('.emoji-grid button').count();
    assert(catalogueCount >= 769, 'full catalogue initially visible');
    await page.screenshot({ path: `${outputDir}/01-catalogue-desktop.png` });
    await search(page).fill('no-such-emoji-8372');
    await picker(page).locator('[role=status]').waitFor();
    assert(await picker(page).locator('.emoji-grid button').count() === 0, 'no-match gives no choices');
    await page.screenshot({ path: `${outputDir}/02-no-results.png` });
    await picker(page).locator('.emoji-clear').click();
    assert(await search(page).inputValue() === '', 'clear resets query');
    assert(await picker(page).locator('.emoji-grid button').count() === catalogueCount, 'clear restores catalogue');
    await search(page).fill('fire');
    await picker(page).locator('nav [aria-label=Hearts]').click();
    assert(await search(page).inputValue() === '', 'category shortcut clears query');
    await page.waitForFunction(() => document.querySelector('.emoji-content').scrollTop > 0);
    const categoryVisible = await picker(page).evaluate(node => {
        const area = node.querySelector('.emoji-content').getBoundingClientRect();
        const section = node.querySelector('section[aria-label=Hearts]').getBoundingClientRect();
        return section.top >= area.top - 8 && section.top < area.bottom;
    });
    assert(categoryVisible, 'category navigation scrolls within catalogue');
    const searchRequests = [];
    const observeSearch = req => { if (new URL(req.url()).pathname.startsWith('/api/')) searchRequests.push(req.url()); };
    page.on('request', observeSearch);
    await search(page).fill('  :HEART:  ');
    await picker(page).locator('.emoji-content [data-emoji="❤️"]').waitFor();
    const found = await picker(page).locator('.emoji-content [data-emoji]').evaluateAll(nodes => nodes.map(node => node.dataset.emoji));
    assert(found[0] === '❤️', 'exact alias ranks first and preserves variation selector');
    assert(new Set(found).size === found.length, 'results are deduplicated');
    assert(searchRequests.length === 0, 'search stays on the client');
    page.off('request', observeSearch);
    await page.screenshot({ path: `${outputDir}/03-search-heart.png` });
    await picker(page).locator('.emoji-content [data-emoji="❤️"]').click();
    await article(second).locator('.chips .chip').filter({ hasText: '❤️' }).waitFor();
    await second.screenshot({ path: `${outputDir}/04-reaction-received.png` });
    await openPicker(second);
    assert(await picker(second).locator('[data-emoji-section=recent]').count() === 0, 'Bob does not see Alice recents');
    await search(second).press('Escape');
    await picker(second).waitFor({ state: 'detached' });
    await openPicker(page);
    await search(page).fill('fire');
    await picker(page).locator('.emoji-content [data-emoji="🔥"]').click();
    await article(second).locator('.chips .chip').filter({ hasText: '🔥' }).waitFor();
    assert(await article(second).locator('.chips .chip').filter({ hasText: '❤️' }).count() === 0, 'reaction was replaced');
    await openPicker(page);
    const recents = await picker(page).locator('[data-emoji-section=recent] button').evaluateAll(nodes => nodes.map(node => node.dataset.emoji));
    expect(recents, 'recents retain order without duplicates').toEqual(['🔥', '❤️']);
    await page.screenshot({ path: `${outputDir}/05-recents.png` });
    await search(page).fill('fire');
    await picker(page).locator('.emoji-content [data-emoji="🔥"]').click();
    await article(second).locator('.chips').waitFor({ state: 'detached' });
    assert(frameTypes.filter(type => type === 'reaction').length >= 3, 'recipient received set/replace/remove events');

    // Nine language contexts exercise the real browser language selection,
    // localised labels and query, rather than setting the Rust global directly.
    const locales = [
        ['en', 'fire', 'fire'], ['de', 'Feuer', 'Feuer'], ['es', 'fuego', 'fuego'], ['fr', 'feu', 'feu'],
        ['ja', '火', '火'], ['ru', 'огонь', 'огонь'], ['sr', 'ватра', 'ватра'], ['sr-Latn', 'vatra', 'vatra'], ['zh-Hans', '火', '火焰'],
    ];
    const languageResults = [];
    for (const [locale, query, expectedLabel] of locales) {
        const context = await page.context().browser().newContext({ locale });
        try {
            const local = await context.newPage();
            await signIn(local, alice.token);
            await openPicker(local);
            await search(local).fill(query);
            const fire = picker(local).locator('.emoji-content [data-emoji="🔥"]');
            await fire.waitFor();
            await expect(fire, `${locale} accessible name is localised`).toHaveAttribute('aria-label', expectedLabel);
            const label = await fire.getAttribute('aria-label');
            languageResults.push({ locale, query, label });
            if (locale === 'ru') await local.screenshot({ path: `${outputDir}/06-search-russian.png` });
        } finally { await context.close(); }
    }
    await page.bringToFront();
    await openPicker(page);
    await search(page).fill('fire');
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.screenshot({ path: `${outputDir}/07-search-dark.png` });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: `${outputDir}/08-search-mobile.png` });
    // A reduced visual viewport is a layout check, not a claim that a real
    // iOS/Android software keyboard was opened by headless Chrome.
    await page.setViewportSize({ width: 390, height: 430 });
    const layout = await picker(page).evaluate(node => {
        const dialog = node.getBoundingClientRect();
        const input = node.querySelector('input').getBoundingClientRect();
        const result = node.querySelector('.emoji-content button').getBoundingClientRect();
        const navigation = node.querySelector('nav').getBoundingClientRect();
        const cancel = node.querySelector('.dialog-actions button').getBoundingClientRect();
        return { dialog: { top: dialog.top, bottom: dialog.bottom, width: dialog.width }, inputBottom: input.bottom, resultBottom: result.bottom, navigationBottom: navigation.bottom, cancelBottom: cancel.bottom, viewportHeight: innerHeight, pageWidth: document.documentElement.scrollWidth };
    });
    assert(layout.dialog.bottom <= layout.viewportHeight + 1 && layout.dialog.top >= 0, 'picker fits short mobile viewport');
    assert(layout.inputBottom < layout.viewportHeight && layout.resultBottom < layout.viewportHeight, 'search and result stay visible');
    assert(layout.pageWidth <= 390, 'no horizontal page overflow');
    assert(layout.navigationBottom <= layout.dialog.bottom && layout.cancelBottom <= layout.dialog.bottom, 'category navigation and Cancel are not clipped');
    await page.screenshot({ path: `${outputDir}/09-short-mobile-viewport.png` });
    await search(page).press('Escape');
    await picker(page).waitFor({ state: 'detached' });
    await page.waitForFunction(text => {
        const article = [...document.querySelectorAll('article')].find(node => node.textContent.includes(text));
        return article?.querySelector('button.more') === document.activeElement;
    }, message);
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.emulateMedia({ colorScheme: 'light' });
    await openPicker(page);
    const restoredRecents = await picker(page).locator('[data-emoji-section=recent] button').evaluateAll(nodes => nodes.map(node => node.dataset.emoji));
    expect(restoredRecents, 'reopening restores recent choices').toEqual(['🔥', '❤️']);
    await search(page).press('Escape');
    await page.getByRole('button', { name: 'Log out', exact: true }).click();
    await page.getByRole('dialog', { name: 'Log out?' }).getByRole('button', { name: 'Log Out', exact: true }).click();
    await page.getByRole('group', { name: 'Log in or register' }).waitFor();
    const saved = await page.evaluate(() => Object.keys(localStorage).filter(key => key.startsWith('fc.reaction.recents.')));
    assert(saved.length === 0, 'logout clears recent reaction data');
    await second.close();
    const summary = {
        test: 'reaction_search_two_accounts_e2e', status: 'passed',
        familyId: family.id, catalogueCount, languageResults, recents,
        reactionEvents: frameTypes.filter(type => type === 'reaction').length,
        searchApiRequests: searchRequests.length, mobileLayout: layout,
        assertions: ['search focus', 'empty state', 'clear', 'category navigation', 'alias ranking', 'Unicode bytes', 'deduplication', 'client-only search', 'set/replace/remove delivered over WebSocket', 'account isolation', 'nine browser locales', 'dark/mobile/short viewport', 'recent order and reopening', 'Escape', 'logout clears data'],
    };
    const summaryPath = testInfo.outputPath('summary.json');
    await writeFile(summaryPath, JSON.stringify(summary, null, 2) + '\n');
    await testInfo.attach('reaction-search-summary', { path: summaryPath, contentType: 'application/json' });
    for (const name of ['03-search-heart.png', '08-search-mobile.png', '09-short-mobile-viewport.png']) {
        await testInfo.attach(name, { path: testInfo.outputPath(name), contentType: 'image/png' });
    }
});
