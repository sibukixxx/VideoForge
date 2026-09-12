# Character & Live2D licensing (design §26)

**No licensing question below has been resolved by legal review.** Every
item is marked `UNKNOWN / NEEDS LEGAL REVIEW` on purpose — this document
records what needs checking and why, not an opinion on the answer. Do not
treat anything here as legal advice or as clearance to redistribute any
third-party asset.

## What VideoForge does today, independent of the answers below

Regardless of how any item resolves, the architecture already enforces the
strictest interpretation:

* **No Live2D model is committed to this repository.** `model.path` in a
  character manifest is a local filesystem path the user supplies; nothing
  in `videoforge-character`, `videoforge-core`, or the CLI ever copies a
  model file into a workspace, into `generated/<slug>/`, or into a release
  artifact (design §6).
* **No VOICEVOX character voice is bundled.** VideoForge talks to a
  locally-running VOICEVOX Engine the user installs themselves; no voice
  model or audio asset ships with VideoForge.
* **The test suite uses a synthetic fixture**, not a real character or
  model — `fixtures/character/mock-character/` is metadata written for this
  repository, matching a real `model3.json`'s field *shape* only, holding no
  meshes, textures, or copyrighted content (design §27).

## Items needing review

### 春日部つむぎ (Kasukabe Tsumugi) usage terms

`UNKNOWN / NEEDS LEGAL REVIEW`

What needs checking: the official usage guidelines for 春日部つむぎ as a
VOICEVOX character (redistribution of generated audio, commercial use of
videos featuring the character, required credit/attribution wording, any
restriction on the *character's likeness* separate from VOICEVOX's own
terms below).

### Live2D model licensing (坂本アヒル氏 / illustrator-specific terms)

`UNKNOWN / NEEDS LEGAL REVIEW`

What needs checking: 春日部つむぎ's official Live2D model is illustrated by
坂本アヒル; illustrator/model-specific terms (separate from the character's
own usage guidelines and from Live2D Cubism's own SDK/runtime license) may
restrict redistribution of the model file itself, derivative works, or
commercial use of rendered output. This is the specific reason model files
are never committed to this repository (see above) — but that mitigation
does not by itself confirm rendered *output* is unrestricted.

### VOICEVOX engine terms

`UNKNOWN / NEEDS LEGAL REVIEW`

What needs checking: VOICEVOX Engine's own license (the engine software
VideoForge talks to over HTTP) versus the terms of any individual character
voice distributed through it — these are commonly separate documents, and
VideoForge integrates with the *engine* generically (`videoforge-voicevox`
has no per-character logic), so the per-character terms are a downstream
concern for whoever configures a specific character, not something this
codebase can resolve generically.

### VOICEVOX 春日部つむぎ voice-specific terms

`UNKNOWN / NEEDS LEGAL REVIEW`

What needs checking: whether 春日部つむぎ's VOICEVOX voice terms differ from
VOICEVOX's baseline terms (attribution requirements, commercial-use
conditions, disallowed content categories).

### Live2D Cubism SDK (Core/Native/Web) license

`UNKNOWN / NEEDS LEGAL REVIEW`

What needs checking: Cubism's own SDK license terms for whichever rendering
approach is eventually built (see `docs/live2d-renderer-decision.md`) —
Cubism Core, Cubism Native, and Cubism Web have historically carried
different (and partly commercial) licensing depending on the licensee's
revenue/scale, which directly affects whether VideoForge can ship *any*
renderer built on them, in what form, and under what conditions.

### Commercial use of the finished video

`UNKNOWN / NEEDS LEGAL REVIEW`

What needs checking: whether a video produced by VideoForge using a
specific character (voice + model) may be used commercially — this compounds
whatever restrictions the character/model/voice terms above carry
individually; VideoForge itself imposes no restriction here, this is
entirely a question of the third-party assets a user chooses to configure.

### Model redistribution

`UNKNOWN / NEEDS LEGAL REVIEW`, though the architectural answer is already
**no**: see "What VideoForge does today" above. This item tracks whether
*any* circumstance (e.g. a user wanting to share their own character
manifest + a pointer to where they got the model) would need additional
guardrails beyond "VideoForge never touches the model file itself."

### Bundling a model with the application

`UNKNOWN / NEEDS LEGAL REVIEW`, with the same architectural answer as
above: VideoForge's releases never include a Live2D model file, full stop
(design §6). This item exists only to record that the question was asked
and answered at the architecture level, not to leave it open.

## What to do if you're extending this feature

* Never commit a real Live2D model, VOICEVOX voice asset, or any third-party
  character asset to this repository, in fixtures or anywhere else — use or
  extend `fixtures/character/mock-character/` instead.
* If a decision above needs to change from `UNKNOWN` to a resolved answer,
  that is a legal-review outcome, not an engineering judgment call — update
  this document with the source of the clearance (a specific terms-of-use
  URL/version, not a summary from memory) rather than removing the
  `NEEDS LEGAL REVIEW` marker based on inference.
