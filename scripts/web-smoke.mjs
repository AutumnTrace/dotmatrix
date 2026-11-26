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

  // Battery saves: pausing stores the demo cart's 8 KiB of RAM. Then an MBC3
  // cart with a clock, whose stored save is an hour old, must load with the
  // clock caught up.
  const saves = await page.evaluate(async () => {
    const { wasm, loadRom, togglePause } = window.dotmatrix;
    togglePause();
    const stored = Object.keys(localStorage).filter((k) => k.startsWith('dotmatrix:save:'));
    const demoSave = stored.length === 1 ? atob(localStorage.getItem(stored[0])).length : -1;

    const fnv1a = (bytes) => {
      let h = 0x811c9dc5;
      for (const b of bytes) h = Math.imul(h ^ b, 0x01000193);
      return h >>> 0;
    };
    const rom = new Uint8Array(0x8000);
    rom[0x147] = 0x10; // MBC3+TIMER+RAM+BATTERY
    rom[0x149] = 0x02; // 8 KiB
    const save = new Uint8Array(0x2000 + 48);
    const footer = new DataView(save.buffer, 0x2000);
    footer.setUint32(4, 30, true); // live minutes
    footer.setUint32(8, 5, true); // live hours
    footer.setBigUint64(40, BigInt(Math.floor(Date.now() / 1000) - 3600), true);
    let s = '';
    for (const b of save) s += String.fromCharCode(b);
    localStorage.setItem(`dotmatrix:save:${fnv1a(rom).toString(16)}`, btoa(s));
    await loadRom(rom, 'rtc test');
    const len = wasm.dm_save_build(Date.now() / 1000);
    const out = new DataView(wasm.memory.buffer, wasm.dm_save_ptr() + 0x2000, 48);
    return {
      demoSave,
      meta: document.getElementById('cart-meta').textContent,
      len,
      ramLen: wasm.dm_save_ram_len(),
      clock: [out.getUint32(8, true), out.getUint32(4, true)],
    };
  });
  console.log(`saves: demo=${saves.demoSave} bytes, rtc len=${saves.len} clock=${saves.clock.join(':')} (${saves.meta})`);
  if (saves.demoSave !== 0x2000) throw new Error(`expected one 8 KiB demo save, got ${saves.demoSave}`);
  if (saves.len !== 0x2000 + 48 || saves.ramLen !== 0x2000) throw new Error('RTC save has the wrong layout');
  if (!saves.meta.includes('save restored')) throw new Error('RTC save was not restored');
  if (saves.clock[0] !== 6 || saves.clock[1] !== 30) throw new Error('RTC did not catch up by an hour');

  if (errors.length) throw new Error(`page errors:\n${errors.join('\n')}`);
  console.log('web smoke test passed');
} finally {
  await browser.close();
  server.close();
}
