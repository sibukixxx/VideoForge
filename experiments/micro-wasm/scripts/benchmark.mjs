import { readFile } from "node:fs/promises";
import { performance } from "node:perf_hooks";
import init, { calculate_timeline, calculate_visual_timeline, validate_video_project } from "../pkg/videoforge_timeline_wasm.js";
import { calculateTimelineJs } from "../src/js-scheduler.js";

const input = await readFile(new URL("../../../fixtures/micro-wasm/multi-clip.input.json", import.meta.url), "utf8");
const wasm = await readFile(new URL("../pkg/videoforge_timeline_wasm_bg.wasm", import.meta.url));
const visualInput = await readFile(new URL("../../../fixtures/micro-wasm/visual-placement.input.json", import.meta.url), "utf8");
const projectInput = await readFile(new URL("../../../fixtures/micro-wasm/project-valid.input.json", import.meta.url), "utf8");
const initStart = performance.now();
await init(wasm);
const initMs = performance.now() - initStart;

function measure(label, fn, iterations) {
  fn();
  const start = performance.now();
  for (let i = 0; i < iterations; i += 1) fn();
  const elapsed = performance.now() - start;
  console.log(`${label.padEnd(5)} ${String(iterations).padStart(5)} calls: ${elapsed.toFixed(3)} ms (${(elapsed * 1e6 / iterations).toFixed(0)} ns/call)`);
}

console.log(`Wasm initialization: ${initMs.toFixed(3)} ms`);
for (const iterations of [1, 100, 1_000, 10_000]) {
  measure("Wasm", () => calculate_timeline(input), iterations);
  measure("JS", () => calculateTimelineJs(input), iterations);
  measure("Visual Wasm", () => calculate_visual_timeline(visualInput), iterations);
  measure("Validate Wasm", () => validate_video_project(projectInput), iterations);
}
