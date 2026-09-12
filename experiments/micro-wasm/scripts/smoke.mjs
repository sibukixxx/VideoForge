import { readFile } from "node:fs/promises";
import init, { calculate_timeline, calculate_visual_timeline, validate_video_project } from "../pkg/videoforge_timeline_wasm.js";

const input = await readFile(new URL("../../../fixtures/micro-wasm/simple.input.json", import.meta.url), "utf8");
const expected = JSON.parse(await readFile(new URL("../../../fixtures/micro-wasm/simple.expected.json", import.meta.url), "utf8"));
const wasm = await readFile(new URL("../pkg/videoforge_timeline_wasm_bg.wasm", import.meta.url));
await init(wasm);
const actual = JSON.parse(calculate_timeline(input));
if (JSON.stringify(actual) !== JSON.stringify(expected)) {
  console.error("Wasm result differs from native expected fixture", { expected, actual });
  process.exit(1);
}
console.log("PASS: Wasm result matches the native expected fixture");

const visualInput = await readFile(new URL("../../../fixtures/micro-wasm/visual-placement.input.json", import.meta.url), "utf8");
const visualExpected = JSON.parse(await readFile(new URL("../../../fixtures/micro-wasm/visual-placement.expected.json", import.meta.url), "utf8"));
const visualActual = JSON.parse(calculate_visual_timeline(visualInput));
if (JSON.stringify(visualActual) !== JSON.stringify(visualExpected)) {
  console.error("Wasm visual placement differs from native expected fixture", { visualExpected, visualActual });
  process.exit(1);
}
console.log("PASS: Wasm visual placement matches the native expected fixture");

for (const name of ["project-valid", "project-invalid"]) {
  const project = await readFile(new URL(`../../../fixtures/micro-wasm/${name}.input.json`, import.meta.url), "utf8");
  const projectExpected = JSON.parse(await readFile(new URL(`../../../fixtures/micro-wasm/${name}.expected.json`, import.meta.url), "utf8"));
  const projectActual = JSON.parse(validate_video_project(project));
  if (JSON.stringify(projectActual) !== JSON.stringify(projectExpected)) {
    console.error(`Wasm project validation differs for ${name}`, { projectExpected, projectActual });
    process.exit(1);
  }
}
console.log("PASS: Wasm VideoProject validation matches native expected fixtures");
