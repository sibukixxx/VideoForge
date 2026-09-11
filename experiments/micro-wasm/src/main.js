import init, { calculate_visual_timeline, validate_video_project } from "../pkg/videoforge_timeline_wasm.js";

const status = document.querySelector("#status");
const output = document.querySelector("#output");

document.querySelector("#run").addEventListener("click", async () => {
  try {
    const started = performance.now();
    await init();
    const initialized = performance.now();
    const [input, expected, project] = await Promise.all([
      fetch("../../fixtures/micro-wasm/visual-placement.input.json").then((response) => response.text()),
      fetch("../../fixtures/micro-wasm/visual-placement.expected.json").then((response) => response.json()),
      fetch("../../fixtures/micro-wasm/project-invalid.input.json").then((response) => response.text()),
    ]);
    const actual = JSON.parse(calculate_visual_timeline(input));
    const matches = JSON.stringify(actual) === JSON.stringify(expected);
    const validation = JSON.parse(validate_video_project(project));
    status.className = matches ? "ok" : "error";
    status.textContent = matches
      ? `PASS — Native scheduling and visual-placement fixture matched (initialization ${(initialized - started).toFixed(3)} ms)`
      : "FAIL — result differs from the shared expected fixture";
    output.textContent = JSON.stringify({ visual_timeline: actual, project_validation: validation }, null, 2);
  } catch (error) {
    status.className = "error";
    status.textContent = `FAIL — ${error}`;
  }
});
