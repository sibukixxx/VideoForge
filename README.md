# VideoForge

**Markdown 台本 → 音声・字幕・タイムラインを持つ OS 非依存の VideoProject → Windows では YMM4 編集プロジェクト、Windows/macOS ではプレビュー動画** を出力するローカル動画制作コンパイラ。

設計書: [`docs/design/mvp-v0.2-cross-platform.md`](docs/design/mvp-v0.2-cross-platform.md)
VideoProject IR v1 仕様: [`docs/video-project.md`](docs/video-project.md)

```text
Claude Code / Codex / 人間
        │  scripts/*.md を書く・CLI を叩く
        ▼
  VideoForge Workspace ── videoforge generate ──▶ generated/<slug>/
                                                   ├── project.vfp.json   ← Single Source of Truth
                                                   ├── captions.srt
                                                   ├── preview.mp4        (FFmpeg)
                                                   ├── manifest.json
                                                   └── assets/audio/*.wav (VOICEVOX)
                                                          │
                                        Windows: export ymm4 ──▶ ymm4/project.ymmp
                                        macOS:   bundle ymm4 ──▶ <slug>-ymm4-bundle.zip → Windows で export
```

## Status

MVP v0.1 の **Core + CLI**（設計書 Phase 1〜6）と Tauri GUI MVP（Phase 7, `apps/desktop`）を実装済み。GUI の実機確認（Windows / macOS）は未実施。

| 機能 | 状態 |
|---|---|
| `videoforge init` / `doctor` / `validate` / `generate` / `export ymm4` / `bundle ymm4` / `speakers` | ✅ |
| Script parser（Front Matter, `話者:` ブロック, `@image` / `@character` / `@bgm` / `@se` / `@transition` directive, 未知 directive は warning） | ✅ |
| VideoProject IR（ms 基準、Workspace 相対パスのみ許可、未知フィールド保持、image / character / bgm / se clip + presentation） | ✅ |
| VOICEVOX（audio_query → override → synthesis）、OS キャッシュ（engine version 込みの key）、concurrency、cancel | ✅ 実 VOICEVOX の統合テストは engine が居る時だけ実行（`docs/testing/voicevox-manual-e2e.md`、macOS 確認済） |
| Timeline / SRT（directive → clip 配置を含む） | ✅ |
| FFmpeg preview（背景 + 音声配置 + 字幕 + speaker 名 + fade + Image/Character立ち絵/Video合成 + crop/fit/rotation/opacity + fade/pan/zoom transition + on-screen Text）(P1-1/P1-2/P1-5) | ✅ command builder + filtergraph escaping はテスト済。実 FFmpeg の統合テストは ffmpeg が見つかった時だけ実行（`docs/testing/ffmpeg-path-escaping.md`）。P1 dogfood（`docs/testing/p1-dogfood-e2e.md`）で背景+立ち絵2体+image(zoom/slide)+video合成を実 FFmpeg で確認し、overlay座標式の未エスケープ、`crop`への無効な`eval=frame`指定の2つのパースエラーを発見・修正済み |
| Audio Engine（Dialogue / Video clip自身の音声 / BGM / SE を `amix` で合成、BGM の volume/loop/trim/fade-in-out/normalize、**Dialogue 発話区間での BGM ducking**）(P1-4) | ✅ command builder テスト済。実 FFmpeg で音声トラック入り動画クリップのミックスをスペクトログラムで位置確認済み。BGMのtrim+adelayの実FFmpegバグも発見・修正済み |
| Subtitle Engine（`preview.subtitle`: position top/bottom・margin・font/outline color・outline width・background box・font_scale、話者別 `caption_color`、字幕と立ち絵の安全領域はどちらの edge でも共有）(P1-3) | ✅ command builder テスト済。P1 dogfoodで話者別色・背景ボックス・font_scaleを実 FFmpeg + 実CJKフォントで確認済み。`position: top` は未検証。行の折返しは固定文字数の hard-wrap（CJK 前提） |
| Render Presets（`youtube-1080p` / `youtube-short` / `preview-low`: 解像度・fps・コーデック・画質・音声ビットレートの一括指定、`videoforge generate --preset`）(P1-6) | ✅ command builder テスト済。P1 dogfoodで `youtube-1080p`/`preview-low` の解像度反映を `ffprobe` で確認済み。ビットレート/画質の主観評価は未実施 |
| Fast Preview（`--range-ms` による出力側 `-ss`/`-t` トリム、`videoforge preview fast` による既存 project.vfp.json からの再レンダリング — パース・検証・TTS・タイムライン構築を全省略）(P1-7) | ✅ command builder / CLI テスト済。P1 dogfoodで実 FFmpeg 実行を確認（45秒レンジ指定 → 実際に45.000秒の出力、フル生成4分25秒に対し27.5秒）。manifest.json への記録なし |
| YMM4 exporter（Template Patch 方式、Windows path materialize、Windows 限定） | ✅ 合成 fixture でテスト済。**実 YMM4 template での Phase 0 検証は未実施** |
| Handoff bundle（dir + zip） | ✅ |
| Tauri GUI（`apps/desktop`: Workspace / Script / Validate / Generate + 進捗 / Preview / Doctor / YMM4 export or bundle） | ✅ MVP。実機での起動確認は `apps/desktop/README.md` のチェックリスト |
| Character（VOICEVOX + Live2D / 3-state PNG）: 話者を character manifest にリンクし、名前ベースで VOICEVOX voice を解決、決定論的な lip-sync データを `character_performance` track として timeline に保持。`model.type: png_lipsync` は closed/half/open の透過 PNG を実際に `preview.mp4` へ合成する（P0-1）。`videoforge character inspect` / `validate` | ✅ Live2D は音声+lip-syncデータまで（フレーム描画は未実装）。PNG は合成まで実装済み — 詳細は `docs/character-system.md` |
| Asset Registry（P0-2）: 生成物が依存するファイルを識別・存在確認・SHA256 ハッシュ化し `generated/<slug>/asset-registry.json` に保存。`videoforge assets <project.vfp.json>` | ✅ 最小実装（巨大な DAM は作らない） |
| Doctor（P0-3）: VOICEVOX / FFmpeg / 出力ディレクトリ書き込み可否 / 空きディスク容量 / 出力設定 / character identity→voice→asset 解決を生成前に確認 | ✅ |

## Quick start

前提: Rust (stable), [VOICEVOX Engine](https://voicevox.hiroshiba.jp/) が `http://127.0.0.1:50021` で起動、FFmpeg が PATH にあること。

```bash
cargo install --path crates/videoforge-cli   # または cargo build --release

videoforge init my-channel
cd my-channel
videoforge doctor                       # VOICEVOX / FFmpeg / platform capability を確認
videoforge speakers                     # speaker_id (VOICEVOX style id) を確認して videoforge.yaml に反映
videoforge validate scripts/sample.md
videoforge generate scripts/sample.md   # → generated/sample/
```

`doctor` はFFmpegの存在だけでなく、preview生成で使うfilterと既定encoderも確認する。
例えばlibfreetypeなしで `drawtext` を持たないFFmpegは、TTS開始前に警告される。

Windows で YMM4 プロジェクトにする:

```bash
videoforge export ymm4 generated/sample/project.vfp.json [--open]
```

macOS では YMM4 が起動できないため handoff bundle を作り、Windows 側で export する:

```bash
videoforge bundle ymm4 generated/sample/project.vfp.json   # → generated/sample-ymm4-bundle(.zip)
# Windows:
videoforge export ymm4 sample-ymm4-bundle/project.vfp.json
```

VOICEVOX / FFmpeg なしで配管だけ試す: `videoforge generate scripts/sample.md --fake-tts --no-preview`

書き出しプリセット（P1-6）と高速プレビュー（P1-7）:

```bash
videoforge generate scripts/sample.md --preset youtube-1080p           # 1920x1080/30fps/crf18
videoforge generate scripts/sample.md --preset preview-low --range-ms 0:15000   # 低解像度 + 範囲指定
videoforge preview fast generated/sample/project.vfp.json --preset preview-low --range-ms 0:15000
```

`--preset` は `youtube-1080p` / `youtube-short` / `preview-low` の3種類（解像度・fps・コーデック・画質・音声ビットレートを一括指定）。
`--range-ms START:END`（ミリ秒）を付けると `preview.fast.mp4` に出力され、増分ビルドキャッシュには参加しない。
`videoforge preview fast <project.vfp.json>` は既存の生成結果からパース・検証・TTS・タイムライン構築を全てスキップして再レンダリングのみ行う。

## 台本フォーマット

資料からAIで台本案を作る半自動P0は
[`docs/script-draft-p0.md`](docs/script-draft-p0.md) を参照。
`draft prompt` → 外部AI → `draft check` → 人間の確認 → `draft export`。
API接続・自動公開は行いません。

```markdown
---
title: 半年前、AIエージェントはまだ苦戦していた
template: yukkuri-tech
---

# 見出しはコメント扱い

霊夢:
半年前までは、最先端のAIでもかなり苦戦していました。

魔理沙:
ところが今は状況がかなり変わっているぜ。
```

話者名は `videoforge.yaml` の `speakers` キーまたは alias。

画像・立ち絵・BGM・効果音は `@` directive で指定する（属性は話者ヘッダと同じ `[key=value, ...]`）。
directive は直後の台詞と一緒に始まり、素材は Workspace 相対パスで `assets/` 配下に置く。

```markdown
@bgm assets/bgm/main.mp3[volume=0.6, loop=true, fade_in_ms=500, fade_out_ms=800, trim_start_ms=0, normalize=true]

@image assets/image/chart.png[role=diagram, duration_ms=3000]
@character reimu[expression=happy]
@transition fade[duration_ms=300]
霊夢:
このグラフを見てください。

@se assets/se/pop.wav
魔理沙:
なるほどな。
```

長さの既定（`duration_ms` 省略時）: `@image` は次の `@image` まで、`@character` は同じ話者の次の立ち絵まで、
`@bgm` は次の `@bgm` まで、`@se` は 1 秒。素材が無い directive は warning になり、その clip だけ飛ばして生成は続く。
`@character <name>` は `assets/character/<話者 key>/<expression>.png`（既定 `default.png`）を探す。
`@pause` などその他の directive は warning として無視される。

## YMM4 template contract

`templates/ymm4/default.ymmp` を YMM4 で一度作成し、以下の備考 (`Remark`) を持つアイテムを置く。Exporter はこれを prototype として複製し、`Text` / `Frame` / `Length` / `FilePath` だけを書き換える。他のプロパティは保持される。

| Remark | 用途 |
|---|---|
| `VF_PROTO_AUDIO` | 音声アイテム |
| `VF_PROTO_CAPTION_<KEY>` / `VF_PROTO_CAPTION` | 話者別 / 汎用の字幕テキスト |
| `VF_PROTO_CHARACTER_<KEY>` | 立ち絵（任意） |
| `VF_PROTO_BACKGROUND` | 背景画像（任意） |

`<KEY>` は speaker キーの大文字（`REIMU`, `MARISA`）。詳細は `fixtures/templates/ymm4/README.md`。

## Repository layout

```text
crates/
├── videoforge-project      Canonical IR (project.vfp.json), RelativeAssetPath, ms→frame
├── videoforge-script       Markdown + Front Matter parser
├── videoforge-timeline     scheduler + SRT
├── videoforge-platform     OS adapter (Finder/Explorer, cache dir, YMM4 detection)
├── videoforge-core         workspace/config/validate/generate pipeline + engine traits
├── videoforge-voicevox     TtsEngine impl (HTTP)
├── videoforge-preview      PreviewRenderer impl (FFmpeg)
├── videoforge-export-ymm4  ProjectExporter impl (.ymmp template patch) + handoff bundle
└── videoforge-cli          `videoforge` binary
apps/desktop/               Tauri v2 + React GUI（src-tauri は独立した cargo workspace）
fixtures/                   sample script, synthetic YMM4 template
docs/design/                設計書
```

Dependency direction: `cli → core ← {voicevox, preview, export-ymm4}`; core は Tauri / Windows API / YMM4 schema に依存しない。

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

GUI: `cd apps/desktop && pnpm install && pnpm tauri dev`（詳細は `apps/desktop/README.md`）。

Micro-Wasm Phase 0（既存timeline schedulerを共有する小規模実験）は
[`docs/architecture/micro-wasm.md`](docs/architecture/micro-wasm.md) を参照。Web版やFFmpeg Wasm化ではない。

CI 定義（Windows / macOS / Ubuntu matrix + offline smoke + desktop build）は `docs/ci/github-actions-ci.yml` に用意済みだが、
`.github/workflows/` への `git mv` がこのセッションの GitHub App トークンには `workflows` 権限がなく拒否されたため、まだ有効化されていない
（手順は `docs/ci/README.md`）。

環境変数: `VIDEOFORGE_CACHE_DIR`（TTS cache の場所）、`VIDEOFORGE_FFMPEG`（ffmpeg バイナリ）、`VIDEOFORGE_YMM4_PATH`（YukkuriMovieMaker.exe）。

## Next

1. **Phase 0 spike**: 実 YMM4 で template を作成し、`export ymm4` の出力が YMM4 で開けることを Windows で確認（最大の技術リスク、Windows 実機が必要）
2. Tauri GUI を Windows / macOS の実機で起動確認（`apps/desktop/README.md` のチェックリスト、実機が必要）
3. 実 VOICEVOX を使った dogfood（今のところ `--fake-tts` の無音での確認のみ — この環境のプロキシ経由でのダウンロードは 403 で拒否された）
4. Live2D フレーム描画（設計は決まっているが未着手 — `docs/character-licensing.md` のライセンス確認が先）
5. CI の有効化: `docs/ci/` から `.github/workflows/` への `git mv`（`workflows` 権限を持つトークン／人が必要 — 詳細は `docs/ci/README.md`）

FFmpeg 実機での preview 確認は `docs/testing/p1-dogfood-e2e.md` で実施済み（Round 1/2 で `overlay` 座標式・
`crop` の `eval=frame` 指定・`bgm_chain` の PTS リセット漏れという3つの実バグを発見・修正、render preset の
比較、動画クリップ自身の音声ミックス機能の検証を含む）。
