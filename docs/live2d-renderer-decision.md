# ADR: Live2D character rendering strategy

Status: **decision made for the direction; not yet implemented.** P0 (this
change) stops at deterministic lip-sync *data* and Live2D model *metadata*
validation (`docs/character-system.md`). This document is the required
design-doc-cited research from design §14/§18, so whoever picks up Phase 1
frame rendering starts from a decision, not a blank page.

## Problem

VideoForge is a Rust CLI/Tauri application. There is no maintained, license-clean
Rust binding for the Cubism Core native runtime, and Live2D's own SDKs are
JS/WebGL (Cubism Web) or C++ (Cubism Native, proprietary, per-platform
licensing). We need a way to turn a Live2D model + a deterministic lip-sync
curve + an expression/motion into a sequence of transparent RGBA frames that
`videoforge-preview`'s existing FFmpeg pipeline can overlay onto a
background, without becoming a Live2D SDK maintainer ourselves.

## Options considered

### Option A — Headless browser / WebView driving Cubism Web

Load Live2D's official Cubism Web SDK in a headless browser (Playwright/
Puppeteer-style Chromium) or, on desktop, the Tauri app's own WebView;
script it to load the model, apply the lip-sync/expression/motion track
frame-by-frame, and capture each frame as a PNG (canvas `toDataURL`/
`toBlob`, or a screenshot API).

* **macOS/Windows compatibility:** good — Chromium runs on both; Tauri's
  WebView is native per-OS (WebKit/WebView2) but Cubism Web is standard
  WebGL/Canvas2D content, portable across both.
* **Deterministic rendering:** good, *if* driven by explicit per-frame state
  (seek to a frame index, not "advance one frame of real time") rather than
  by `requestAnimationFrame`-paced playback — must be verified against the
  actual Cubism Web API, not assumed.
* **Transparent background:** supported — Cubism Web renders to a WebGL
  canvas with an alpha channel; capturing that as PNG preserves it.
* **Performance:** the weakest point — a full browser process per render,
  screenshot-per-frame at typical video frame rates (24–60fps) is slow
  compared to a native renderer; acceptable for short demo clips, a real
  bottleneck for long videos without frame caching/batching.
* **Implementation complexity:** medium — no native bindings to write, but a
  browser-automation dependency (a new external tool requirement, akin to
  FFmpeg/VOICEVOX) and a driver script/page to build and maintain.
* **Live2D license:** Cubism Web SDK's license terms need the same
  UNKNOWN/NEEDS LEGAL REVIEW treatment as everything else in
  `docs/character-licensing.md` — not resolved here.
* **CI testability:** good — a headless Chromium is already a common CI
  dependency (this repo's manual-e2e docs already assume a browser-capable
  environment is possible); the mock character fixture has no real model to
  render, so CI can only exercise the *pipeline*, not real Cubism output,
  either way.

### Option B — Transparent frame sequence via a native/CLI Live2D renderer

Shell out to a separate, purpose-built renderer binary (Cubism Native SDK
compiled to a small CLI tool, or a third-party equivalent) that takes a
model + a per-frame parameter track and writes RGBA PNGs directly, no
browser involved.

* **macOS/Windows compatibility:** depends entirely on the Cubism Native SDK
  build story per platform (it is a C++ SDK requiring per-OS builds and,
  per Live2D's licensing, generally proprietary/commercial terms beyond a
  free tier) — the biggest unknown of the three options.
* **Deterministic rendering:** good — same argument as Option A, this is
  fundamentally more controllable since there's no browser event loop.
* **Transparent background:** supported by the native renderer.
* **Performance:** best of the three — no browser overhead, direct
  rasterization.
* **Implementation complexity:** highest — writing and maintaining a
  from-scratch native renderer binary (even a thin wrapper around Cubism
  Native) is a substantial, ongoing engineering commitment, squarely a "P1
  build a small SDK" project rather than "call an existing tool" the way
  FFmpeg/VOICEVOX are called today.
* **Live2D license:** Cubism Native SDK licensing is commercial-leaning and
  needs explicit legal review before any redistribution of a compiled
  renderer — see `docs/character-licensing.md`.
* **CI testability:** poor without a CI-installable renderer binary; would
  need its own "skip if unavailable" pattern like
  `crates/videoforge-preview/tests/ffmpeg_real.rs`.

### Option C — Character runtime as a separate long-lived process

Run a persistent renderer process (could itself be Option A or B
internally) that VideoForge talks to over a small protocol (stdio/IPC),
requesting "render character X, expression Y, motion Z, lip-sync value V at
time T" and receiving a frame back, similar in spirit to how
`videoforge-voicevox` talks to the VOICEVOX Engine as a separate process.

* **macOS/Windows compatibility:** same as whatever backs it (A or B); this
  option is really an *architectural* choice orthogonal to the rendering
  technology, not a fourth rendering method.
* **Deterministic rendering:** good, and arguably the cleanest place to
  enforce it — a stateless request/response protocol makes "given the same
  request, the same frame" easy to reason about and test against a fake/
  mock renderer process, exactly like `FakeTtsEngine` today.
* **Transparent background:** inherits from the backing technology.
* **Performance:** avoids per-frame process-spawn overhead (the process
  stays warm across a whole generate run) at the cost of process-lifecycle
  complexity (start/health-check/shutdown, akin to `TtsEngine::health()`).
* **Implementation complexity:** adds a protocol layer on top of whichever
  of A/B backs it — more moving parts, but each part (the trait boundary,
  the process management) mirrors patterns this codebase already has for
  VOICEVOX and FFmpeg.
* **Live2D license:** inherits from the backing technology.
* **CI testability:** best of the three for the *architecture* (a fake
  renderer process, or an in-process fake implementing the same
  `CharacterRenderer` trait, tests the protocol/timeline-integration without
  any real Live2D asset) even though real-model rendering still needs the
  real backing technology to test end-to-end.

## Decision

**Direction for Phase 1: Option A (headless browser / WebView) behind the
process-boundary shape of Option C**, i.e. a `CharacterRenderer` trait
(mirroring `TtsEngine`/`PreviewRenderer`) implemented by a crate that drives
Cubism Web in a headless/embedded browser context, producing one RGBA PNG
per requested `(character, expression, motion, mouth_open, frame_index)`
tuple, consumed by `videoforge-preview`'s existing FFmpeg overlay step
(design §16, §17) — never the other way around: the character renderer
never invokes FFmpeg itself, matching the "separate Renderer and Compositor"
requirement in design §16.

Rationale: Option B's native-SDK path is the highest-value long-term
outcome (performance, no browser dependency) but represents a
multi-week SDK-integration project with unresolved licensing terms; starting
there risks the whole character feature on an unresolved legal question.
Option A reuses infrastructure this repository is already comfortable with
(a browser is already assumed reachable per the manual-e2e docs; Tauri's own
WebView is a natural home for it on desktop) and gets to a real, visible
result — a rendered, lip-synced character frame sequence — fastest. It does
not foreclose Option B later: the `CharacterRenderer` trait boundary means a
native renderer can be swapped in as a second implementation exactly the way
`videoforge-voicevox` and a hypothetical second `TtsEngine` would coexist.

Frame storage starts as a plain PNG-sequence directory
(`assets/character/<id>/frames/frame-<N>.png`, design §17) for P0-of-Phase-1
simplicity; the module doc should carry a note that pipe/stream-based frame
delivery is the intended evolution once disk usage on longer videos becomes
a real problem, per design §17's explicit deferral.

## Not decided here

* Which specific headless-browser tooling (Playwright vs. a raw CDP client
  vs. Tauri's own WebView capture APIs) — needs a spike against the actual
  Cubism Web SDK's frame-seeking API before committing.
* The Live2D/Cubism licensing question in every option above — tracked in
  `docs/character-licensing.md`, marked UNKNOWN/NEEDS LEGAL REVIEW, and a
  hard prerequisite before shipping *any* renderer, not just before shipping
  a specific one.
* Whether the headless browser dependency should be bundled, downloaded on
  first use, or required as a user-provided prerequisite like VOICEVOX/
  FFmpeg already are.
