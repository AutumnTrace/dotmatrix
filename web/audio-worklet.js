// Plays interleaved stereo samples posted from the main thread.
//
// A plain ring buffer: on overflow the oldest audio is dropped, on underrun
// we output silence. Every few blocks the fill level is reported back so the
// main thread can nudge emulation speed and keep latency steady.

class DotmatrixOutput extends AudioWorkletProcessor {
  constructor() {
    super();
    this.capacity = sampleRate; // one second of stereo frames
    this.left = new Float32Array(this.capacity);
    this.right = new Float32Array(this.capacity);
    this.read = 0;
    this.write = 0;
    this.count = 0;
    this.blocks = 0;
    this.port.onmessage = (e) => this.push(e.data);
  }

  push(samples) {
    for (let i = 0; i + 1 < samples.length; i += 2) {
      if (this.count === this.capacity) {
        this.read = (this.read + 1) % this.capacity;
        this.count--;
      }
      this.left[this.write] = samples[i];
      this.right[this.write] = samples[i + 1];
      this.write = (this.write + 1) % this.capacity;
      this.count++;
    }
  }

  process(_inputs, outputs) {
    const [left, right = left] = outputs[0];
    for (let i = 0; i < left.length; i++) {
      if (this.count > 0) {
        left[i] = this.left[this.read];
        right[i] = this.right[this.read];
        this.read = (this.read + 1) % this.capacity;
        this.count--;
      } else {
        left[i] = 0;
        right[i] = 0;
      }
    }
    if (++this.blocks % 8 === 0) this.port.postMessage({ buffered: this.count });
    return true;
  }
}

registerProcessor('dotmatrix-output', DotmatrixOutput);
