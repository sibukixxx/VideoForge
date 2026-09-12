# VideoForge

**Markdown 台本 → 音声・字幕・タイムラインを持つ OS 非依存の VideoProject → Windows では YMM4 編集プロジェクト、Windows/macOS ではプレビュー動画** を出力するローカル動画制作コンパイラ。

設計書: [`docs/design/mvp-v0.2-cross-platform.md`](docs/design/mvp-v0.2-cross-platform.md)

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
| FFmpeg preview（背景 + 透過PNG立ち絵 + 音声配置 + 字幕 + speaker 名 + fade） | ✅ `@character`の時間・位置・拡大率・回転・透明度・layerを反映。command builder + filtergraph escapingはテスト済。macOSで基本previewの生成・再生を手動確認済み（2026-09-12） |
| 資料に基づく台本案（`draft prompt` / `check` / `export`） | ✅ P0。外部AIへのAPI接続や自動公開は行わず、根拠付きJSONと人間の承認を必須にする半自動フロー |
| YMM4 exporter（Template Patch 方式、Windows path materialize、Windows 限定） | ✅ 合成 fixture でテスト済。**実 YMM4 template での Phase 0 検証は未実施** |
| Handoff bundle（dir + zip） | ✅ |
| Tauri GUI（`apps/desktop`: Workspace / Script / Validate / Generate + 進捗 / Preview / Doctor / YMM4 export or bundle） | ✅ MVP。実機での起動確認は `apps/desktop/README.md` のチェックリスト |
| Character（VOICEVOX + Live2D）: 話者を character manifest にリンクし、名前ベースで VOICEVOX voice を解決、決定論的な lip-sync データを `character_performance` track として timeline に保持。`videoforge character inspect` / `validate` | ✅ P0（音声+lip-syncデータまで）。フレーム描画・preview.mp4 統合は未実装 — 詳細は `docs/character-system.md` |

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

## 台本を作る

台本には次の2経路があります。

1. `scripts/*.md` を人間またはエージェントが直接作る
2. 資料を `brief.json` にまとめ、`draft` コマンドで根拠付きの台本案を作る

P0の「台本自動生成」は、VideoForge自身がLLM APIを呼ぶ完全自動化ではありません。
`draft prompt`で専用プロンプトを生成し、任意の外部AIで回答JSONを作り、VideoForgeが
構造検査してから人間が承認します。公開・送信・動画生成は自動実行されません。

```bash
# fixtures/draft/brief.json を参考に、Workspace内へ brief.json を用意
videoforge draft prompt brief.json > prompt.txt

# prompt.txt を外部AIへ渡し、JSONだけを response.json として保存
videoforge draft check brief.json response.json

# check結果の review_hash と、確認者名を明示してMarkdown化
videoforge draft export brief.json response.json \
  --reviewed-hash <review_hash> \
  --reviewer takada \
  --out scripts/reviewed-draft.md

videoforge validate scripts/reviewed-draft.md
videoforge generate scripts/reviewed-draft.md
open generated/reviewed-draft/preview.mp4  # macOS
```

`fact`の台詞には資料中に実在する短い引用とsource IDが必要です。ただし、この検査は
引用文字列の存在を確認するだけで、内容の真偽・名誉毀損・著作権・引用の妥当性までは
判定しません。詳しいJSON形式、承認条件、エラー対応は
[`docs/script-draft-p0.md`](docs/script-draft-p0.md) を参照してください。

### Markdown台本フォーマット

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
@bgm assets/bgm/main.mp3[volume=0.6, loop=true]

@image assets/image/chart.png[role=diagram, duration_ms=3000]
@character reimu[expression=happy]
@transition fade[duration_ms=300]
霊夢:
このグラフを見てください。

@se assets/se/pop.wav
魔理沙:
なるほどな。
```

### 透過PNGの立ち絵

背景を透過したPNGを `assets/character/<speaker-key>/default.png` に置き、台詞の直前で
`@character`を指定すると、該当時間だけpreviewへ合成されます。PNGのアルファチャンネルは
保持され、字幕は立ち絵より前面に描画されます。

```markdown
@character zundamon[x=0.78, y=0.56, scale=0.9, opacity=1.0, layer=1]
ずんだもん:
ぼくの透過PNGが右側に表示されるのだ。

@character metan[src=assets/character/metan/talking.png, x=0.22, y=0.56, scale=0.9]
四国めたん:
srcを指定すれば別の表情画像も使えます。
```

`x` / `y` は画面に対する中心位置（0.0〜1.0）、`scale` は倍率、`opacity` は
0.0〜1.0、`rotation_deg` は回転角、`layer` は重なり順です。素材の利用規約と
キャラクターごとのクレジット条件は、配布元で必ず確認してください。

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

CI 定義（Windows / macOS / Ubuntu matrix + offline smoke + desktop build）は `docs/ci/github-actions-ci.yml` にある。`.github/workflows/ci.yml` へ移動して有効化する（`docs/ci/README.md` 参照）。

環境変数: `VIDEOFORGE_CACHE_DIR`（TTS cache の場所）、`VIDEOFORGE_FFMPEG`（ffmpeg バイナリ）、`VIDEOFORGE_YMM4_PATH`（YukkuriMovieMaker.exe）。

## Next

1. **Phase 0 spike**: 実 YMM4 で template を作成し、`export ymm4` の出力が YMM4 で開けることを Windows で確認（最大の技術リスク）
2. 台本案P1: 実AI出力を用いた品質・修正時間・費用の評価（自動公開は対象外）
3. Tauri GUI を Windows / macOS の実機で起動確認（`apps/desktop/README.md` のチェックリスト）
