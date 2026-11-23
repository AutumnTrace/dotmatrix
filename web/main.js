// dotmatrix browser frontend.
//
// The emulator is a ~60 KB wasm module with plain C-ABI exports (see
// crates/web/src/lib.rs). This file owns everything else: the frame loop,
// input from keyboard, touch and gamepads, audio through an AudioWorklet,
// and battery saves in localStorage.

const CLOCK_HZ = 4194304;
const FRAME_MS = (1000 * 70224) / CLOCK_HZ; // about 16.74 ms (59.73 Hz)
const FAST_FORWARD = 3;
const BUTTONS = ['right', 'left', 'up', 'down', 'a', 'b', 'select', 'start'];
const DEMO_ROM = 'roms/tobutobugirl.gb';

const PALETTES = {
  dmg: { name: 'DMG', colors: ['#9bbc0f', '#8bac0f', '#306230', '#0f380f'] },
  pocket: { name: 'Pocket', colors: ['#d8d8c0', '#a3a38c', '#5d5d4f', '#20201b'] },
  light: { name: 'Light', colors: ['#7ff2d4', '#32c79f', '#137a62', '#08302a'] },
  mono: { name: 'Mono', colors: ['#ffffff', '#aaaaaa', '#555555', '#000000'] },
};

const KEYMAP = {
  ArrowRight: 'right', ArrowLeft: 'left', ArrowUp: 'up', ArrowDown: 'down',
  KeyX: 'a', KeyZ: 'b', KeyS: 'a', KeyA: 'b',
  Enter: 'start', ShiftLeft: 'select', ShiftRight: 'select', Backspace: 'select',
};

const $ = (id) => document.getElementById(id);
const canvas = $('screen');
const ctx = canvas.getContext('2d', { alpha: false });
const image = ctx.createImageData(160, 144);

let wasm = null;
let romLoaded = false;
let romBytes = null;
let saveKey = null;
let lastSaveHash = 0;
let running = false;
let paused = false;
let fastKey = false;
let fastToggle = false;

// Every input source keeps its own set; the union is sent to the core.
const held = { keys: new Set(), touch: new Map(), pad: new Set(), mouse: new Set() };
let sentMask = 0;

// ---------------------------------------------------------------- wasm glue

async function instantiate() {
  const response = await fetch('dotmatrix.wasm');
  if (!response.ok) throw new Error(`dotmatrix.wasm: HTTP ${response.status}`);
  // The module imports nothing: no JS glue, no WASI.
  const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
  return instance.exports;
}

function memory(ptr, len) {
  return new Uint8Array(wasm.memory.buffer, ptr, len);
}

function writeInput(bytes) {
  const ptr = wasm.dm_input_buffer(bytes.length);
  memory(ptr, bytes.length).set(bytes);
}

function cartTitle() {
  const len = wasm.dm_title_len();
  return new TextDecoder().decode(memory(wasm.dm_title_ptr(), len)) || 'Untitled';
}

const LOAD_ERRORS = {
  1: 'That file is too small to be a Game Boy ROM.',
  2: 'This cartridge uses a memory controller dotmatrix does not support yet.',
  3: 'This is a Game Boy Color-only game. dotmatrix emulates the original monochrome model.',
};

// ---------------------------------------------------------------- saves

function fnv1a(bytes) {
  let h = 0x811c9dc5;
  for (let i = 0; i < bytes.length; i++) {
    h ^= bytes[i];
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function toBase64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

function fromBase64(text) {
  const s = atob(text);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

function persistSave(force = false) {
  if (!romLoaded || !saveKey) return;
  const len = wasm.dm_save_len();
  if (!len) return;
  const data = memory(wasm.dm_save_ptr(), len);
  const hash = fnv1a(data);
  if (!force && hash === lastSaveHash) return;
  try {
    localStorage.setItem(saveKey, toBase64(data));
    lastSaveHash = hash;
  } catch (err) {
    console.warn('could not store save data', err);
  }
}

function restoreSave() {
  const stored = saveKey && localStorage.getItem(saveKey);
  if (!stored) return false;
  const data = fromBase64(stored);
  writeInput(data);
  const ok = wasm.dm_load_save() === 1;
  if (ok) lastSaveHash = fnv1a(data);
  return ok;
}

// ---------------------------------------------------------------- loading

async function loadRom(bytes, label) {
  if (romLoaded) persistSave(true);
  writeInput(bytes);
  const code = wasm.dm_load_rom();
  if (code !== 0) {
    toast(LOAD_ERRORS[code] || `Could not load ${label}.`, true);
    return false;
  }
  romLoaded = true;
  romBytes = bytes;
  sentMask = 0;
  lastSaveHash = 0;
  saveKey = `dotmatrix:save:${fnv1a(bytes).toString(16)}`;
  const restored = restoreSave();
  const hasBattery = wasm.dm_save_len() > 0;
  const title = cartTitle();
  $('cart-title').textContent = title;
  $('cart-meta').textContent = [
    label,
    `${(bytes.length / 1024) | 0} KiB`,
    hasBattery ? (restored ? 'save restored' : 'battery save') : null,
  ].filter(Boolean).join(' · ');
  document.title = `${title} · dotmatrix`;
  draw();
  return true;
}

async function loadFile(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (await loadRom(bytes, file.name)) start();
}

// ---------------------------------------------------------------- video

function draw() {
  image.data.set(memory(wasm.dm_framebuffer(), 160 * 144 * 4));
  ctx.putImageData(image, 0, 0);
}

function applyPalette(id) {
  const palette = PALETTES[id] || PALETTES.dmg;
  palette.colors.forEach((hex, i) => {
    const n = parseInt(hex.slice(1), 16);
    wasm.dm_set_palette(i, n >> 16, (n >> 8) & 0xff, n & 0xff);
  });
  document.documentElement.style.setProperty('--lcd', palette.colors[0]);
  for (const el of document.querySelectorAll('.swatch')) {
    el.setAttribute('aria-checked', String(el.dataset.palette === id));
  }
  localStorage.setItem('dotmatrix:palette', id);
  if (romLoaded) draw();
}

function buildSwatches() {
  const host = $('palettes');
  host.setAttribute('role', 'radiogroup');
  for (const [id, palette] of Object.entries(PALETTES)) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'swatch';
    b.dataset.palette = id;
    b.setAttribute('role', 'radio');
    const chips = document.createElement('span');
    chips.className = 'chips';
    for (const c of palette.colors) {
      const chip = document.createElement('i');
      chip.style.background = c;
      chips.append(chip);
    }
    b.append(chips, palette.name);
    b.addEventListener('click', () => applyPalette(id));
    host.append(b);
  }
}

// ---------------------------------------------------------------- audio

const audio = { ctx: null, node: null, gain: null, buffered: 0, ready: false, muted: false };

async function startAudio() {
  if (audio.ctx) {
    if (audio.ctx.state === 'suspended') await audio.ctx.resume();
    return;
  }
  try {
    const ac = new AudioContext({ latencyHint: 'interactive' });
    audio.ctx = ac;
    await ac.audioWorklet.addModule('audio-worklet.js');
    audio.node = new AudioWorkletNode(ac, 'dotmatrix-output', { outputChannelCount: [2] });
    audio.gain = ac.createGain();
    audio.node.connect(audio.gain).connect(ac.destination);
    audio.node.port.onmessage = (e) => { audio.buffered = e.data.buffered; };
    wasm.dm_set_sample_rate(ac.sampleRate);
    wasm.dm_audio_clear();
    updateVolume();
    audio.ready = true;
    if (ac.state === 'suspended') await ac.resume();
  } catch (err) {
    console.warn('audio unavailable', err);
    $('audio').textContent = 'n/a';
  }
}

function updateVolume() {
  if (!audio.gain) return;
  const v = audio.muted ? 0 : Number($('volume').value) / 100;
  audio.gain.gain.setTargetAtTime(v * v, audio.ctx.currentTime, 0.02);
}

function pushAudio(discard) {
  const len = wasm.dm_audio_len();
  if (len && audio.ready && !discard) {
    const view = new Float32Array(wasm.memory.buffer, wasm.dm_audio_ptr(), len);
    const copy = view.slice();
    audio.node.port.postMessage(copy, [copy.buffer]);
  }
  wasm.dm_audio_clear();
}

// The emulator follows the wall clock, the sound card follows its own
// crystal. A tiny speed trim keeps the worklet's queue near ~70 ms instead
// of slowly draining or overflowing.
function rateTrim() {
  if (!audio.ready) return 1;
  const target = audio.ctx.sampleRate * 0.07;
  const error = (target - audio.buffered) / target;
  return 1 + Math.max(-1, Math.min(1, error)) * 0.006;
}

// ---------------------------------------------------------------- input

function updateButtons() {
  if (!romLoaded) return;
  let mask = 0;
  const add = (name) => { mask |= 1 << BUTTONS.indexOf(name); };
  held.keys.forEach(add);
  held.pad.forEach(add);
  held.mouse.forEach(add);
  for (const set of held.touch.values()) set.forEach(add);
  // A real d-pad cannot press opposite directions at once.
  if ((mask & 0b11) === 0b11) mask &= ~0b11;
  if ((mask & 0b1100) === 0b1100) mask &= ~0b1100;
  const changed = mask ^ sentMask;
  if (!changed) return;
  for (let i = 0; i < 8; i++) {
    if (changed & (1 << i)) wasm.dm_set_button(i, (mask >> i) & 1);
  }
  sentMask = mask;
  paintPressed(mask);
}

function paintPressed(mask) {
  const on = (name) => (mask >> BUTTONS.indexOf(name)) & 1;
  for (const el of document.querySelectorAll('[data-button]')) {
    el.classList.toggle('pressed', !!on(el.dataset.button));
  }
  for (const dir of ['up', 'down', 'left', 'right']) {
    document.querySelector(`.dpad-arm.${dir}`).classList.toggle('pressed', !!on(dir));
  }
}

function onKey(e, down) {
  if (e.target instanceof HTMLInputElement && e.target.type !== 'range' && e.target.type !== 'checkbox') return;
  const button = KEYMAP[e.code];
  if (button) {
    e.preventDefault();
    if (down) {
      held.keys.add(button);
      if (!running && romLoaded) start();
    } else {
      held.keys.delete(button);
    }
    updateButtons();
    return;
  }
  if (e.code === 'Space') {
    e.preventDefault();
    fastKey = down;
    syncFast();
  } else if (down && e.code === 'KeyP') {
    togglePause();
  } else if (down && e.code === 'KeyM') {
    toggleMute();
  }
}

function dpadDirections(el, x, y) {
  const r = el.getBoundingClientRect();
  const dx = (x - (r.left + r.width / 2)) / (r.width / 2);
  const dy = (y - (r.top + r.height / 2)) / (r.height / 2);
  const dirs = new Set();
  if (Math.hypot(dx, dy) < 0.22) return dirs;
  // Diagonals get a 30 degree slice each, so sliding a thumb around works.
  const angle = (Math.atan2(dy, dx) * 180) / Math.PI;
  if (angle > -75 && angle < 75) dirs.add('right');
  if (angle > 105 || angle < -105) dirs.add('left');
  if (angle > 15 && angle < 165) dirs.add('down');
  if (angle < -15 && angle > -165) dirs.add('up');
  return dirs;
}

function bindPointers() {
  const dpad = $('dpad');
  const track = (el, resolve) => {
    el.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      el.setPointerCapture(e.pointerId);
      held.touch.set(e.pointerId, resolve(e));
      if (!running && romLoaded) start();
      updateButtons();
    });
    el.addEventListener('pointermove', (e) => {
      if (!held.touch.has(e.pointerId)) return;
      held.touch.set(e.pointerId, resolve(e));
      updateButtons();
    });
    const release = (e) => {
      held.touch.delete(e.pointerId);
      updateButtons();
    };
    el.addEventListener('pointerup', release);
    el.addEventListener('pointercancel', release);
    el.addEventListener('lostpointercapture', release);
  };
  track(dpad, (e) => dpadDirections(dpad, e.clientX, e.clientY));
  for (const el of document.querySelectorAll('[data-button]')) {
    track(el, () => new Set([el.dataset.button]));
  }
}

const PAD_BUTTONS = { 0: 'b', 1: 'a', 2: 'b', 3: 'a', 8: 'select', 9: 'start', 12: 'up', 13: 'down', 14: 'left', 15: 'right' };

function pollGamepads() {
  if (!navigator.getGamepads) return;
  const next = new Set();
  let ff = false;
  for (const pad of navigator.getGamepads()) {
    if (!pad || !pad.connected) continue;
    pad.buttons.forEach((b, i) => {
      if (!b.pressed) return;
      if (PAD_BUTTONS[i]) next.add(PAD_BUTTONS[i]);
      if (i === 7 || i === 5) ff = true;
    });
    const [x = 0, y = 0] = pad.axes;
    if (x > 0.5) next.add('right');
    if (x < -0.5) next.add('left');
    if (y > 0.5) next.add('down');
    if (y < -0.5) next.add('up');
  }
  const changed = next.size !== held.pad.size || [...next].some((b) => !held.pad.has(b));
  if (changed) {
    held.pad = next;
    if (next.size && !running && romLoaded) start();
    updateButtons();
  }
  if (ff !== padFast) {
    padFast = ff;
    syncFast();
  }
}
let padFast = false;

// ---------------------------------------------------------------- loop

let lastTime = 0;
let budget = 0;
let statFrames = 0;
let statStart = 0;

function frame(now) {
  requestAnimationFrame(frame);
  pollGamepads();
  const dt = Math.min(now - lastTime, 250);
  lastTime = now;
  if (!running || paused) return;

  const fast = isFast();
  budget += dt * (fast ? FAST_FORWARD : rateTrim());
  let ran = 0;
  const maxFrames = fast ? FAST_FORWARD * 2 : 3;
  while (budget >= FRAME_MS && ran < maxFrames) {
    wasm.dm_run_frame();
    budget -= FRAME_MS;
    ran++;
  }
  if (budget > FRAME_MS * maxFrames) budget = 0; // too far behind: skip ahead
  if (ran) {
    draw();
    pushAudio(fast);
    statFrames += ran;
  }

  if (now - statStart >= 500) {
    const fps = (statFrames * 1000) / (now - statStart);
    $('fps').textContent = `${fps.toFixed(1)} fps`;
    $('speed').textContent = `${((fps / (1000 / FRAME_MS)) * 100).toFixed(0)}%`;
    $('audio').textContent = audio.ready ? `${((audio.buffered / audio.ctx.sampleRate) * 1000).toFixed(0)} ms` : 'off';
    statFrames = 0;
    statStart = now;
    persistSave();
  }
}

function isFast() {
  return fastKey || fastToggle || padFast;
}

function syncFast() {
  $('fast').setAttribute('aria-pressed', String(isFast()));
}

function start() {
  if (!romLoaded) return;
  running = true;
  paused = false;
  $('pause').setAttribute('aria-pressed', 'false');
  $('pause').textContent = 'Pause';
  $('overlay').hidden = true;
  $('led').classList.add('on');
  budget = 0;
  startAudio();
}

function togglePause() {
  if (!running) {
    start();
    return;
  }
  paused = !paused;
  $('pause').setAttribute('aria-pressed', String(paused));
  $('pause').textContent = paused ? 'Resume' : 'Pause';
  $('led').classList.toggle('on', !paused);
  if (paused) persistSave(true);
  if (audio.ctx) paused ? audio.ctx.suspend() : audio.ctx.resume();
}

function toggleMute() {
  audio.muted = !audio.muted;
  $('mute').setAttribute('aria-pressed', String(audio.muted));
  updateVolume();
}

function toast(message, isError = false) {
  const el = $('toast');
  el.textContent = message;
  el.classList.toggle('error', isError);
  el.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => { el.hidden = true; }, 5000);
}

// ---------------------------------------------------------------- wiring

function bindUi() {
  $('load-button').addEventListener('click', () => $('file').click());
  $('file').addEventListener('change', (e) => {
    const [file] = e.target.files;
    if (file) loadFile(file);
    e.target.value = '';
  });
  $('overlay').addEventListener('click', start);
  $('pause').addEventListener('click', togglePause);
  $('fast').addEventListener('click', () => { fastToggle = !fastToggle; syncFast(); });
  $('reset').addEventListener('click', async () => {
    if (!romBytes) return;
    persistSave(true);
    await loadRom(romBytes, $('cart-meta').textContent.split(' · ')[0]);
    start();
  });
  $('mute').addEventListener('click', toggleMute);
  $('volume').addEventListener('input', updateVolume);
  $('grid').addEventListener('change', (e) => {
    $('screen-wrap').classList.toggle('grid', e.target.checked);
    localStorage.setItem('dotmatrix:grid', e.target.checked ? '1' : '0');
  });

  window.addEventListener('keydown', (e) => onKey(e, true));
  window.addEventListener('keyup', (e) => onKey(e, false));
  window.addEventListener('blur', () => {
    held.keys.clear();
    fastKey = false;
    syncFast();
    updateButtons();
  });

  const drop = $('drop');
  let depth = 0;
  window.addEventListener('dragenter', (e) => { e.preventDefault(); depth++; drop.hidden = false; });
  window.addEventListener('dragleave', () => { if (--depth <= 0) { depth = 0; drop.hidden = true; } });
  window.addEventListener('dragover', (e) => e.preventDefault());
  window.addEventListener('drop', (e) => {
    e.preventDefault();
    depth = 0;
    drop.hidden = true;
    const [file] = e.dataTransfer.files;
    if (file) loadFile(file);
  });

  document.addEventListener('visibilitychange', () => {
    if (document.hidden) persistSave(true);
  });
  window.addEventListener('pagehide', () => persistSave(true));
  bindPointers();
}

async function main() {
  buildSwatches();
  const grid = localStorage.getItem('dotmatrix:grid') !== '0';
  $('grid').checked = grid;
  $('screen-wrap').classList.toggle('grid', grid);
  bindUi();
  try {
    wasm = await instantiate();
  } catch (err) {
    $('overlay-title').textContent = 'Could not start the emulator';
    $('overlay-hint').textContent = String(err.message || err);
    return;
  }
  applyPalette(localStorage.getItem('dotmatrix:palette') || 'dmg');
  // A small handle for the console and for the Playwright scripts that
  // record the README media frame by frame.
  window.dotmatrix = {
    wasm,
    loadRom,
    start,
    togglePause,
    step(frames = 1) {
      for (let i = 0; i < frames; i++) wasm.dm_run_frame();
      wasm.dm_audio_clear();
      draw();
    },
    hold(button, down) {
      if (down) held.mouse.add(button);
      else held.mouse.delete(button);
      updateButtons();
    },
  };

  try {
    const response = await fetch(DEMO_ROM);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    await loadRom(new Uint8Array(await response.arrayBuffer()), 'demo cartridge');
    $('overlay-title').textContent = 'Tobu Tobu Girl';
    $('overlay-hint').textContent = 'Click, tap or press a key to play. Or load your own ROM.';
  } catch {
    $('overlay-title').textContent = 'No cartridge';
    $('overlay-hint').textContent = 'Load a .gb file or drop one anywhere on the page.';
  }
  requestAnimationFrame((t) => {
    lastTime = t;
    statStart = t;
    requestAnimationFrame(frame);
  });
}

main();
