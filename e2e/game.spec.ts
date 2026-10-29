import { test, expect, Page } from '@playwright/test';
import type { GameDetail, HeroDetail, MapData } from '../client/src/types';

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
async function createGame(page: Page, name = 'E2E Game', aiPlayers = 0) {
  await page.getByRole('button', { name: 'New Game' }).click();
  await page.getByLabel('Name').fill(name);
  await page.getByLabel('Computer opponents', { exact: true }).selectOption(String(aiPlayers));
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.locator('canvas').first()).toBeVisible({ timeout: 20_000 });
}

async function adventureFixture(page: Page, name: string, options: Record<string, unknown>) {
  const headers = { 'X-Player-Id': 'alice' };
  const games = await (await page.request.get(`${BASE}/api/games`, { headers })).json();
  const game = games.find((g: { name: string }) => g.name === name);
  const heroes = await (await page.request.get(`${BASE}/api/games/${game.id}/heroes`, { headers })).json();
  const primary = heroes.find((h: HeroDetail) => h.name === 'Christian') ?? heroes[0];
  const response = await page.request.post(`${BASE}/api/heroes/${primary.id}/_test/adventure`, { headers, data: options });
  expect(response.ok()).toBeTruthy();
  const hero = await response.json() as HeroDetail;
  await page.reload();
  await expect(page.getByRole('button', { name: 'Christian', exact: true })).toBeVisible();
  return { hero, gameId: game.id as string };
}

async function readHero(page: Page, id: string): Promise<HeroDetail> {
  return (await page.request.get(`${BASE}/api/heroes/${id}`)).json();
}

async function tilePoint(page: Page, x: number, y: number) {
  const canvas = page.locator('canvas').first();
  const box = (await page.locator('canvas').first().boundingBox())!;
  const camX = Number(await canvas.getAttribute('data-cam-x')), camY = Number(await canvas.getAttribute('data-cam-y'));
  return { x: box.x + ((x - camX) * 32 + 16) * box.width / 594,
    y: box.y + ((y - camY) * 32 + 16) * box.height / 546 };
}

async function clickTile(page: Page, hero: HeroDetail, x: number, y: number) {
  const point = await tilePoint(page, x, y);
  await page.mouse.click(point.x, point.y);
  await expect(page.locator('canvas').first()).toHaveAttribute('data-path-end', `${x},${y}`);
  await page.mouse.click(point.x, point.y);
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
    const hero = page.getByRole('button', { name: 'Christian', exact: true });
    await expect(hero).toHaveAttribute('title', /Level 1 Knight/);
    await expect(hero.getByRole('meter', { name: 'Movement points' })).toHaveAttribute('aria-valuemax', '1560');
    await expect(hero.locator('[style*="imobil"]')).toBeVisible();
  });

  test('S5 first click previews, second click moves the hero', async ({ page }) => {
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
    await expect(canvas).not.toHaveAttribute('data-path-end', '');
    expect(await readHero()).toEqual(before);
    await page.mouse.click(box.x + box.width / 2 + 64, box.y + box.height / 2);
    await expect.poll(async () => (await readHero()).movement).toBeLessThan(before.movement);
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
    await expect(page.locator('[data-building="upg1"]')).toHaveAttribute('data-state', 'built');
    await expect(page.locator('#toast.show')).toHaveCount(0);
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
    await expect(page.locator('img[src*="TPRCRT"]')).toBeHidden();
    await expect(page.locator('[data-army="garrison"][data-slot="0"]')).toContainText('14');
    await expect(page.locator('#toast.show')).toHaveCount(0);
  });

  test('S9 hero screen shows stats and army', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Hero Screen Test');
    await page.getByRole('button', { name: 'Hero', exact: true }).click();

    for (const label of ['Attack', 'Defense', 'Power', 'Knowledge', 'Experience', 'Spell Points']) {
      await expect(page.getByText(label, { exact: true }).first()).toBeVisible();
    }
  });

  test('S9a artifacts sit in their own paper-doll slot', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Artifact Slots Test');
    // a shield picked up first used to land on the head; give a shield, a helm and a second shield
    const heroId = await page.evaluate(async () => {
      const g = (await (await fetch('/api/games', { headers: { 'X-Player-Id': 'alice' } })).json())
        .find((x: { name: string }) => x.name === 'Artifact Slots Test');
      const h = (await (await fetch(`/api/games/${g.id}/heroes`, { headers: { 'X-Player-Id': 'alice' } })).json())[0];
      await fetch(`/api/heroes/${h.id}/_test/artifacts`, { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(['artifact1', 'artifact2', 'artifact10']) });
      return h.id as string;
    });
    expect(heroId).toBeTruthy();
    await page.getByRole('button', { name: 'Hero', exact: true }).click();

    const shield = page.locator('[data-art-slot="leftHand"]');
    await expect(shield).toHaveAttribute('data-art-id', '14');
    await expect(page.locator('[data-art-slot="head"]')).toHaveAttribute('data-art-id', '21');
    await expect(page.locator('[data-backpack="0"]')).toHaveAttribute('data-art-id', '17');
    await shield.hover();
    await expect(page.getByText('Shield of the Yawning Dead (+3 Defense)').first()).toBeVisible();
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
    await expect(modal.getByText('2500', { exact: true })).toBeVisible();

    await expect(modal.getByRole('button', { name: 'Hire', exact: true })).toBeDisabled();
    await expect(modal.getByText('Send the visiting hero out before hiring another.')).toBeVisible();
    await page.evaluate(async () => {
      const headers = { 'X-Player-Id': 'alice', 'Content-Type': 'application/json' };
      const games = await (await fetch('/api/games', { headers })).json();
      const game = games.find((g: { name: string }) => g.name === 'Tavern Test');
      const heroes = await (await fetch(`/api/games/${game.id}/heroes`, { headers })).json();
      const hero = heroes[0];
      const moved = await fetch(`/api/heroes/${hero.id}/move`, {
        method: 'POST', headers, body: JSON.stringify({ path: [{ x: hero.x, y: hero.y + 1 }] })
      });
      if (!moved.ok) throw new Error(await moved.text());
    });
    await page.reload();
    await page.locator('.town-scene [data-building-slot="tavern"]').first().click({ force: true });
    await expect(modal.getByRole('button', { name: 'Hire', exact: true })).toBeEnabled();
    await modal.getByRole('button', { name: 'Hire', exact: true }).click();
    await expect(page.getByRole('button', { name: /^View / })).toBeVisible();
    await expect(modal.getByRole('button', { name: 'Hire', exact: true })).toBeDisabled();
    await expect(page.locator('#toast.show')).toHaveCount(0);
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

  test('S17 treasure offers exactly one reward and survives reload', async ({ page }) => {
    for (const choice of ['gold', 'experience']) {
      await choosePlayer(page, 'Alice');
      const name = `Treasure ${choice} Test`;
      await createGame(page, name);
      const { hero, gameId } = await adventureFixture(page, name, { treasure: true });
      const game = await (await page.request.get(`${BASE}/api/games/${gameId}`, { headers: { 'X-Player-Id': 'alice' } })).json() as GameDetail;
      const gold = game.players.find(p => p.name === 'Alice')!.resources.gold;
      await clickTile(page, hero, hero.x, hero.y + 1);
      await expect(page.getByRole('dialog', { name: 'Treasure chest' })).toBeVisible();
      await page.reload();
      const dialog = page.getByRole('dialog', { name: 'Treasure chest' });
      await expect(dialog).toBeVisible();
      await dialog.getByRole('button', { name: choice === 'gold' ? 'Take 1500 gold' : 'Gain 1000 experience' }).click();
      await dialog.getByRole('button', { name: 'OK', exact: true }).click();
      await expect(dialog).not.toBeVisible();
      const after = await readHero(page, hero.id);
      const updated = await (await page.request.get(`${BASE}/api/games/${gameId}`, { headers: { 'X-Player-Id': 'alice' } })).json() as GameDetail;
      expect(after.experience).toBe(hero.experience + (choice === 'experience' ? 1000 : 0));
      expect(updated.players.find(p => p.name === 'Alice')!.resources.gold).toBe(gold + (choice === 'gold' ? 1500 : 0));
      expect(after.pendingTreasure).toBeNull();
    }
  });

  test('S18 Fly crosses an obstacle using the actual map controls', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Flight Test');
    const { hero } = await adventureFixture(page, 'Flight Test', { spells: ['fly'], flightLane: true });
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    await page.getByRole('button', { name: 'Cast Fly', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Spellbook' })).toBeHidden();
    await expect.poll(async () => (await readHero(page, hero.id)).flying).toBe(true);
    await clickTile(page, hero, hero.x, hero.y + 2);
    await expect.poll(async () => {
      const h = await readHero(page, hero.id);
      return { x: h.x, y: h.y, movement: h.movement };
    }).toEqual({ x: hero.x, y: hero.y + 2, movement: hero.movement - 280 });
  });

  test('S19 Haste does not stack and Cure does not refill movement', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Utility Test');
    const { hero } = await adventureFixture(page, 'Utility Test', { spells: ['haste', 'cure'], movementPenalty: 300 });
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    await page.getByRole('button', { name: 'Cast Haste', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Christian', exact: true })
      .getByRole('meter', { name: 'Movement points' })).toHaveAttribute('aria-valuenow', '800');
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Cast Haste', exact: true })).toBeDisabled();
    await page.getByRole('button', { name: 'Cast Cure', exact: true }).click();
    await expect.poll(async () => {
      const h = await readHero(page, hero.id);
      return { movement: h.movement, maxMovement: h.maxMovement, penalty: h.movementPenalty, mana: h.mana };
    }).toEqual({ movement: 800, maxMovement: 1860, penalty: 0, mana: 88 });
  });

  test('S20 survey spells show hidden locations without exploring them', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Survey Test');
    const { gameId } = await adventureFixture(page, 'Survey Test', {
      spells: ['magicArrow', 'haste', 'shield', 'cure', 'fly', 'viewAir', 'viewEarth'], surveyTargets: true
    });
    for (const [spell, kind, x] of [['viewAir', 'artifact', 0], ['viewEarth', 'resource', 1]] as const) {
      await page.getByRole('button', { name: 'Spells', exact: true }).click();
      await page.locator(`button[data-spell="${spell}"]`).click();
      const world = page.getByRole('dialog', { name: 'World view' });
      await expect(world).toBeVisible();
      await expect(world.locator(`[data-location-kind="${kind}"][data-x="${x}"][data-y="0"]`)).toBeVisible();
      await world.getByRole('button', { name: 'Close', exact: true }).click();
    }
    const map = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json();
    expect(map.tiles[0].fog).toBe(0);
    expect(map.tiles[1].fog).toBe(0);
  });

  for (const resource of ['wood', 'ore', 'mercury', 'sulfur', 'crystal', 'gems', 'gold']) {
    test(`S21 ${resource} uses a visit cursor, previews without collecting, and reports in the info bar`, async ({ page }, testInfo) => {
      await choosePlayer(page, 'Alice');
      const name = `Pickup ${resource}`;
      await createGame(page, name);
      const { hero, gameId } = await adventureFixture(page, name, { objectKind: 'resource', objectData: `${resource}:5` });
      const canvas = page.locator('canvas').first();
      const point = await tilePoint(page, hero.x, hero.y - 1);
      await page.mouse.move(point.x, point.y);
      await expect(canvas).toHaveAttribute('data-cursor-frame', '9');
      await expect(canvas).toHaveCSS('cursor', /data:image\/png/);
      if (resource === 'gold') {
        const matches = await canvas.evaluate(async (element) => {
          const meta = await (await fetch('/h3/def/CRADVNTR.json')).json();
          const image = new Image(); image.src = '/h3/def/CRADVNTR.png'; await image.decode();
          const actual = new Image(); actual.src = (element as HTMLCanvasElement).style.cursor.match(/url\("([^"]+)"/)![1]; await actual.decode();
          const a = document.createElement('canvas'), b = document.createElement('canvas');
          a.width = b.width = meta.w; a.height = b.height = meta.h;
          a.getContext('2d')!.drawImage(image, 9 % meta.cols * meta.w, Math.floor(9 / meta.cols) * meta.h, meta.w, meta.h, 0, 0, meta.w, meta.h);
          b.getContext('2d')!.drawImage(actual, 0, 0);
          const expected = a.getContext('2d')!.getImageData(0, 0, meta.w, meta.h).data;
          return b.getContext('2d')!.getImageData(0, 0, meta.w, meta.h).data.every((v, i) => v === expected[i]);
        });
        expect(matches).toBe(true);
      }
      await page.mouse.click(point.x, point.y);
      await expect(canvas).toHaveAttribute('data-path-end', `${hero.x},${hero.y - 1}`);
      expect((await readHero(page, hero.id)).movement).toBe(hero.movement);
      const before = await (await page.request.get(`${BASE}/api/games/${gameId}`, { headers: { 'X-Player-Id': 'alice' } })).json() as GameDetail;
      if (resource === 'gold') await page.screenshot({ path: testInfo.outputPath('visit-path-preview.png') });
      const moveResponse = page.waitForResponse(r => r.url().endsWith(`/heroes/${hero.id}/move`) && r.request().method() === 'POST');
      await page.getByRole('button', { name: 'Move Hero', exact: true }).click();
      const response = await moveResponse;
      const events = JSON.parse(response.headers()['x-move-events']);
      expect(events).toHaveLength(1);
      expect(events[0]).toMatchObject({ kind: 'resource', key: resource, amount: 5 });
      const reward = page.getByRole('status', { name: 'Adventure reward' });
      await expect(reward).toHaveAttribute('data-resource', resource);
      await expect(reward).toHaveAttribute('data-amount', '5');
      await expect(page.locator('#toast.show')).toHaveCount(0);
      const after = await (await page.request.get(`${BASE}/api/games/${gameId}`, { headers: { 'X-Player-Id': 'alice' } })).json() as GameDetail;
      expect(after.players.find(p => p.name === 'Alice')!.resources[resource as keyof GameDetail['players'][number]['resources']])
        .toBe(before.players.find(p => p.name === 'Alice')!.resources[resource as keyof GameDetail['players'][number]['resources']] + 5);
      expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y - 1, movement: hero.movement - 100 });
      if (resource === 'gold') {
        await page.screenshot({ path: testInfo.outputPath('native-resource-info.png') });
        await expect(reward).not.toBeVisible({ timeout: 5000 });
      } else {
        await reward.click();
        await expect(reward).not.toBeVisible();
      }
    });
  }

  for (const resource of ['wood', 'ore', 'mercury', 'sulfur', 'crystal', 'gems', 'gold']) {
    test(`S22 ${resource} mine visits without entering its physical body`, async ({ page }) => {
      await choosePlayer(page, 'Alice');
      const name = `Mine ${resource}`;
      await createGame(page, name);
      const { hero, gameId } = await adventureFixture(page, name, { objectKind: 'mine', objectData: resource });
      await clickTile(page, hero, hero.x, hero.y - 1);
      const dialog = page.getByRole('dialog', { name: 'Mine', exact: true });
      await expect(dialog).toBeVisible();
      await expect(dialog.getByText('per day', { exact: false })).toBeVisible();
      expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y, movement: hero.movement - 100 });
      const map = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json() as MapData;
      expect(map.tiles[(hero.y - 1) * map.width + hero.x]).toMatchObject({ objectKind: 'mine', passable: false, owner: 1 });
      await dialog.getByRole('button', { name: 'OK', exact: true }).click();
    });
  }

  for (const [spell, level] of [['magicArrow', 1], ['viewAir', 1], ['visions', 2], ['animateDead', 3]] as const) {
    test(`S23 shrine ${spell} has its body on the visit tile and never lets the hero walk through`, async ({ page }, testInfo) => {
      await choosePlayer(page, 'Alice');
      const name = `Shrine ${spell}`;
      await createGame(page, name);
      const { hero, gameId } = await adventureFixture(page, name, { objectKind: 'shrine', objectData: spell, spells: [], wisdom: 1 });
      const canvas = page.locator('canvas').first();
      const map = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json() as MapData;
      expect(map.tiles[(hero.y - 1) * map.width + hero.x]).toMatchObject({ sprite: `AVXl${level}sh0`, passable: false });
      await expect.poll(async () => canvas.evaluate(async (element, at) => {
        const image = new Image(); image.src = `/h3/def/AVXl${at.level}sh0.png`; await image.decode();
        const sheet = document.createElement('canvas'); sheet.width = image.width; sheet.height = image.height;
        const source = sheet.getContext('2d')!; source.drawImage(image, 0, 0);
        const expected = source.getImageData(56, 56, 1, 1).data;
        const actual = (element as HTMLCanvasElement).getContext('2d')!.getImageData(
          (at.x - Number(element.getAttribute('data-cam-x'))) * 32 + 24,
          (at.y - Number(element.getAttribute('data-cam-y'))) * 32 + 24, 1, 1).data;
        return expected[3] === 255 && actual.every((v, i) => v === expected[i]);
      }, { x: hero.x, y: hero.y - 1, level })).toBe(true);
      await clickTile(page, hero, hero.x, hero.y - 1);
      const dialog = page.getByRole('dialog', { name: 'Shrine of Magic' });
      await expect(dialog).toBeVisible();
      await expect(dialog.getByText(/learn/i)).toBeVisible();
      expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y, spells: [spell] });
      if (spell === 'magicArrow') await page.screenshot({ path: testInfo.outputPath('native-shrine-visit.png') });
      await dialog.getByRole('button', { name: 'OK', exact: true }).click();
      await clickTile(page, hero, hero.x, hero.y - 1);
      await expect(dialog.getByText(/already know/i)).toBeVisible();
    });
  }

  for (const [kind, data, title] of [
    ['artifact', 'artifact1', 'Artifact'], ['treasure', '1500:1000', 'Treasure chest'],
    ['monster', 'pikeman:5', ''], ['obelisk', '', ''], ['grail', '', 'The Grail']
  ]) {
    test(`S24 ${kind} previews a visit and executes its own interaction`, async ({ page }) => {
      await choosePlayer(page, 'Alice');
      const name = `Object ${kind}`;
      await createGame(page, name);
      const { hero } = await adventureFixture(page, name, { objectKind: kind, objectData: data });
      const point = await tilePoint(page, hero.x, hero.y - 1);
      await page.mouse.move(point.x, point.y);
      await expect(page.locator('canvas').first()).toHaveAttribute('data-cursor-frame', '9');
      await clickTile(page, hero, hero.x, hero.y - 1);
      if (title) await expect(page.getByRole('dialog', { name: title, exact: true })).toBeVisible();
      if (kind === 'monster') {
        await expect(page.getByRole('dialog', { name: 'Creature encounter' }).getByRole('button', { name: 'Yes', exact: true })).toBeVisible();
        expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y });
      }
      if (kind === 'obelisk') {
        await expect(page).toHaveURL(/\/obelisk$/);
        expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y });
      }
      if (kind === 'grail') expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y - 1, movement: 0 });
    });
  }

  test('S25 clicking the solid town body plans a gate visit and enters only after confirmation', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Town gate pointer');
    const { hero } = await adventureFixture(page, 'Town gate pointer', { objectKind: 'tree' });
    const response = await page.request.post(`${BASE}/api/heroes/${hero.id}/move`, {
      headers: { 'X-Player-Id': 'alice' }, data: { path: [
        { x: hero.x + 1, y: hero.y }, { x: hero.x + 1, y: hero.y - 1 }, { x: hero.x + 1, y: hero.y - 2 },
        { x: hero.x + 1, y: hero.y - 3 }, { x: hero.x + 1, y: hero.y - 4 }, { x: hero.x, y: hero.y - 4 }
      ] }
    });
    expect(response.ok()).toBe(true);
    const before = await response.json() as HeroDetail;
    await page.reload();
    await expect(page.getByText('Christian')).toBeVisible();
    const canvas = page.locator('canvas').first();
    const point = await tilePoint(page, hero.x + 2, hero.y - 5);
    await page.mouse.move(point.x, point.y);
    await expect(canvas).toHaveAttribute('data-cursor-frame', '9');
    await page.mouse.click(point.x, point.y);
    await expect(canvas).toHaveAttribute('data-path-end', `${hero.x},${hero.y - 5}`);
    expect(await readHero(page, hero.id)).toMatchObject({ x: before.x, y: before.y, movement: before.movement });
    await page.mouse.click(point.x, point.y);
    await expect(page.locator('.town-scene canvas')).toBeVisible();
    expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y - 5, movement: before.movement - 100 });
  });

  test('S26 insufficient movement uses next-day visit cursor and the actual red destination cross', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Next day pointer');
    const { hero } = await adventureFixture(page, 'Next day pointer', { objectKind: 'resource', objectData: 'gold:500', movement: 0 });
    const canvas = page.locator('canvas').first();
    const point = await tilePoint(page, hero.x, hero.y - 1);
    await page.mouse.move(point.x, point.y);
    await expect(canvas).toHaveAttribute('data-cursor-frame', '15');
    await page.mouse.click(point.x, point.y);
    await expect(canvas).toHaveAttribute('data-path-end', `${hero.x},${hero.y - 1}`);
    await expect.poll(() => canvas.evaluate(async (element, at) => {
      const meta = await (await fetch('/h3/def/ADAG.json')).json();
      const image = new Image(); image.src = '/h3/def/ADAG.png'; await image.decode();
      const sheet = document.createElement('canvas'); sheet.width = meta.w; sheet.height = meta.h;
      const source = sheet.getContext('2d')!;
      source.drawImage(image, 25 % meta.cols * meta.w, Math.floor(25 / meta.cols) * meta.h, meta.w, meta.h, 0, 0, meta.w, meta.h);
      const expected = source.getImageData(0, 0, meta.w, meta.h).data;
      const actual = (element as HTMLCanvasElement).getContext('2d')!.getImageData(
        (at.x - Number(element.getAttribute('data-cam-x'))) * 32,
        (at.y - Number(element.getAttribute('data-cam-y'))) * 32, meta.w, meta.h).data;
      let matched = 0;
      for (let i = 0; i < expected.length; i += 4) {
        if (expected[i + 3] !== 255) continue;
        if (expected[i] !== actual[i] || expected[i + 1] !== actual[i + 1] || expected[i + 2] !== actual[i + 2]) return false;
        matched++;
      }
      return matched > 0;
    }, { x: hero.x, y: hero.y - 1 })).toBe(true);
    expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y, movement: 0 });
    let moveRequests = 0;
    page.on('request', request => { if (request.method() === 'POST' && request.url().endsWith(`/heroes/${hero.id}/move`)) moveRequests++; });
    await page.mouse.click(point.x, point.y);
    await page.getByRole('button', { name: 'Move Hero', exact: true }).click();
    await expect(canvas).toHaveAttribute('data-path-end', `${hero.x},${hero.y - 1}`);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(moveRequests).toBe(0);
    expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y, movement: 0 });
    await page.getByRole('button', { name: 'End Turn', exact: true }).click();
    await expect(page.getByRole('status', { name: 'New turn' })).toHaveAttribute('data-day', '2');
    const nextPoint = await tilePoint(page, hero.x, hero.y - 1);
    await page.mouse.move(nextPoint.x, nextPoint.y);
    await expect(canvas).toHaveAttribute('data-cursor-frame', '9');
    await expect(canvas).toHaveAttribute('data-path-end', `${hero.x},${hero.y - 1}`);
    await page.getByRole('button', { name: 'Move Hero', exact: true }).click();
    await expect(page.getByRole('status', { name: 'Adventure reward' })).toHaveAttribute('data-amount', '500');
    expect(moveRequests).toBe(1);
  });

  for (const accept of [true, false]) {
    test(`S27 creature offer uses native Yes/No, survives reload and ${accept ? 'joins' : 'declines without moving through'}`, async ({ page }, testInfo) => {
      await choosePlayer(page, 'Alice');
      const name = `Creature offer ${accept}`;
      await createGame(page, name);
      const { hero, gameId } = await adventureFixture(page, name, { objectKind: 'monster', objectData: 'pikeman:5' });
      const before = await (await page.request.get(`${BASE}/api/games/${gameId}`)).json() as GameDetail;
      await clickTile(page, hero, hero.x, hero.y - 1);
      await expect(page.getByRole('dialog', { name: 'Creature encounter' })).toBeVisible();
      await page.reload();
      const dialog = page.getByRole('dialog', { name: 'Creature encounter' });
      await expect(dialog).toContainText('5 Pikemen');
      await expect(dialog).toContainText('300 gold');
      await expect(dialog.getByRole('button')).toHaveCount(2);
      await expect(dialog.locator('[style*="RESOUR82"]')).toBeVisible();
      await expect(dialog.getByRole('button', { name: 'Yes', exact: true }).locator('[style*="IOKAY"]')).toBeVisible();
      await expect(page.getByRole('button', { name: /Observe|Pass by/ })).toHaveCount(0);
      if (accept) await page.screenshot({ path: testInfo.outputPath('native-creature-offer.png') });
      const choice = page.waitForResponse(r => r.url().endsWith(`/heroes/${hero.id}/encounter`) && r.request().method() === 'POST');
      await dialog.getByRole('button', { name: accept ? 'Yes' : 'No', exact: true }).click();
      expect((await choice).request().postDataJSON()).toEqual({ action: accept ? 'recruit' : 'pass' });
      await expect(dialog).not.toBeVisible();
      const after = await readHero(page, hero.id);
      expect(after).toMatchObject({ x: hero.x, y: hero.y, pendingEncounter: null, experience: hero.experience });
      const count = (h: HeroDetail) => h.army.filter(s => s.creature === 'pikeman').reduce((n, s) => n + s.count, 0);
      expect(count(after)).toBe(count(hero) + (accept ? 5 : 0));
      const game = await (await page.request.get(`${BASE}/api/games/${gameId}`)).json() as GameDetail;
      expect(game.players.find(p => p.name === 'Alice')!.resources.gold)
        .toBe(before.players.find(p => p.name === 'Alice')!.resources.gold - (accept ? 300 : 0));
      const map = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json() as MapData;
      expect(map.tiles[(hero.y - 1) * map.width + hero.x])
        .toMatchObject({ objectKind: accept ? null : 'monster', passable: accept });
      await expect(page.locator('#toast.show')).toHaveCount(0);
    });
  }

  test('S28 non-joining creatures have a single native acknowledgment and stay solid', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'No creature offer');
    const { hero, gameId } = await adventureFixture(page, 'No creature offer', {
      objectKind: 'monster', objectData: 'pikeman:5000'
    });
    await clickTile(page, hero, hero.x, hero.y - 1);
    const dialog = page.getByRole('dialog', { name: 'Creature encounter' });
    await expect(dialog).toContainText('will not join');
    await expect(dialog.getByRole('button')).toHaveCount(1);
    await dialog.getByRole('button', { name: 'OK', exact: true }).click();
    await expect(dialog).not.toBeVisible();
    expect(await readHero(page, hero.id)).toMatchObject({ x: hero.x, y: hero.y, pendingEncounter: null, experience: hero.experience });
    const map = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json() as MapData;
    expect(map.tiles[(hero.y - 1) * map.width + hero.x]).toMatchObject({ objectKind: 'monster', passable: false });
  });

  for (const failure of ['gold', 'army']) {
    test(`S29 creature offer ${failure} failure uses a native error dialog without consuming the offer`, async ({ page }) => {
      await choosePlayer(page, 'Alice');
      const name = `Creature failure ${failure}`;
      await createGame(page, name);
      const { hero, gameId } = await adventureFixture(page, name, {
        objectKind: 'monster', objectData: 'pikeman:5', gold: failure === 'gold' ? 0 : 5000,
        ...(failure === 'army' ? { army: ['archer', 'griffin', 'swordsman', 'monk', 'cavalier', 'angel', 'skeleton']
          .map(creature => ({ creature, count: 10 })) } : {})
      });
      await clickTile(page, hero, hero.x, hero.y - 1);
      const offer = page.getByRole('dialog', { name: 'Creature encounter' });
      await offer.getByRole('button', { name: 'Yes', exact: true }).click();
      const error = page.getByRole('dialog', { name: 'Adventure', exact: true });
      await expect(error).toContainText(failure === 'gold' ? 'Not enough gold' : 'army is full');
      expect(await readHero(page, hero.id)).toMatchObject({ army: hero.army, experience: hero.experience });
      expect((await readHero(page, hero.id)).pendingEncounter).not.toBeNull();
      await error.getByRole('button', { name: 'OK', exact: true }).click();
      await offer.getByRole('button', { name: 'No', exact: true }).click();
      await expect(offer).not.toBeVisible();
      const map = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json() as MapData;
      expect(map.tiles[(hero.y - 1) * map.width + hero.x]).toMatchObject({ objectKind: 'monster', passable: false });
      await expect(page.locator('#toast.show')).toHaveCount(0);
    });
  }

  test('S30 movement exhaustion preserves the route and continues next day without a popup', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Partial movement');
    const { hero } = await adventureFixture(page, 'Partial movement', { objectKind: 'tree', movement: 100 });
    const canvas = page.locator('canvas').first();
    await clickTile(page, hero, hero.x + 2, hero.y);
    await expect.poll(async () => {
      const h = await readHero(page, hero.id);
      return { x: h.x, y: h.y, movement: h.movement };
    }).toEqual({ x: hero.x + 1, y: hero.y, movement: 0 });
    await expect(page.getByRole('button', { name: 'Christian', exact: true })).toHaveAttribute('data-movement', '0');
    await expect(canvas).toHaveAttribute('data-path-end', `${hero.x + 2},${hero.y}`);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.getByRole('button', { name: 'End Turn', exact: true }).click();
    await expect(page.getByRole('status', { name: 'New turn' })).toHaveAttribute('data-day', '2');
    await page.getByRole('button', { name: 'Move Hero', exact: true }).click();
    await expect.poll(async () => (await readHero(page, hero.id)).x).toBe(hero.x + 2);
    await expect(canvas).toHaveAttribute('data-path-end', '');
  });

  test('S31 end-turn reminder can cancel; day/week feedback uses the original info-bar animations', async ({ page }, testInfo) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Native turn feedback');
    const { hero, gameId } = await adventureFixture(page, 'Native turn feedback', { objectKind: 'tree' });
    await page.getByRole('button', { name: 'End Turn', exact: true }).click();
    const question = page.getByRole('dialog', { name: 'End turn', exact: true });
    await expect(question).toContainText('heroes can still move');
    await question.getByRole('button', { name: 'No', exact: true }).click();
    expect((await (await page.request.get(`${BASE}/api/games/${gameId}`)).json()).day).toBe(1);
    await page.getByRole('button', { name: 'End Turn', exact: true }).click();
    await question.getByRole('button', { name: 'Yes', exact: true }).click();
    const notice = page.getByRole('status', { name: 'New turn' });
    await expect(notice).toHaveAttribute('data-day', '2');
    await expect(notice.locator('[style*="NEWDAY"]')).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('native-new-day.png') });
    await expect(notice).not.toBeVisible({ timeout: 5000 });
    expect((await readHero(page, hero.id)).movement).toBe(hero.maxMovement);
    expect((await page.request.post(`${BASE}/api/games/${gameId}/endturn`, { headers: { 'X-Player-Id': 'alice' } })).ok()).toBe(true);
    await expect(notice).toHaveAttribute('data-day', '3');
    for (let day = 4; day <= 7; day++) {
      expect((await page.request.post(`${BASE}/api/games/${gameId}/_test/nextday`)).ok()).toBe(true);
      await expect(notice).toHaveAttribute('data-day', String(day));
    }
    expect((await page.request.post(`${BASE}/api/games/${gameId}/_test/nextday`)).ok()).toBe(true);
    await expect(notice).toHaveAttribute('data-week', '2');
    await expect(notice).toHaveAttribute('data-animation', 'NEWWEEK2');
    await expect(notice.locator('[style*="NEWWEEK2"]')).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('native-new-week.png') });
    await notice.click();
    await expect(notice).not.toBeVisible();
    await expect(page.locator('#toast.show')).toHaveCount(0);
    await expect(page.getByText(/Next day in|A new day begins/)).toHaveCount(0);
  });

  test('S32 sleep persists without ending the turn; explicit wake and movement wake the hero', async ({ page }) => {
    await choosePlayer(page, 'Alice');
    await createGame(page, 'Native sleep');
    const { hero, gameId } = await adventureFixture(page, 'Native sleep', { objectKind: 'tree' });
    const heroButton = page.getByRole('button', { name: 'Christian', exact: true });
    await page.getByRole('button', { name: 'Sleep', exact: true }).click();
    await expect(heroButton).toHaveAttribute('data-sleeping', 'true');
    await page.reload();
    await page.getByRole('button', { name: 'Wake', exact: true }).click();
    await expect(heroButton).toHaveAttribute('data-sleeping', 'false');
    expect((await (await page.request.get(`${BASE}/api/games/${gameId}`)).json()).day).toBe(1);
    expect((await readHero(page, hero.id)).movement).toBe(hero.movement);
    await page.getByRole('button', { name: 'Sleep', exact: true }).click();
    await clickTile(page, hero, hero.x + 1, hero.y);
    await expect.poll(async () => (await readHero(page, hero.id)).x).toBe(hero.x + 1);
    await expect(heroButton).toHaveAttribute('data-movement', String(hero.movement - 100));
    await expect(heroButton).toHaveAttribute('data-sleeping', 'false');
    await page.getByRole('button', { name: 'Sleep', exact: true }).click();
    await page.getByRole('button', { name: 'End Turn', exact: true }).click();
    await expect(page.getByRole('status', { name: 'New turn' })).toHaveAttribute('data-day', '2');
    await expect(heroButton).toHaveAttribute('data-sleeping', 'true');
    await expect(page.getByRole('dialog')).toHaveCount(0);
  });

  test('S33 meeting owned heroes opens native exchange and transfers split armies and artifacts', async ({ page }, testInfo) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Exchange fidelity');
    const { hero, gameId } = await adventureFixture(page, 'Exchange fidelity', {
      meeting: 'own', army: [{ creature: 'pikeman', count: 25 }, { creature: 'archer', count: 8 }]
    });
    await page.request.post(`${BASE}/api/heroes/${hero.id}/_test/artifacts`, { data: ['artifact1', 'artifact2'] });
    await page.reload();
    await clickTile(page, hero, hero.x + 1, hero.y);
    const exchange = page.getByRole('dialog', { name: 'Hero exchange' });
    await expect(exchange.locator('img[src*="TRADE2"]')).toBeVisible();
    await exchange.locator('[data-exchange-army="0:0"]').click();
    await exchange.locator('[data-exchange-army="1:2"]').click({ modifiers: ['Shift'] });
    await page.getByRole('button', { name: 'Split', exact: true }).click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[0].count).toBe(13);
    await exchange.locator('[data-exchange-army="1:2"]').click();
    await exchange.locator('[data-exchange-army="0:0"]').click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[0].count).toBe(25);
    await exchange.locator('[data-exchange-army="0:1"]').click();
    await exchange.locator('[data-exchange-army="1:2"]').click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[1].creature).toBeNull();
    await exchange.locator('[data-exchange-army="1:2"]').click();
    await exchange.locator('[data-exchange-army="0:0"]').click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[0].creature).toBe('archer');
    await exchange.locator('[data-exchange-artifact="0:leftHand"]').click();
    await exchange.locator('[data-exchange-artifact="1:leftHand"]').click();
    await expect.poll(async () => (await readHero(page, hero.id)).worn.leftHand ?? null).toBeNull();
    const heroes: HeroDetail[] = await (await page.request.get(`${BASE}/api/games/${gameId}/heroes`, { headers: { 'X-Player-Id': 'alice' } })).json();
    expect(heroes.find(h => h.id !== hero.id)!.worn.leftHand.id).toBe(14);
    await page.screenshot({ path: testInfo.outputPath('native-exchange.png') });
    await exchange.getByRole('button', { name: 'Close exchange' }).click();
    expect((await readHero(page, hero.id)).movement).toBe(hero.movement);
  });

  test('S34 hero equipment supports unequip, replacement and native incompatible-slot errors', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Equipment fidelity');
    const { hero } = await adventureFixture(page, 'Equipment fidelity', {});
    await page.request.post(`${BASE}/api/heroes/${hero.id}/_test/artifacts`, { data: ['artifact1', 'artifact2', 'artifact10'] });
    await page.getByRole('button', { name: 'Hero', exact: true }).click();
    await page.getByRole('button', { name: 'Artifact leftHand', exact: true }).click();
    await page.getByRole('button', { name: 'Backpack 2', exact: true }).click();
    await expect(page.locator('[data-art-slot="leftHand"]')).not.toHaveAttribute('data-art-id', '14');
    await page.getByRole('button', { name: 'Backpack 1', exact: true }).click();
    await page.getByRole('button', { name: 'Artifact leftHand', exact: true }).click();
    await expect(page.locator('[data-art-slot="leftHand"]')).toHaveAttribute('data-art-id', '17');
    await page.getByRole('button', { name: 'Artifact leftHand', exact: true }).click();
    await page.getByRole('button', { name: 'Artifact head', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Message' })).toContainText('does not fit');
    await page.getByRole('button', { name: 'OK', exact: true }).click();
    await page.reload();
    await expect(page.locator('[data-art-slot="leftHand"]')).toHaveAttribute('data-art-id', '17');
    expect((await readHero(page, hero.id)).backpack[0].id).toBe(14);
  });

  test('S35 troop details precede dismissal and confirmations preserve the final stack', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Dismiss troops');
    const { hero } = await adventureFixture(page, 'Dismiss troops', {
      army: [{ creature: 'pikeman', count: 25 }, { creature: 'archer', count: 8 }]
    });
    await page.getByRole('button', { name: 'Hero', exact: true }).click();
    await page.locator('[data-hero-army-slot="0"]').dblclick();
    await expect(page.locator('img[src*="CRSTKPU"]')).toBeVisible();
    await page.getByRole('button', { name: 'Dismiss creatures' }).click();
    await page.getByRole('button', { name: 'No', exact: true }).click();
    expect((await readHero(page, hero.id)).army[0].count).toBe(25);
    await page.locator('[data-hero-army-slot="0"]').dblclick();
    await page.getByRole('button', { name: 'Dismiss creatures' }).click();
    await page.getByRole('button', { name: 'Yes', exact: true }).click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[0].creature).toBeNull();
    await page.locator('[data-hero-army-slot="1"]').dblclick();
    await page.getByRole('button', { name: 'Dismiss creatures' }).click();
    await page.getByRole('button', { name: 'Yes', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Message' })).toContainText('at least one stack');
    expect((await readHero(page, hero.id)).army[1].count).toBe(8);
  });

  test('S36 dismissing a hero requires native confirmation', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Dismiss hero');
    const { hero } = await adventureFixture(page, 'Dismiss hero', {});
    await page.getByRole('button', { name: 'Hero', exact: true }).click();
    await page.getByRole('button', { name: 'Dismiss hero', exact: true }).click();
    await page.getByRole('button', { name: 'No', exact: true }).click();
    expect((await readHero(page, hero.id)).id).toBe(hero.id);
    await page.getByRole('button', { name: 'Dismiss hero', exact: true }).click();
    await page.getByRole('button', { name: 'Yes', exact: true }).click();
    await expect(page.locator('canvas').first()).toBeVisible();
    expect((await page.request.get(`${BASE}/api/heroes/${hero.id}`)).status()).toBe(404);
  });

  test('S37 native level choices survive reload and resolve multiple levels in sequence', async ({ page }, testInfo) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Level fidelity');
    const { hero } = await adventureFixture(page, 'Level fidelity', { experience: 3000, skills: {} });
    const level = page.getByRole('dialog', { name: 'Level up' });
    await expect(level.locator('img[src*="LVLUPBKG"]')).toBeVisible();
    await expect(level.getByRole('button', { name: 'Confirm skill' })).toBeDisabled();
    const offered = (await readHero(page, hero.id)).pendingLevel!;
    await page.reload();
    expect((await readHero(page, hero.id)).pendingLevel).toEqual(offered);
    await expect(level.locator('[style*="PSKIL42.png"]')).toBeVisible();
    await expect(level.locator('[style*="SECSKILL.png"]').first()).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('native-level.png') });
    await level.locator('[data-level-skill]').first().click();
    await level.getByRole('button', { name: 'Confirm skill' }).click();
    await expect(level).toContainText('level 3');
    await level.locator('[data-level-skill]').first().click();
    await level.getByRole('button', { name: 'Confirm skill' }).click();
    await expect(level).toBeHidden();
    expect((await readHero(page, hero.id)).level).toBe(3);
    expect(Object.keys((await readHero(page, hero.id)).skills).length).toBeGreaterThan(0);
  });

  test('S38 Advanced Town Portal uses the native-style destination selector and retains movement', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Portal fidelity');
    const { hero, gameId } = await adventureFixture(page, 'Portal fidelity', {
      spells: ['townPortal'], skills: { earth: 2, wisdom: 3 }, portalTown: true
    });
    const game: GameDetail = await (await page.request.get(`${BASE}/api/games/${gameId}`, { headers: { 'X-Player-Id': 'alice' } })).json();
    const map: MapData = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json();
    const target = map.towns.find(t => t.name === 'Portal destination')!;
    expect(target).toBeTruthy(); expect(game.state).toBe('InProgress');
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    await page.getByRole('button', { name: 'Cast Town Portal', exact: true }).click();
    const towns = page.getByRole('dialog', { name: 'Town Portal destination' });
    await expect(towns.getByRole('button', { name: 'Confirm town' })).toBeDisabled();
    await towns.getByRole('button', { name: 'Portal to Portal destination' }).click();
    await towns.getByRole('button', { name: 'Confirm town' }).click();
    await expect.poll(async () => {
      const current = await readHero(page, hero.id);
      return [current.x, current.y, current.movement, current.mana];
    }).toEqual([target.x, target.y, hero.movement - 300, 88]);
    await expect(page.locator('.town-scene')).toBeVisible();
  });

  test('S39 native spellbook hides combat casts and Dimension Door uses cancelable map targeting', async ({ page }, testInfo) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Door fidelity');
    const { hero } = await adventureFixture(page, 'Door fidelity', { spells: ['dimensionDoor', 'magicArrow', 'fly'], flightLane: true });
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    const book = page.getByRole('dialog', { name: 'Spellbook' });
    await expect(book.locator('img[src*="SpelBack"]')).toBeVisible();
    await expect(book.getByRole('button', { name: 'Cast Magic Arrow' })).toHaveCount(0);
    await book.getByRole('button', { name: 'Air spells', exact: true }).click();
    await expect(book.locator('[data-spell="dimensionDoor"]')).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('native-spellbook.png') });
    await book.getByRole('button', { name: 'Cast Dimension Door', exact: true }).click();
    await page.keyboard.press('Escape');
    expect((await readHero(page, hero.id)).mana).toBe(100);
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    await page.getByRole('button', { name: 'Cast Dimension Door', exact: true }).click();
    const point = await tilePoint(page, hero.x, hero.y + 2);
    await page.mouse.click(point.x, point.y);
    await expect.poll(async () => (await readHero(page, hero.id)).doorCasts).toBe(1);
    expect((await readHero(page, hero.id)).y).toBe(hero.y + 2);
    expect((await readHero(page, hero.id)).mana).toBe(75);
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Cast Dimension Door', exact: true })).toBeDisabled();
  });

  test('S40 Visions reveals real creature information through map right-click', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Visions fidelity');
    await adventureFixture(page, 'Visions fidelity', { meeting: 'own' });
    const { hero } = await adventureFixture(page, 'Visions fidelity', { spells: ['visions'], objectKind: 'monster', objectData: 'pikeman:5' });
    await page.getByRole('button', { name: 'Christian', exact: true }).click();
    await page.getByRole('button', { name: 'Spells', exact: true }).click();
    await page.getByRole('button', { name: 'Cast Visions', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Spellbook' })).toBeHidden();
    await expect.poll(async () => (await readHero(page, hero.id)).visions).toBe(true);
    const point = await tilePoint(page, hero.x, hero.y - 1);
    await page.mouse.click(point.x, point.y, { button: 'right' });
    await expect(page.getByRole('dialog', { name: 'Message' })).toContainText('5 Pikeman');
    expect((await readHero(page, hero.id)).pendingEncounter).toBeNull();
    await page.getByRole('button', { name: 'OK', exact: true }).click();
    await page.getByRole('button', { name: 'Companion', exact: true }).click();
    const otherPoint = await tilePoint(page, hero.x, hero.y - 1);
    await page.mouse.click(otherPoint.x, otherPoint.y, { button: 'right' });
    await expect(page.getByRole('dialog', { name: 'Message' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Christian', exact: true }).click();
    const originalPoint = await tilePoint(page, hero.x, hero.y - 1);
    await page.mouse.click(originalPoint.x, originalPoint.y, { button: 'right' });
    await expect(page.getByRole('dialog', { name: 'Message' })).toContainText('5 Pikeman');
  });

  test('S41 mine ownership grants and revokes half-yield sharing through native confirmations', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Mine permission fidelity');
    const { hero, gameId } = await adventureFixture(page, 'Mine permission fidelity', { objectKind: 'mine', objectData: 'mercury' });
    await clickTile(page, hero, hero.x, hero.y - 1);
    await page.getByRole('button', { name: 'OK', exact: true }).click();
    const point = await tilePoint(page, hero.x, hero.y - 1);
    const shared = async () => {
      const map = await (await page.request.get(`${BASE}/api/games/${gameId}/map`, { headers: { 'X-Player-Id': 'alice' } })).json();
      return map.tiles[(hero.y - 1) * map.width + hero.x].mineShared;
    };
    await page.mouse.click(point.x, point.y, { button: 'right' });
    await expect(page.getByRole('dialog', { name: 'Confirmation' })).toContainText('Half units are saved');
    await page.getByRole('button', { name: 'Yes', exact: true }).click();
    await expect.poll(shared).toBe(true);
    await page.mouse.click(point.x, point.y, { button: 'right' });
    await page.getByRole('button', { name: 'No', exact: true }).click();
    expect(await shared()).toBe(true);
    await page.mouse.click(point.x, point.y, { button: 'right' });
    await page.getByRole('button', { name: 'Yes', exact: true }).click();
    await expect.poll(shared).toBe(false);
  });

  test('S42 physical other-player meetings link messages, map markers and accepted trades', async ({ page, browser }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Social meeting fidelity');
    const { hero, gameId } = await adventureFixture(page, 'Social meeting fidelity', { meeting: 'other' });
    await clickTile(page, hero, hero.x + 1, hero.y);
    const meeting = page.getByRole('dialog', { name: 'Hero meeting' });
    await meeting.getByLabel('Message', { exact: true }).fill('Road meeting message');
    await meeting.getByRole('button', { name: 'Send message', exact: true }).click();
    await clickTile(page, hero, hero.x + 1, hero.y);
    await meeting.getByRole('button', { name: 'Share map marker' }).click();
    await page.getByRole('dialog', { name: 'Add note' }).getByPlaceholder('e.g. rich ore here').fill('Road meeting marker');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await clickTile(page, hero, hero.x + 1, hero.y);
    await meeting.getByRole('button', { name: 'Offer a trade…' }).click();
    await page.getByLabel('Give amount').fill('1'); await page.getByLabel('Want amount').fill('1');
    await page.getByRole('button', { name: 'Offer', exact: true }).click();
    const context = await browser.newContext();
    try {
      const bob = await context.newPage(); await choosePlayer(bob, 'Bob');
      await bob.goto(`${BASE}/#/game/${gameId}/social`);
      await expect(bob.getByText('Road meeting message')).toBeVisible();
      await bob.getByText('accept', { exact: true }).click();
      await expect(bob.getByText('· accepted', { exact: true })).toBeVisible();
      const trades = await (await bob.request.get(`${BASE}/api/games/${gameId}/trades`, { headers: { 'X-Player-Id': 'bob' } })).json();
      expect(trades.some((t: { status: string }) => t.status === 'accepted')).toBe(true);
      const annotations = await (await bob.request.get(`${BASE}/api/games/${gameId}/annotations`, { headers: { 'X-Player-Id': 'bob' } })).json();
      expect(annotations.some((a: { text: string }) => a.text === 'Road meeting marker')).toBe(true);
    } finally { await context.close(); }
  });

  test('S43 human unanimity, withdrawal and AI consent navigate both players to game over', async ({ page, browser }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Agreement fidelity', 2);
    const { gameId } = await adventureFixture(page, 'Agreement fidelity', { meeting: 'other' });
    const game: GameDetail = await (await page.request.get(`${BASE}/api/games/${gameId}`, { headers: { 'X-Player-Id': 'alice' } })).json();
    expect(game.players.filter(p => !p.isAi).map(p => p.name).sort()).toEqual(['Alice', 'Bob']);
    expect(game.players.some(p => p.isAi)).toBe(true);
    const context = await browser.newContext();
    try {
      const bob = await context.newPage(); await choosePlayer(bob, 'Bob');
      await page.goto(`${BASE}/#/game/${gameId}/social`); await bob.goto(`${BASE}/#/game/${gameId}/social`);
      await page.getByRole('button', { name: 'Agree to end game' }).click();
      await page.getByRole('button', { name: 'Yes', exact: true }).click();
      await page.getByRole('button', { name: 'Withdraw end agreement' }).click();
      await page.getByRole('button', { name: 'Yes', exact: true }).click();
      await page.getByRole('button', { name: 'Agree to end game' }).click();
      await page.getByRole('button', { name: 'Yes', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Withdraw end agreement' })).toBeVisible();
      await bob.getByRole('button', { name: 'Agree to end game' }).click();
      await bob.getByRole('button', { name: 'Yes', exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`/game/${gameId}/over`));
      await expect(bob).toHaveURL(new RegExp(`/game/${gameId}/over`));
    } finally { await context.close(); }
  });

  test('S44 town screens split, merge, upgrade, trade and buy gear using their real controls', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Town action fidelity');
    const { hero } = await adventureFixture(page, 'Town action fidelity', { gold: 20000,
      army: [{ creature: 'pikeman', count: 25 }, { creature: 'archer', count: 8 }],
      buildings: ['villageHall', 'fort', 'tavern', 'dwelling1', 'upg1', 'marketplace', 'blacksmith', 'mageGuild1'] });
    await page.getByRole('button', { name: 'Town', exact: true }).click();
    await page.locator('[data-army="hero"][data-slot="0"]').click();
    await page.locator('[data-army="garrison"][data-slot="0"]').click({ modifiers: ['Shift'] });
    await page.getByRole('dialog').getByRole('button', { name: 'Split', exact: true }).click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[0].count).toBe(13);
    await page.locator('[data-army="garrison"][data-slot="0"]').click();
    await page.locator('[data-army="hero"][data-slot="0"]').click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[0].count).toBe(25);
    await page.locator('[data-army="hero"][data-slot="0"]').dblclick();
    await page.getByRole('button', { name: 'Upgrade to Halberdier' }).click();
    await page.getByRole('button', { name: 'Upgrade', exact: true }).click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[0].creature).toBe('halberdier');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.locator('#toast.show')).toHaveCount(0);
    await page.locator('.town-scene [data-building-slot="marketplace"]').click({ force: true });
    await page.locator('[data-market="from"][data-resource="wood"]').click();
    await page.locator('[data-market="to"][data-resource="ore"]').click();
    await page.getByRole('button', { name: 'Max', exact: true }).click();
    await page.getByRole('button', { name: 'Trade', exact: true }).click();
    await expect(page.locator('[data-market="from"][data-resource="wood"]')).toContainText('0');
    await expect(page.locator('#toast.show')).toHaveCount(0);
    await page.getByRole('button', { name: 'Done', exact: true }).click();
    await page.locator('.town-scene [data-building-slot="blacksmith"]').click({ force: true });
    await page.getByRole('button', { name: 'Buy', exact: true }).click();
    await expect.poll(async () => (await readHero(page, hero.id)).warMachines.includes('ballista')).toBe(true);
    await page.locator('.town-scene [data-building-slot="mageGuild1"]').click({ force: true });
    await page.getByRole('button', { name: 'Buy spellbook', exact: true }).click();
    await expect.poll(async () => (await readHero(page, hero.id)).spellbook).toBe(true);
    expect((await readHero(page, hero.id)).spells.length).toBeGreaterThan(0);
    await expect(page.locator('#toast.show')).toHaveCount(0);
    await page.getByRole('dialog').getByRole('button', { name: 'Exit', exact: true }).click();
    await page.locator('[data-army="hero"][data-slot="1"]').click();
    await page.locator('[data-army="garrison"][data-slot="1"]').click();
    await expect.poll(async () => (await readHero(page, hero.id)).army[1].creature).toBeNull();
    await page.locator('[data-army="hero"][data-slot="0"]').click();
    await page.locator('[data-army="garrison"][data-slot="0"]').click();
    await expect(page.getByRole('dialog', { name: 'Message' })).toContainText('at least one stack');
    await expect(page.locator('#toast.show')).toHaveCount(0);
    expect((await readHero(page, hero.id)).army[0].count).toBe(25);
  });

  test('S45 projects accept an actual screen contribution without changing another resource', async ({ page }) => {
    await choosePlayer(page, 'Alice'); await createGame(page, 'Project action fidelity');
    const { gameId } = await adventureFixture(page, 'Project action fidelity', { gold: 20000 });
    await page.getByRole('button', { name: 'Projects', exact: true }).click();
    await page.getByLabel('Gold', { exact: true }).first().fill('100');
    await page.getByLabel('Wood', { exact: true }).first().fill('0');
    await page.getByLabel('Ore', { exact: true }).first().fill('0');
    await page.getByRole('button', { name: 'Contribute', exact: true }).first().click();
    await expect.poll(async () => {
      const projects = await (await page.request.get(`${BASE}/api/games/${gameId}/projects`, { headers: { 'X-Player-Id': 'alice' } })).json();
      return projects[0].contributedGold;
    }).toBe(100);
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