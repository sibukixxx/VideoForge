# FFmpeg path escaping — expectations and cross-platform tests

Issue #12. What VideoForge does with paths that reach FFmpeg, what is tested
automatically, and what must be checked by hand.

## Where paths go

| Path | How it reaches FFmpeg | Escaping |
|---|---|---|
| workspace / project directory | `cwd` of the ffmpeg process | none needed |
| background image, audio WAVs | `-i <relative path>` argument | none needed (not shell, not a filtergraph) |
| `preview.mp4` output | last argument | none needed |
| caption / speaker text files | `drawtext=textfile=…` inside `-filter_complex`, **relative** (`.preview.mp4.tmp/caption-001.txt`) | never contains user characters |
| `preview.font` | `drawtext=fontfile=…` inside `-filter_complex`, may be **absolute** | `quote_filter_value` |

So a workspace path with a space, Japanese, `'`, `:` (Windows drive) or `;` never
enters the filter graph: only the font path does. The escaping rules still have to
be right for that one case, and they are the same rules a future absolute path
would need.

## The two parsing passes

`-filter_complex` is parsed twice by FFmpeg (`ffmpeg-filters(1)`, "Notes on
filtergraph escaping"):

1. **graph pass** — splits on `[ ] , ;` and `=`; `'…'` protects everything, including
   backslashes, until the next `'`.
2. **option pass** — splits the surviving text on `:`; here `\x` means `x`, `'…'`
   quotes again, and unescaped leading/trailing whitespace is trimmed.

`videoforge_preview::quote_filter_value` therefore escapes for pass 2 first
(`\`, `'`, `:` and edge whitespace get a backslash) and then quotes the result for
pass 1 (`'…'`, embedded `'` as `'\''`). Quoting alone — the pre-#12 behaviour —
survives pass 1 but loses every `'` and `:` in pass 2; that was confirmed against
FFmpeg 7.0.2 before the change.

## Expected output per character class

| input | `quote_filter_value` output | notes |
|---|---|---|
| `with space` | `'with space'` | interior whitespace needs nothing |
| `日本語 音声` | `'日本語 音声'` | UTF-8 passes through byte-for-byte |
| `it's` | `'it\'\''s'` | pass 1: close-quote, `\'`, reopen → `it\'s`; pass 2: `\'` → `'` |
| `co:lon` | `'co\:lon'` | pass 2 would otherwise split here |
| `com,ma` `semi;colon` `br[ack]ets` `eq=ual` | unchanged inside `'…'` | pass-1 specials, protected by the quotes |
| `back\slash` | `'back\\slash'` | kept, not rewritten to `/` |
| ` leading` / `trailing ` | `'\ leading'` / `'trailing\ '` | escaped so pass 2 does not trim |
| `C:\Users\霊夢\Fonts\it's.ttf` | `'C\:\\Users\\霊夢\\Fonts\\it\'\''s.ttf'` | Windows drive path |
| `C:/Windows/Fonts/meiryo.ttc` | `'C\:/Windows/Fonts/meiryo.ttc'` | forward slashes are fine on Windows |
| `\\server\share\fonts\a.ttf` | `'\\\\server\\share\\fonts\\a.ttf'` | UNC: only backslashes to escape |

These rows are asserted in `command::tests::filter_value_escaping_per_character_class`.

## Automated tests

`crates/videoforge-preview/tests/ffmpeg_real.rs` runs against a real FFmpeg when
one is found (`$VIDEOFORGE_FFMPEG`, else `ffmpeg` on `PATH`) and prints `skipped:`
otherwise, so `cargo test --workspace` stays offline-safe.

- `escaped_paths_are_accepted_by_the_filtergraph_parser` — one file per class above,
  passed through `quote_filter_value` into `amovie=<path>`; both parsing passes run
  exactly as for `drawtext=fontfile=`. Runs on every FFmpeg build. On Windows the
  temp directory is a drive path, which covers `:` and `\`; `co:lon` and
  `back\slash` file names are Unix-only because Windows forbids those characters.
- `preview_renders_from_a_workspace_with_special_characters` — full
  `generate` (fake TTS + real FFmpeg) in a workspace named
  `my videos [v2]; 霊夢's channel`, with a system font copied to
  `fonts dir; it's [x]/日本語 font.<ext>` and set as `preview.font`. Needs
  `drawtext`; skipped on builds without libfreetype. Set `VIDEOFORGE_TEST_FONT` to
  use a specific font file.

CI (`docs/ci/github-actions-ci.yml`, not yet enabled under `.github/workflows/`) installs FFmpeg
on the Windows, macOS and Ubuntu runners so both tests actually execute on each OS.

Run locally:

```bash
cargo test -p videoforge-preview --test ffmpeg_real -- --nocapture
VIDEOFORGE_FFMPEG=/path/to/ffmpeg cargo test -p videoforge-preview --test ffmpeg_real
```

## Manual checks

Not covered automatically:

| Case | Why manual | Procedure | Result |
|---|---|---|---|
| UNC workspace (`\\server\share\videos`) | runners have no share | `videoforge init` on a share, `generate scripts/sample.md --fake-tts`; FFmpeg's cwd is the UNC dir, nothing is escaped | _pending_ |
| UNC font path | as above | set `preview.font: \\server\share\fonts\x.ttf`, generate; filter graph must contain `fontfile='\\\\server\\share\\fonts\\x.ttf'` | _pending_ |
| `preview.font` with a Japanese file name on Windows | code page / long path interplay in FreeType | copy `meiryo.ttc` to `C:\Users\<you>\フォント\明朝 it's.ttc`, set it, generate | _pending_ |

Fill in the result column with the FFmpeg version and OS build when done.
