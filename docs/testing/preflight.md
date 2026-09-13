# Target preflight

`doctor` checks the machine and workspace. `preflight` checks one concrete
script or canonical `project.vfp.json` before synthesis or rendering.

```bash
# Offline structure/asset check. Exit 3 means "warning/unverified", because
# the fake engine cannot prove which VOICEVOX speakers are installed.
videoforge preflight scripts/sample.md --fake-tts

# Production check against the configured local VOICEVOX endpoint.
videoforge preflight scripts/sample.md

# Check an already generated canonical project and every referenced asset.
videoforge preflight generated/sample/project.vfp.json --fake-tts
videoforge --json preflight generated/sample/project.vfp.json --fake-tts
```

## Exit codes

| Code | Meaning |
| ---: | --- |
| 0 | All target checks passed |
| 1 | Invocation or unexpected application error |
| 2 | A definite preflight failure was found |
| 3 | No definite failure, but at least one check is unverified or needs review |

The JSON result contains stable `code`, `status`, `path`, `detail`, and
`remediation` fields. A stopped VOICEVOX connection is a failure in a real
preflight. With `--fake-tts`, it is `tts_unverified`, not a false claim that a
speaker is missing.

## Checks

- script parse, speaker resolution, directives, and referenced source assets;
- live VOICEVOX health and speaker identity before synthesis;
- canonical VideoProject schema and semantic validation;
- all project clip assets and referenced character model/sprites;
- configured preview background and font;
- exact project duration, or a clearly labelled script-duration estimate;
- conservative output size estimate and available output disk space.

FFmpeg binary/filter/encoder checks remain in `videoforge doctor`; run both
commands before a production render. Neither command synthesizes audio or
renders video.

