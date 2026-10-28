import { test, expect, Page } from '@playwright/test';

const BASE = process.env.PLAYWRIGHT_BASE_URL ?? 'http://localhost:5199';

/** Pick a player on the landing screen. */
async function choosePlayer(page: Page, name: 'Alice' | 'Bob' | 'Carol') {
  await page.goto(BASE);
  await page.evaluate(() => localStorage.removeItem('cityoftowers.player'));
  await page.reload();
  await expect(page.getByText('Choose your city')).toBeVisible();
  const card = page.locator('.panel', { hasText: name }).first();
  await card.getByRole('button', { name: 'Play' }).click();
  await expect(page.getByRole('button', { name: 'New Game' })).toBeVisible();
}

/** Create a game and enter the adventure map. */
async function createGame(page: Page, name = 'E2E Game') {
  await page.getByRole('button', { name: 'New Game' }).click();
  await page.getByLabel('Name').fill(name);
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.locator('canvas').first()).toBeVisible({ timeout: 20_000 });
}

test.describe('City of Towers', () => {
  test('S1 landing shows three players', async ({ page }) => {
    await page.goto(BASE);
    await page.evaluate(() => localStorage.removeItem('cityoftowers.player'));
    await page.reload();

    await expect(page.getByText('CITY OF TOWERS')).toBeVisible();
    await expect(page.getByText('Choose your city')).toBeVisible();
    await expect(page.getByText('Alice')).toBeVisible();
    await expect(page.getByText('Bob')).toBeVisible();
    await expect(page.getByText('Carol')).toBeVisible();
    await expect(page.getByText('Castle')).toBeVisible();
    await expect(page.getByText('Rampart')).toBeVisible();
    await expect(page.getByText('Necropolis')).toBeVisible();
  });

  test('S2 choosing a player persists across reload', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await page.reload();
    await expect(page.getByText('Playing as: Alice')).toBeVisible();
  });

  test('S3 adventure map uses HoMM3 composition', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Map Test');

    const canvas = page.locator('canvas').first();
    const box = await canvas.boundingBox();
    expect(box).not.toBeNull();
    expect(Math.round(box!.width)).toBe(594);
    expect(Math.round(box!.height)).toBe(546);

    for (const label of ['Hero', 'Spells', 'Town', 'End Turn', 'Obelisk', 'Menu', 'Social', 'Projects']) {
      await expect(page.getByRole('button', { name: label, exact: true })).toBeVisible();
    }
  });

  test('S4 hero status window shows army and movement', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Hero Test');

    await expect(page.getByText('Christian')).toBeVisible();
    await expect(page.getByText(/Lvl 1/)).toBeVisible();
    await expect(page.getByText(/Move \d+\/1500/)).toBeVisible();
  });

  test('S5 clicking the map moves the hero', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Move Test');

    const readHero = () => page.evaluate(async () => {
      const r = await fetch('/api/games', { headers: { 'X-Player-Id': 'alice' } });
      const games = await r.json();
      const g = games[games.length - 1];
      const h = await fetch(`/api/games/${g.id}/heroes`, { headers: { 'X-Player-Id': 'alice' } });
      const heroes = await h.json();
      return { x: heroes[0].x, y: heroes[0].y, movement: heroes[0].movement };
    });

    const before = await readHero();
    const canvas = page.locator('canvas').first();
    const box = (await canvas.boundingBox())!;
    await page.mouse.click(box.x + box.width / 2 + 64, box.y + box.height / 2);
    await page.waitForTimeout(2000);
    const after = await readHero();

    expect(after.movement).toBeLessThan(before.movement);
  });

  test('S6 town screen is the original HoMM3 scene', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Town Test');

    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await expect(page.getByText('Steadwick')).toBeVisible();

    // the original HoMM3 scene: a canvas of structures over the town backdrop
    await expect(page.locator('.town-scene canvas')).toBeVisible();
    expect(await page.locator('.town-scene [data-building-slot]').count()).toBeGreaterThan(0);

    // the lower panel drives the windows, as in HoMM3 — no invented text row
    await expect(page.getByRole('button', { name: 'Hall', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Fort', exact: true })).toBeVisible();
    expect(await page.locator('[data-creature]').count()).toBeGreaterThan(0);
    expect(await page.getByRole('button', { name: 'Build', exact: true }).count()).toBe(0);
  });

  test('S7 build a dwelling from the town screen', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Build Test');
    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await page.getByRole('button', { name: 'Hall', exact: true }).click();

    // the HoMM3 hall: green boxes can be built; Griffin Tower still needs Barracks (red)
    await expect(page.locator('[data-building="dwelling3"]')).toHaveAttribute('data-state', 'locked');
    await page.locator('[data-building="upg1"]').click();
    await page.locator('.modal').getByRole('button', { name: 'Build', exact: true }).click();
    await expect(page.getByText(/Built Upg\. Guardhouse/)).toBeVisible({ timeout: 10_000 });
  });

  test('S8 recruit creatures', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Recruit Test');
    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await page.locator('[data-creature]').first().click();

    // the original HoMM3 recruit screen
    await expect(page.locator('img[src*="TPRCRT"]')).toBeVisible();
    await expect(page.getByText('Recruit Pikeman')).toBeVisible();

    await page.locator('.modal').getByRole('button', { name: 'Max', exact: true }).click();
    await page.locator('.modal').getByRole('button', { name: 'Buy', exact: true }).click();
    await expect(page.getByText(/Recruited 14 Pikeman/)).toBeVisible({ timeout: 10_000 });
  });

  test('S9 hero screen shows stats and army', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Hero Screen Test');
    await page.getByRole('button', { name: 'Hero', exact: true }).click();

    for (const label of ['Attack', 'Defense', 'Power', 'Knowledge', 'Experience', 'Spell Points']) {
      await expect(page.getByText(label, { exact: true }).first()).toBeVisible();
    }
  });

  test('S9b tavern shows hero details and hires', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Tavern Test');
    await page.getByRole('button', { name: 'Town', exact: true }).click();

    // the tavern is already built in a starting town; open it from the scene
    await page.locator('.town-scene [data-building-slot="tavern"]').first().click({ force: true });

    const modal = page.locator('.modal');
    // the original HoMM3 tavern screen
    await expect(modal.locator('img[src*="tptavern"]')).toBeVisible();
    await expect(modal.getByText('Attack', { exact: true })).toBeVisible();
    await expect(modal.getByText('Defense', { exact: true })).toBeVisible();
    await expect(modal.getByText('Power', { exact: true })).toBeVisible();
    await expect(modal.getByText('Knowledge', { exact: true })).toBeVisible();
    await expect(modal.getByText('Skills', { exact: true })).toBeVisible();
    await expect(modal.getByText('2000', { exact: true })).toBeVisible();

    await modal.getByRole('button', { name: 'Hire', exact: true }).click();
    await expect(page.getByText(/^Hired /)).toBeVisible({ timeout: 10_000 });
  });

  test('S9c walking into your town opens the town screen', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Walk In Test');
    await page.waitForTimeout(2000);

    const info = await page.evaluate(async () => {
      const pid = localStorage.getItem('cityoftowers.player');
      const games = await (await fetch('/api/games', { headers: { 'X-Player-Id': pid } })).json();
      const g = games[0];
      const map = await (await fetch(`/api/games/${g.id}/map`, { headers: { 'X-Player-Id': pid } })).json();
      const heroes = await (await fetch(`/api/games/${g.id}/heroes`, { headers: { 'X-Player-Id': pid } })).json();
      const idx = map.tiles.findIndex((t: { objectKind: string | null; owner: number }) => t.objectKind === 'town' && t.owner === 0);
      return { width: map.width, town: { x: idx % map.width, y: Math.floor(idx / map.width) }, hero: { x: heroes[0].x, y: heroes[0].y } };
    });

    const canvas = page.locator('canvas[aria-label="Adventure map"]');
    const box = (await canvas.boundingBox())!;
    const TILE = 32;
    // the camera starts at (0,0), so a tile renders at (tile * TILE) from the canvas origin
    await page.mouse.click(
      box.x + info.town.x * TILE + TILE / 2,
      box.y + info.town.y * TILE + TILE / 2
    );

    await expect(page.locator('.town-scene')).toBeVisible({ timeout: 15_000 });
  });

  test('S10 obelisk reveals a fragment', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Obelisk Test');
    await page.getByRole('button', { name: 'Obelisk', exact: true }).click();

    await expect(page.getByText('THE OBELISK').first()).toBeVisible();
    await expect(page.getByText(/You have seen 0 of 10/)).toBeVisible();
    await page.getByRole('button', { name: 'Visit the Obelisk' }).click();
    await expect(page.getByText(/You have seen 1 of 10/)).toBeVisible({ timeout: 10_000 });
  });

  test('S11 social screen shows players and trade form', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Social Test');
    await page.getByRole('button', { name: 'Social', exact: true }).click();

    await expect(page.getByText('SOCIAL').first()).toBeVisible();
    await expect(page.getByText('MESSAGES').first()).toBeVisible();
    await expect(page.getByText('TRADES').first()).toBeVisible();
    await expect(page.getByText('PLAYERS').first()).toBeVisible();
  });

  test('S12 send a message to another player', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Message Test');
    await page.getByRole('button', { name: 'Social', exact: true }).click();

    await page.locator('input[placeholder="message…"]').fill('Hello from Alice');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(page.getByText('Hello from Alice')).toBeVisible({ timeout: 10_000 });
  });

  test('S13 projects screen renders', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Projects Test');
    await page.getByRole('button', { name: 'Projects', exact: true }).click();

    await expect(page.getByText('SHARED PROJECTS').first()).toBeVisible();
  });

  test('S14 tower shows the journal', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Tower Test');
    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await page.getByRole('button', { name: 'Tower', exact: true }).click();

    await expect(page.getByText('MAGIC TOWER')).toBeVisible();
  });

  test('S15 wall accepts a message', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Wall Test');
    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await page.getByRole('button', { name: 'Wall', exact: true }).click();

    await expect(page.getByText('CITY WALL')).toBeVisible();
    await page.locator('input[placeholder="leave a message…"]').fill('First post!');
    await page.getByRole('button', { name: 'Post', exact: true }).click();
    await expect(page.getByText('First post!')).toBeVisible({ timeout: 10_000 });
  });

  test('S16 two players see the same game', async ({ browser }) => {
    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const a = await ctxA.newPage();
    const b = await ctxB.newPage();

    await choosePlayer(a, 'Alice');
    await createGame(a, 'Two Player Test');

    await choosePlayer(b, 'Bob');
    await b.getByRole('button', { name: 'Enter', exact: true }).first().click();
    await expect(b.locator('canvas').first()).toBeVisible({ timeout: 20_000 });

    await ctxA.close();
    await ctxB.close();
  });
});