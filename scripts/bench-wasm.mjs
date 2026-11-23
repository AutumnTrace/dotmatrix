// Measures the wasm build's raw emulation speed in headless Chrome: the
// demo cartridge is driven through the same button script as the native
// `dotmatrix run --bench` numbers in the README, with no rendering to the
// canvas and no frame pacing. Usage: scripts/build-web.sh && node scripts/bench-wasm.mjs
import { chromium } from 'playwright';
import { serve } from './serve.mjs';

const FRAMES = Number(process.argv[2] ?? 6000);
const server = await serve();
const browser = await chromium.launch({ channel: process.env.PW_CHANNEL ?? 'chrome', headless: true });
const page = await browser.newPage();
try {
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.waitForFunction(() => window.dotmatrix && document.getElementById('cart-title').textContent === 'TOBU');
  const runs = [];
  for (let run = 0; run < 3; run++) {
    runs.push(await page.evaluate(async (frames) => {
      const { wasm, loadRom } = window.dotmatrix;
      const rom = new Uint8Array(await (await fetch('roms/tobutobugirl.gb')).arrayBuffer());
      await loadRom(rom, 'bench');
      const presses = { 450: 7, 700: 7, 1100: 7, 1300: 7, 1500: 4 }; // button indices: 7 start, 4 A
      const t0 = performance.now();
      for (let f = 0; f < frames; f++) {
        if (presses[f] !== undefined) wasm.dm_set_button(presses[f], 1);
        if (presses[f - 5] !== undefined) wasm.dm_set_button(presses[f - 5], 0);
        wasm.dm_run_frame();
        wasm.dm_audio_clear();
      }
      return performance.now() - t0;
    }, FRAMES));
  }
  const best = Math.min(...runs);
  const fps = (FRAMES * 1000) / best;
  const version = browser.version();
  console.log(`Chrome ${version}: ${FRAMES} frames, best of 3 = ${best.toFixed(0)} ms -> ${fps.toFixed(0)} frames/s, ${(fps / 59.7275).toFixed(1)}x real time`);
} finally {
  await browser.close();
  server.close();
}
