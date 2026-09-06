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

MVP v0.1 の **Core + CLI**（設計書 Phase 1〜6）を実装済み。Tauri GUI（Phase 7）は未着手。

| 機能 | 状態 |
|---|---|
| `videoforge init` / `doctor` / `validate` / `generate` / `export ymm4` / `bundle ymm4` / `speakers` | ✅ |
| Script parser（Front Matter, `話者:` ブロック, 未知 directive は warning） | ✅ |
| VideoProject IR（ms 基準、Workspace 相対パスのみ許可、未知フィールド保持） | ✅ |
| VOICEVOX（audio_query → override → synthesis）、OS キャッシュ、concurrency、cancel | ✅ |
| Timeline / SRT | ✅ |
| FFmpeg preview（背景 + 音声配置 + 字幕 + speaker 名 + fade） | ✅ command builder はテスト済。実レンダリングは要 FFmpeg |
| YMM4 exporter（Template Patch 方式、Windows path materialize、Windows 限定） | ✅ 合成 fixture でテスト済。**実 YMM4 template での Phase 0 検証は未実施** |
| Handoff bundle（dir + zip） | ✅ |
| Tauri GUI | ⏳ |

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

## 台本フォーマット

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

話者名は `videoforge.yaml` の `speakers` キーまたは alias。`@pause` などの directive は v0.1 では warning として無視される。

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

CI 定義（Windows / macOS / Ubuntu matrix + offline smoke）は `docs/ci/github-actions-ci.yml` にある。`.github/workflows/ci.yml` へ移動して有効化する（`docs/ci/README.md` 参照）。

環境変数: `VIDEOFORGE_CACHE_DIR`（TTS cache の場所）、`VIDEOFORGE_FFMPEG`（ffmpeg バイナリ）、`VIDEOFORGE_YMM4_PATH`（YukkuriMovieMaker.exe）。

## Next

1. **Phase 0 spike**: 実 YMM4 で template を作成し、`export ymm4` の出力が YMM4 で開けることを Windows で確認（最大の技術リスク）
2. FFmpeg 実機での preview 確認（日本語フォント指定 `preview.font`）
3. Tauri v2 + React GUI（`apps/desktop`）を Core/CLI の上に被せる
