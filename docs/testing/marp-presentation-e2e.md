# Marp presentation P0

Issue #59 adds a deliberately small preprocessing boundary:

```text
presentation.md -> external Marp CLI -> slide PNGs
                -> existing Clip::Image -> project.vfp.json
                -> existing FFmpeg preview renderer
```

`project.vfp.json` remains the only canonical video IR. Marp Markdown is an
editable source and the PNG files are materialized assets; neither is a second
timeline model.

## Requirements

- VideoForge built from this repository
- FFmpeg on `PATH`
- Marp CLI on `PATH`, or `VIDEOFORGE_MARP` set to its executable
- Chrome, Edge, or Firefox available to Marp for image conversion

Install Marp yourself. VideoForge never runs `npm install`, `npx`, or downloads
a browser during generation. See the
[official Marp CLI installation guide](https://github.com/marp-team/marp-cli#install).

## CLI

```bash
videoforge presentation prompt scripts/sample.md
videoforge presentation validate presentation.md
videoforge presentation render presentation.md
videoforge preflight scripts/sample.md --presentation presentation.md
videoforge generate scripts/sample.md --presentation presentation.md
```

`presentation validate` checks the required front matter, empty/overfull
slides, local image existence/path safety, and Marp availability. Local image
references are relative to the presentation source and must remain inside the
workspace. Remote URLs, data URIs, absolute paths,
`..`, and symlink escapes are rejected before Marp is invoked.

The adapter invokes the configured executable directly with arguments; it
does not use a shell. `--allow-local-files` is enabled only after the Markdown
image paths have passed VideoForge validation. Keep presentation Markdown
reviewed and trusted.

## Fake TTS dogfood (six slides, about 30–60 seconds)

Create a disposable workspace and copy the synthetic fixtures:

```bash
videoforge init /tmp/videoforge-marp-dogfood
cd /tmp/videoforge-marp-dogfood

cp /path/to/VideoForge/fixtures/presentation/script.md scripts/marp-dogfood.md
cp /path/to/VideoForge/fixtures/presentation/presentation.md presentation.md
mkdir -p assets/image characters/sprites
cp /path/to/VideoForge/fixtures/presentation/presentation-flow.svg \
  assets/image/presentation-flow.svg
cp /path/to/VideoForge/fixtures/character/mock-png-character/manifest.yaml \
  characters/manifest.yaml
cp /path/to/VideoForge/fixtures/character/mock-png-character/sprites/{closed,half,open}.png \
  characters/sprites/
```

Set these fields in `videoforge.yaml` (keep the existing video/preview
settings):

```yaml
character_manifest: characters/manifest.yaml
speakers:
  mock_a:
    character_id: mock_a
  mock_b:
    character_id: mock_b
```

Then run:

```bash
videoforge presentation prompt scripts/marp-dogfood.md > /tmp/marp-prompt.txt
videoforge presentation validate presentation.md
videoforge preflight scripts/marp-dogfood.md \
  --presentation presentation.md --fake-tts
videoforge generate scripts/marp-dogfood.md \
  --presentation presentation.md --fake-tts

ffprobe -v error -show_entries stream=width,height \
  -show_entries format=duration -of default=nw=1 \
  generated/marp-dogfood/preview.mp4
jq '.tracks[] | select(.id == "presentation")' \
  generated/marp-dogfood/project.vfp.json
open generated/marp-dogfood/preview.mp4
```

The fake engine produces silence, so visual inspection proves timing,
captions, slide changes, character placement, and three-state sprite
composition—not audible speech. Repeat without `--fake-tts` for the real
VOICEVOX narration check.

Expected:

- six `presentation/slide-NNN.png` files at 1920x1080
- six existing `type: image` clips on an `image` track
- each image start/duration equals its corresponding audio clip
- subtitles render above the slides
- the synthetic left/right PNG characters remain above the slides
- `preview.mp4` exists and changes slide at each narration boundary

P0 rejects a slide/dialogue count mismatch. It never stretches, drops, or
duplicates a slide silently.

## Scope

P0 does not add an LLM API, PresentationPlan JSON, scene system, slide editor,
PDF/Excel analysis, image generation, effects, Slidev, or reveal.js.
