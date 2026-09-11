# Micro-Wasm experiment (Phase 0)

## 1. Why micro-Wasm

VideoForge already has Rust calculations that may later be useful in a browser. Phase 0 tests one
small hot path instead of treating WebAssembly as a way to port the desktop application. The
inspiration is Kanryu Kato's Qiita article about small, focused Wasm modules; its Hike measurements
are not assumed to apply to this Rust codebase.

The decision question is not merely whether the code compiles to Wasm. It is whether sharing an
existing Rust rule is worth download, initialization, and JSON boundary costs.

## 2. Why not full FFmpeg Wasm

FFmpeg, VOICEVOX, filesystem access, process spawning, networking, Tauri, UI, and Node APIs remain
outside the module. `ffmpeg.wasm` would add a large runtime and solve a different problem. Preview
rendering continues to derive from the canonical `project.vfp.json` through the native FFmpeg
adapter.

## 3. Current VideoForge architecture

`project.vfp.json` remains the canonical VideoProject IR and single source of truth. The existing
generation path is unchanged:

```text
script -> validation -> TTS -> timeline -> project.vfp.json -> SRT / preview / YMM4
```

The dependency survey found:

| Area | Classification | Wasm suitability |
|---|---|---|
| `videoforge-project` time conversion and value validation | Pure calculation | P1 candidate |
| `videoforge-timeline::schedule` | Pure calculation | **P0 selected** |
| visual clip placement | Pure calculation, coupled to full project construction | P1 candidate |
| SRT formatting | Pure text generation from VideoProject | P1 candidate |
| workspace, manifest, cache | Filesystem | Excluded |
| preview renderer | Process + FFmpeg + filesystem | Excluded |
| VOICEVOX engine | Network + audio files | Excluded |
| YMM4 export/bundle | Filesystem + Windows/editor contract | Excluded |
| desktop commands | Tauri + OS composition root | Excluded |

## 4. Selected Phase 0 hot path

Phase 0 exposes dialogue timeline scheduling: ordered dialogue durations plus a gap become stable
start/end times. This is already the pure `videoforge_timeline::schedule` function used by native
project generation. No scheduling algorithm was copied into the Wasm crate.

## 5. Native/Wasm shared-core architecture

```text
videoforge-timeline::schedule
        |                 |
 native generation   JSON/Wasm adapter
                          |
                    browser or Worker
```

`videoforge-timeline-wasm` is a `cdylib`/`rlib`. The native-testable
`calculate_timeline_json` function performs boundary conversion and delegates to `schedule`.
Only a `#[wasm_bindgen]` function is target-gated for `wasm32`.

## 6. Wasm boundary

Input is a small JSON object containing `dialogue_gap_ms` and dialogue metadata (`index`, speaker,
display name, text, workspace-relative audio path, duration). Output contains scheduled dialogue
metadata and `total_duration_ms`. Paths are validated with the existing `RelativeAssetPath` type.

The boundary deliberately contains no complete filesystem object and does not mutate a
VideoProject. The canonical IR schema is unchanged. JSON was chosen for a transparent Phase 0
contract; a typed binary boundary should only be considered after profiling demonstrates that
serialization is material.

## 7. Benchmark

The experiment measures the complete public call, including JSON decode/encode. This prevents a
misleading comparison that hides boundary overhead.

```bash
cargo run -p videoforge-timeline-wasm --release --example native_benchmark
cd experiments/micro-wasm
npm run build
npm run benchmark
```

Each runtime is measured at 1, 100, 1,000, and 10,000 calls. Wasm initialization is reported
separately. The GitHub Actions job writes the current machine-dependent numbers to its job summary;
numbers should not be treated as stable product guarantees. For this small JSON call, JS is expected
to be competitive and the Wasm boundary may dominate. Phase 0 therefore validates code sharing
first, not a speed claim.

## 8. Binary size

After `npm run build`, run `npm run size`. It records raw, gzip, and Brotli byte counts for the
release `.wasm`. CI records these in the same job summary as performance. No unsafe or opaque size
optimization is enabled merely to win the experiment.

## 9. Browser test

```bash
cd experiments/micro-wasm
npm run build
npm run smoke
npm run serve
# open http://localhost:4173/experiments/micro-wasm/
```

The page loads the Simple fixture, executes Wasm, compares with the expected result shared by the
native test, and prints the JSON. `npm run smoke` performs the same parity check headlessly in CI.
The fixture set covers Simple, Multi Clip, and Edge Case (1 ms, zero gap, Japanese and Unicode).

## 10. Limitations

- This is not a Web VideoForge implementation.
- The boundary benchmarks serialization as well as calculation; it does not isolate raw arithmetic.
- The sample scheduler is small, so Wasm is unlikely to improve a single call's latency.
- Browser engines and CI hardware differ, so measurements must be repeated on target devices.
- The harness does not render media and intentionally has no FFmpeg or VOICEVOX integration.

## 11. Candidate P1/P2 modules

| Priority | Candidate | Reason / gate |
|---|---|---|
| P0 | Dialogue timeline scheduling | Safest existing pure function; shared today |
| P1 | Batch visual clip placement | Pure, browser-useful, enough work per boundary crossing |
| P1 | VideoProject validation | Preserves one rule set across desktop/browser; design a structured error contract first |
| P1 | Subtitle timing/layout preparation | Browser preview use case; keep SRT file output outside Wasm |
| P2 | Waveform analysis | Strong compute/Worker fit, but requires a stable PCM input contract and benchmarks |
| P2 | Image dimension/layout calculation | Useful for preview; define pixel/fit semantics first |
| P2 | Media metadata parsing | Binary-input boundary and format scope need separate evaluation |
| P2 | Text normalization | Likely cheaper in JS unless native/browser rule consistency is the main value |

Hike and TinyGo remain possible comparison experiments only; neither is a production dependency.

## 12. Web VideoForge implications

The experiment supports a future split where both runtimes consume or produce the same VideoProject
IR, while native adapters use FFmpeg and browser adapters use Web APIs. Pure calculation crates can
sit below both runtimes. Heavy, batchable waveform or image operations are good Web Worker
candidates. Timeline scheduling is fast and can remain on the main thread unless profiling a large
project shows visible blocking; Worker startup and message serialization would otherwise add more
overhead than they remove.

Phase 0's likely value is eliminating duplicate business rules, not making two-dialogue scheduling
faster. Continue to P1 only for a browser-facing need or a larger batch hot path where sharing and/or
compute gains outweigh bundle and boundary costs.
