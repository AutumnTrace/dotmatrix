// Records the README media from the real web build in headless Chrome:
//   docs/media/demo.gif        the console playing Tobu Tobu Girl
//   docs/media/web-desktop.png the page at 1280 px
//   docs/media/web-mobile.png  the page on a phone-sized viewport
//
// Emulation is stepped by hand (window.dotmatrix.step), so every GIF frame
// is exactly two emulated frames no matter how slow screenshots are.
// Usage: scripts/build-web.sh && npm run media
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { PNG } from 'pngjs';
import gifenc from 'gifenc';
import { serve } from './serve.mjs';

const { GIFEncoder, quantize, applyPalette } = gifenc;
const OUT = new URL('../docs/media/', import.meta.url);
const FRAMES_PER_SHOT = 2;

// Button presses in emulated frames, worked out by trial with
// `dotmatrix run --press ... --screenshot`.
const SCRIPT = [
  [450, 'start', 5], [700, 'start', 5], [1100, 'start', 5],
  [1300, 'start', 6], [1500, 'a', 6],
  [1640, 'right', 30], [1700, 'a', 4], [1730, 'left', 40], [1790, 'a', 4],
  [1820, 'right', 24], [1860, 'a', 4], [1890, 'left', 30], [1940, 'a', 4], [1960, 'right', 20],
];
const CAPTURE_FROM = 1210;
const CAPTURE_TO = 2010;

async function boot(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.dotmatrix && document.getElementById('cart-title').textContent === 'TOBU');
  await page.evaluate(() => {
    document.getElementById('overlay').hidden = true;
    document.getElementById('led').classList.add('on');
  });
}

async function advance(page, from, to) {
  for (let f = from; f < to; f++) {
    const events = SCRIPT.flatMap(([at, button, hold]) =>
      at === f ? [[button, true]] : at + hold === f ? [[button, false]] : []);
    await page.evaluate((evs) => {
      for (const [b, down] of evs) window.dotmatrix.hold(b, down);
      window.dotmatrix.step(1);
    }, events);
  }
}

async function recordGif(browser, url) {
  const page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
  await boot(page, url);
  // 0.68 of the console width is the LCD: make it exactly 2x (320 px).
  await page.addStyleTag({ content: '.stage { grid-template-columns: 470.5882px 320px !important; }' });
  await page.evaluate(() => {
    const grid = document.getElementById('grid');
    if (grid.checked) grid.click();
  });
  await advance(page, 0, CAPTURE_FROM);

  const box = await page.locator('.console').boundingBox();
  const pad = 18;
  const clip = { x: Math.floor(box.x - pad), y: Math.floor(box.y - pad), width: Math.ceil(box.width + 2 * pad), height: Math.ceil(box.height + 2 * pad) };

  const shots = [];
  for (let f = CAPTURE_FROM; f < CAPTURE_TO; f += FRAMES_PER_SHOT) {
    shots.push(PNG.sync.read(await page.screenshot({ clip })).data);
    await advance(page, f, f + FRAMES_PER_SHOT);
  }
  await page.close();

  // One palette for the whole clip (255 colours + 1 transparent). Pixels
  // that did not change since the previous frame become transparent, so
  // each GIF frame only stores the LCD and whichever button moved.
  const sample = new Uint8Array(shots.length > 8 ? shots[0].length * 2 : shots[0].length);
  sample.set(shots[0]);
  if (shots.length > 8) sample.set(shots[Math.floor(shots.length * 0.75)], shots[0].length);
  const palette = quantize(sample, 255);
  const transparent = palette.length;
  palette.push([0, 0, 0]);

  const gif = GIFEncoder();
  let previous = null;
  for (const rgba of shots) {
    const full = applyPalette(rgba, palette);
    const index = previous ? full.map((v, i) => (v === previous[i] ? transparent : v)) : full;
    gif.writeFrame(index, clip.width, clip.height, {
      palette,
      delay: Math.round((1000 * FRAMES_PER_SHOT) / 59.7275),
      transparent: previous !== null,
      transparentIndex: transparent,
      dispose: 1,
    });
    previous = full;
  }
  gif.finish();
  const bytes = gif.bytes();
  await writeFile(new URL('demo.gif', OUT), bytes);
  console.log(`demo.gif: ${shots.length} frames, ${clip.width}x${clip.height}, ${(bytes.length / 1024).toFixed(0)} KiB`);
}

async function pageShots(browser, url) {
  const desktop = await browser.newPage({ viewport: { width: 1280, height: 860 } });
  await boot(desktop, url);
  await advance(desktop, 0, 1640);
  // Hand over to the page's own loop for a moment so the stats are live.
  await desktop.evaluate(() => window.dotmatrix.start());
  await desktop.waitForTimeout(1500);
  await desktop.screenshot({ path: fileURLToPath(new URL('web-desktop.png', OUT)) });
  await desktop.close();

  const phone = await browser.newPage({
    viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true,
  });
  await boot(phone, url);
  await advance(phone, 0, 1640);
  await phone.evaluate(() => window.dotmatrix.start());
  await phone.waitForTimeout(1200);
  await phone.evaluate(() => window.dotmatrix.hold('right', true));
  await phone.waitForTimeout(300);
  await phone.screenshot({ path: fileURLToPath(new URL('web-mobile.png', OUT)) });
  await phone.close();
  console.log('web-desktop.png, web-mobile.png written');
}

await mkdir(OUT, { recursive: true });
const server = await serve();
const url = `http://127.0.0.1:${server.address().port}/`;
const browser = await chromium.launch({ channel: process.env.PW_CHANNEL ?? 'chrome', headless: true });
try {
  if (!process.argv.includes('--no-gif')) await recordGif(browser, url);
  await pageShots(browser, url);
} finally {
  await browser.close();
  server.close();
}
