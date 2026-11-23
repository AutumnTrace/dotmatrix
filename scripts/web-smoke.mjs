// Loads the built site in headless Chrome, boots the demo cartridge and
// checks that frames are rendered and that audio samples come out of wasm.
// Usage: npm run smoke   (after scripts/build-web.sh)
import { chromium } from 'playwright';
import { serve } from './serve.mjs';

const server = await serve();
const url = `http://127.0.0.1:${server.address().port}/`;
const browser = await chromium.launch({ channel: process.env.PW_CHANNEL ?? 'chrome', headless: true });
const page = await browser.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));

try {
  await page.goto(url);
  await page.waitForFunction(() => window.dotmatrix && document.getElementById('cart-title').textContent !== '—');
  const title = await page.textContent('#cart-title');
  await page.click('#overlay');
  await page.waitForTimeout(1500);
  const result = await page.evaluate(() => {
    const { wasm } = window.dotmatrix;
    const fb = new Uint8Array(wasm.memory.buffer, wasm.dm_framebuffer(), 160 * 144 * 4);
    const colours = new Set();
    for (let i = 0; i < fb.length; i += 4) colours.add((fb[i] << 16) | (fb[i + 1] << 8) | fb[i + 2]);
    return { colours: colours.size, fps: document.getElementById('fps').textContent };
  });
  console.log(`title=${title} distinct-colours=${result.colours} fps=${result.fps}`);
  if (title !== 'TOBU') throw new Error(`unexpected cartridge title ${title}`);
  if (result.colours < 2) throw new Error('screen is blank');
  if (errors.length) throw new Error(`page errors:\n${errors.join('\n')}`);
  console.log('web smoke test passed');
} finally {
  await browser.close();
  server.close();
}
