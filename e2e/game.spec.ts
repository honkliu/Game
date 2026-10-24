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

  test('S6 town screen is a painted scene with tabs', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Town Test');

    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await expect(page.getByText("Alice's Keep")).toBeVisible();

    for (const label of ['Build', 'Recruit', 'Market', 'Guild', 'Tavern', 'Garrison', 'Tower', 'Wall']) {
      await expect(page.getByRole('button', { name: label, exact: true })).toBeVisible();
    }

    const imgs = page.locator('.town-scene img');
    expect(await imgs.count()).toBeGreaterThan(0);
    await expect(imgs.first()).toHaveAttribute('src', /\/towns\/\w+\.png/);
  });

  test('S7 build a dwelling from the town screen', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Build Test');
    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await page.getByRole('button', { name: 'Build', exact: true }).click();

    await expect(page.getByText('One structure per turn.')).toBeVisible();
    await page.locator('[data-building="dwelling3"]').click();
    await page.locator('.modal').getByRole('button', { name: 'Build', exact: true }).click();
    await expect(page.getByText(/Built (Dwelling 3|Griffin Tower)/)).toBeVisible({ timeout: 10_000 });
  });

  test('S8 recruit creatures', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Recruit Test');
    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await page.getByRole('button', { name: 'Recruit', exact: true }).click();

    await expect(page.getByText('Pikeman')).toBeVisible();
    await page.locator('.modal').getByRole('button', { name: '10', exact: true }).first().click();
    await expect(page.getByText(/Recruited 10 Pikeman/)).toBeVisible({ timeout: 10_000 });
  });

  test('S9 hero screen shows stats and army', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Hero Screen Test');
    await page.getByRole('button', { name: 'Hero', exact: true }).click();

    for (const label of ['Attack', 'Defense', 'Power', 'Knowledge', 'Experience', 'Spell Points']) {
      await expect(page.getByText(label, { exact: true }).first()).toBeVisible();
    }
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