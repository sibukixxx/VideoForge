# VideoForge MVP 設計書 v0.2
**Status:** Implementation Ready  
**Target:** Windows 10/11 + macOS 14+  
**Desktop:** Tauri v2 + React + TypeScript  
**Core / CLI:** Rust  
**TTS:** VOICEVOX Engine  
**Primary editable export:** YukkuriMovieMaker4 (`.ymmp`, Windows only)  
**Cross-platform canonical format:** VideoForge Project IR (`project.vfp.json`)

---

# 0. 設計変更の要点

前版の「Tauri GUI → YMM4プロジェクト生成」中心の構成から、以下へ変更する。

> **Workspace-first + CLI-first + Cross-platform Core**

VideoForge自身はAIエージェントを内蔵しない。Claude Code / Codex / ChatGPTなどの任意のエージェントは、VideoForge Workspace上のファイルを編集し、CLIを呼び出す。

```text
Claude Code / Codex / Other Agent
              │
              ▼
       VideoForge Workspace
              │
      script.md / assets
              │
              ▼
        VideoForge Core
       ┌──────┴──────┐
       │             │
     Tauri           CLI
       │             │
       └──────┬──────┘
              ▼
       VideoProject IR
       project.vfp.json
          │       │
          │       ├──────────────┐
          ▼                      ▼
   FFmpeg Preview          YMM4 Exporter
   Windows / macOS          Windows materialize
          │                      │
          ▼                      ▼
     preview.mp4             project.ymmp
```

最大の変更点は、**YMM4をVideoForgeの内部モデルにしない**こと。

YMM4はWindows専用であり、Macでは起動できない。そのため `.ymmp` はあくまでExporterの一つとして扱う。

VideoForge本体とVideoProject IRはWindows/macOS共通とする。

---

# 1. MVPの目的

台本を渡すだけで、以下を自動生成する。

1. 台本解析
2. VOICEVOXによる音声生成
3. WAV実尺取得
4. 字幕タイミング生成
5. VideoProject IR生成
6. 簡易プレビュー生成
7. WindowsではYMM4編集可能プロジェクト生成

MVPでは「完全自動投稿ツール」は作らない。

目的は、

> **AIまたは人間が書いた台本を、編集可能な動画プロジェクトへコンパイルするローカルツール**

を成立させること。

---

# 2. MVP成功条件

## 共通 — Windows / macOS

以下が成立すればVideoForge Coreとして成功。

```text
videoforge init
↓
scripts/sample.md を編集
↓
videoforge generate scripts/sample.md
↓
VOICEVOX音声生成
↓
project.vfp.json生成
↓
字幕タイムライン生成
↓
preview.mp4生成
```

生成物：

```text
generated/sample/
├── project.vfp.json
├── manifest.json
├── preview.mp4
├── captions.srt
└── assets/
    └── audio/
        ├── 001.wav
        ├── 002.wav
        └── 003.wav
```

## Windows

さらに、

```text
videoforge export ymm4 generated/sample/project.vfp.json
```

で、

```text
generated/sample/ymm4/project.ymmp
```

を生成し、YMM4で正常に開けること。

## macOS

MacではYMM4そのものは起動できないため、

- 台本編集
- 音声生成
- タイムライン生成
- preview.mp4確認
- VideoProject IR編集
- Windows向けhandoff bundle生成

までをサポートする。

---

# 3. プラットフォーム方針

| 機能 | Windows | macOS |
|---|---:|---:|
| Tauri GUI | ◎ | ◎ |
| CLI | ◎ | ◎ |
| Workspace | ◎ | ◎ |
| VOICEVOX CPU | ◎ | ◎ |
| VOICEVOX GPU | 環境依存 | × |
| Script parse | ◎ | ◎ |
| Audio生成 | ◎ | ◎ |
| Timeline生成 | ◎ | ◎ |
| SRT生成 | ◎ | ◎ |
| Preview MP4 | ◎ | ◎ |
| VideoProject IR | ◎ | ◎ |
| `.ymmp` materialize | ◎ | △ |
| YMM4起動 | ◎ | × |
| YMM4編集 | ◎ | × |
| YMM4 handoff bundle | ◎ | ◎ |

`△`:
MacでもJSONとして `.ymmp` を組み立てること自体は可能だが、YMM4をMac上で検証・起動できず、素材パスもWindows形式になるため、MVPでは**Windows materializeを正式経路**とする。

---

# 4. macOS対応の設計原則

## 4.1 内部パスにWindows絶対パスを保存しない

禁止：

```text
C:\Users\xxx\VideoForge\assets\001.wav
```

Canonical IRでは必ずWorkspace相対パス。

```json
{
  "audio": "assets/audio/001.wav"
}
```

Exporterが実行時にOSネイティブパスへ解決する。

---

## 4.2 OS固有処理をPlatform Adapterへ隔離

```rust
pub trait Platform {
    fn reveal_in_file_manager(&self, path: &Path) -> Result<()>;
    fn open_file(&self, path: &Path) -> Result<()>;
    fn app_data_dir(&self) -> Result<PathBuf>;
}
```

実装：

```text
WindowsPlatform
MacPlatform
```

Windows：

```text
Explorer
YukkuriMovieMaker.exe
```

macOS：

```text
Finder
open
```

---

## 4.3 YMM4依存をCoreに入れない

```text
videoforge-core
    │
    ├── script
    ├── tts
    ├── timeline
    ├── project-ir
    └── render-preview

exporters
    └── ymm4
```

YMM4 schemaやWindows path処理がCoreへ漏れないこと。

---

# 5. Workspace設計

プロジェクトの実体はフォルダ。

```text
my-channel/
├── videoforge.yaml
├── AGENTS.md
│
├── channel/
│   ├── identity.md
│   └── plan.md
│
├── research/
│
├── scripts/
│   ├── 001-ai-news.md
│   └── 002-rust.md
│
├── assets/
│   ├── character/
│   ├── background/
│   ├── image/
│   ├── bgm/
│   └── se/
│
├── templates/
│   └── ymm4/
│       └── default.ymmp
│
└── generated/
```

VideoForgeはWorkspace外の絶対パスへの依存を極力避ける。

---

# 6. Agent連携

MVPでは専用AI SDKを組み込まない。

AIエージェントとの契約は、

1. File
2. CLI

だけ。

## AGENTS.md

`videoforge init` 時に生成。

例：

```markdown
# VideoForge Agent Instructions

動画を生成するとき：

1. scripts/ にMarkdown台本を作成する
2. `videoforge validate scripts/<file>.md` を実行する
3. エラーがなければ `videoforge generate scripts/<file>.md` を実行する
4. generated/<slug>/manifest.json を確認する
5. 必要なら台本を修正して再生成する

WindowsでYMM4プロジェクトが必要な場合：

`videoforge export ymm4 generated/<slug>/project.vfp.json`
```

Claude Code / CodexなどはCLIを通常の開発ツールとして利用する。

---

# 7. CLI仕様

## 7.1 init

```bash
videoforge init my-channel
```

生成：

```text
my-channel/
├── videoforge.yaml
├── AGENTS.md
├── scripts/
├── assets/
├── templates/
└── generated/
```

---

## 7.2 doctor

```bash
videoforge doctor
```

確認項目：

```text
✓ Workspace
✓ VOICEVOX connection
✓ FFmpeg
✓ Output directory
✓ Template
✓ Platform

Platform: macOS arm64
YMM4: unavailable on this platform
```

MacでYMM4がないことはエラーではなくCapabilityとして扱う。

---

## 7.3 validate

```bash
videoforge validate scripts/001.md
```

確認：

- YAML Front Matter
- speaker
- text
- voice mapping
- asset reference
- unsupported directive

---

## 7.4 generate

```bash
videoforge generate scripts/001.md
```

実行：

```text
Parse
↓
TTS
↓
Duration
↓
Timeline
↓
Project IR
↓
SRT
↓
Preview
```

---

## 7.5 export

```bash
videoforge export ymm4 generated/001/project.vfp.json
```

Windows：

```text
project.ymmp生成
```

macOS：

```text
Error:
YMM4 materialization requires Windows.

Use:
videoforge bundle ymm4 generated/001/project.vfp.json
```

---

## 7.6 bundle

```bash
videoforge bundle ymm4 generated/001/project.vfp.json
```

MacからWindowsへ渡せるportable bundleを生成。

```text
001-ymm4-bundle/
├── project.vfp.json
├── template.ymmp
├── manifest.json
├── source.md
└── assets/
    └── audio/
```

Windows側：

```bash
videoforge export ymm4 001-ymm4-bundle/project.vfp.json
```

素材パスをWindows環境に合わせて再解決し `.ymmp` を生成する。

---

# 8. 台本フォーマット

MVPはMarkdown + YAML Front Matter。

```markdown
---
title: 半年前、AIエージェントはまだ苦戦していた
template: yukkuri-tech
---

霊夢:
半年前までは、最先端のAIでもかなり苦戦していました。

魔理沙:
ところが今は状況がかなり変わっているぜ。

霊夢:
今日は何が変わったのか整理してみます。
```

---

# 9. Script DSL

MVP v0.1では最小限。

```text
Speaker:
Text
```

対応：

```markdown
霊夢:
こんにちは。

魔理沙:
今日はAIについて解説するぜ。
```

## コメント

```markdown
# これはMarkdown見出しとして扱う
```

## 将来の拡張

```markdown
魔理沙[emotion=surprise]:
これはかなりヤバいぜ。

@pause 500ms

@image src="assets/image/chart.png"

@human id="personal-experience"
ここに本人の経験を書く
@end
```

v0.1 parserは未知directiveをエラーではなくwarningとして扱える設計にする。

---

# 10. videoforge.yaml

```yaml
version: 1

project:
  name: ai-news-channel

video:
  width: 1920
  height: 1080
  fps: 30

timeline:
  dialogue_gap_ms: 200

tts:
  engine: voicevox
  endpoint: http://127.0.0.1:50021
  concurrency: 1

speakers:
  reimu:
    aliases:
      - 霊夢
    voice:
      speaker_id: 2
      speed_scale: 1.05
      pitch_scale: 0.0
      intonation_scale: 1.0

  marisa:
    aliases:
      - 魔理沙
    voice:
      speaker_id: 3
      speed_scale: 1.08
      pitch_scale: 0.0
      intonation_scale: 1.0

preview:
  enabled: true
  background: assets/background/default.png

export:
  ymm4:
    template: templates/ymm4/default.ymmp
```

---

# 11. Canonical VideoProject IR

ファイル：

```text
project.vfp.json
```

このファイルがVideoForgeにおけるSingle Source of Truth。

`.ymmp` は派生物。

---

## 11.1 Project

```rust
pub struct VideoProject {
    pub schema_version: u32,

    pub id: String,
    pub title: String,

    pub video: VideoSettings,

    pub source: SourceInfo,

    pub tracks: Vec<Track>,
}
```

---

## 11.2 Track

```rust
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
}
```

```rust
pub enum TrackKind {
    Audio,
    Caption,
    Character,
    Image,
    Background,
    SoundEffect,
}
```

---

## 11.3 AudioClip

```rust
pub struct AudioClip {
    pub id: String,

    pub source: RelativeAssetPath,

    pub start_ms: u64,
    pub duration_ms: u64,

    pub speaker: String,
}
```

---

## 11.4 CaptionClip

```rust
pub struct CaptionClip {
    pub id: String,

    pub text: String,

    pub start_ms: u64,
    pub duration_ms: u64,

    pub speaker: String,
}
```

時間はframeではなく`ms`をCanonical値にする。

Exporterでframeへ変換する。

理由：

- FPS変更に強い
- 音声実尺との対応が容易
- FCPXML等への将来展開が容易

---

# 12. Path Model

重要。

```rust
pub struct RelativeAssetPath(PathBuf);
```

検証：

```text
OK
assets/audio/001.wav

NG
C:\data\001.wav

NG
/Users/foo/data/001.wav
```

Canonical IRにはWorkspace外の絶対パスを書かない。

---

# 13. VOICEVOX Adapter

```rust
#[async_trait]
pub trait TtsEngine {
    async fn list_speakers(&self) -> Result<Vec<Speaker>>;

    async fn synthesize(
        &self,
        request: TtsRequest,
    ) -> Result<SynthesizedAudio>;
}
```

実装：

```text
VoicevoxEngine
```

---

# 14. VOICEVOXフロー

```text
Dialogue
↓
audio_query
↓
parameter override
↓
synthesis
↓
WAV
↓
duration取得
↓
TTS Cache
```

デフォルト：

```text
endpoint:
http://127.0.0.1:50021
```

---

# 15. MacでのVOICEVOX

macOSではCPU版を利用する。

VideoForge側ではOSを見てVOICEVOX起動方式を変えるのではなく、HTTP endpointに接続するだけとする。

```text
VideoForge
    │ HTTP
    ▼
VOICEVOX Engine
```

そのためWindows/macOSでCore実装を共通化できる。

MVPのデフォルトTTS concurrencyは`1`。

設定で変更可能。

---

# 16. TTS Cache

Key：

```text
SHA256(
  engine +
  speaker_id +
  text +
  speed +
  pitch +
  intonation
)
```

保存：

Windows：

```text
%LOCALAPPDATA%\VideoForge\cache\tts\
```

macOS：

```text
~/Library/Caches/VideoForge/tts/
```

Workspaceへキャッシュをコミットしない。

---

# 17. Timeline Builder

基本：

```text
Audio duration = WAV実尺
Caption duration = Audio duration
Gap = configuration
```

例：

```text
Dialogue 1: 0     - 3410ms
Gap:        3410  - 3610ms
Dialogue 2: 3610  - 7290ms
```

frame変換はExporter時。

```rust
fn millis_to_frame(ms: u64, fps: u32) -> u32 {
    ((ms as f64 * fps as f64) / 1000.0).round() as u32
}
```

---

# 18. Preview Renderer

Macでも生成結果を確認できる必要がある。

MVPではFFmpegによる簡易Previewを生成する。

目的：

> 完成動画ではなく、タイミング確認

内容：

- 1920x1080
- 指定background
- VOICEVOX音声
- 字幕
- speaker名
- 最小限のfade

実装対象外：

- 高度な立ち絵animation
- YMM4 effect完全再現
- 高度なtransition

---

# 19. Preview Render Interface

```rust
pub trait PreviewRenderer {
    async fn render(
        &self,
        project: &VideoProject,
        output: &Path,
    ) -> Result<()>;
}
```

実装：

```text
FfmpegPreviewRenderer
```

将来：

```text
NativeRenderer
WebRenderer
```

---

# 20. YMM4 Exporter

## 原則

YMM4の完全schemaをVideoForgeで再実装しない。

**Template Patch方式**を採用する。

```text
template.ymmp
↓
JSON parse
↓
Prototype検出
↓
clone
↓
VideoProject IR値をpatch
↓
Windows絶対pathへmaterialize
↓
project.ymmp
```

---

# 21. YMM4 Template Contract

YMM4で一度テンプレートを作成。

Prototypeは`Remark`で識別。

```text
VF_PROTO_CAPTION_REIMU
VF_PROTO_CAPTION_MARISA
VF_PROTO_AUDIO
VF_PROTO_CHARACTER_REIMU
VF_PROTO_CHARACTER_MARISA
```

VideoForgeは未知のYMM4 propertyを保持する。

つまり、

```text
deserialize
↓
必要部分だけ変更
↓
serialize
```

にする。

不要なフィールドを消さない。

---

# 22. YMM4 path materialization

VideoProject：

```text
assets/audio/001.wav
```

Windows exporter：

```text
C:\Users\...\generated\001\assets\audio\001.wav
```

へ変換。

Macから直接Windows絶対pathを推測してはいけない。

そのためYMM4の正式exportはWindows側で行う。

---

# 23. Portable Handoff

Macで作成：

```bash
videoforge bundle ymm4 generated/001/project.vfp.json
```

結果：

```text
001-ymm4-bundle.zip
```

展開：

```text
001-ymm4-bundle/
├── project.vfp.json
├── source.md
├── manifest.json
├── template.ymmp
└── assets/
```

Windowsで：

```bash
videoforge export ymm4 ./project.vfp.json
```

これによりPCごとのpath問題をExporterで吸収する。

---

# 24. Tauri GUI

GUIは動画編集ソフトにしない。

編集の中心は台本と生成設定。

```text
┌─────────────────────────────────────────────────────┐
│ VideoForge                              macOS arm64 │
├──────────────────────────────┬──────────────────────┤
│ Script                       │ Settings             │
│                              │                      │
│ 霊夢: ...                    │ TTS: VOICEVOX ✓      │
│ 魔理沙: ...                  │ FPS: 30              │
│                              │ Gap: 200ms           │
│                              │ Template: default    │
├──────────────────────────────┴──────────────────────┤
│ Timeline                                           │
│ 01 霊夢   0.00 - 3.41                             │
│ 02 魔理沙 3.61 - 7.29                             │
├─────────────────────────────────────────────────────┤
│ [Validate] [Generate] [Preview]                    │
│                                                     │
│ macOS: YMM4 export requires Windows                │
│ [Create YMM4 Handoff Bundle]                       │
└─────────────────────────────────────────────────────┘
```

Windows：

```text
[Export YMM4]
[Open in YMM4]
```

を追加表示。

---

# 25. Capability Detection

フロント側でOS名だけを判定して分岐しない。

Rust側からCapabilityを返す。

```rust
pub struct Capabilities {
    pub platform: PlatformKind,

    pub voicevox_available: bool,
    pub ffmpeg_available: bool,

    pub can_export_ymm4: bool,
    pub can_open_ymm4: bool,
}
```

これにより将来Linuxにも伸ばせる。

---

# 26. Tauri Command

```rust
#[tauri::command]
async fn doctor() -> Result<DoctorResult, AppError>;
```

```rust
#[tauri::command]
async fn validate_script(
    path: PathBuf
) -> Result<ValidationResult, AppError>;
```

```rust
#[tauri::command]
async fn generate(
    request: GenerateRequest
) -> Result<GenerateResult, AppError>;
```

```rust
#[tauri::command]
async fn render_preview(
    project_path: PathBuf
) -> Result<PathBuf, AppError>;
```

```rust
#[tauri::command]
async fn export_ymm4(
    project_path: PathBuf
) -> Result<PathBuf, AppError>;
```

```rust
#[tauri::command]
async fn bundle_ymm4(
    project_path: PathBuf
) -> Result<PathBuf, AppError>;
```

---

# 27. Rust Workspace

```text
videoforge/
├── Cargo.toml
├── package.json
├── pnpm-workspace.yaml
│
├── apps/
│   └── desktop/
│       ├── src/
│       └── src-tauri/
│
├── crates/
│   ├── videoforge-core/
│   ├── videoforge-cli/
│   ├── videoforge-script/
│   ├── videoforge-project/
│   ├── videoforge-voicevox/
│   ├── videoforge-timeline/
│   ├── videoforge-preview/
│   ├── videoforge-export-ymm4/
│   └── videoforge-platform/
│
├── fixtures/
│   ├── scripts/
│   ├── templates/
│   └── projects/
│
└── tests/
```

---

# 28. Dependency Direction

```text
desktop
   │
   ▼
core ◀──── cli
 │
 ├── script
 ├── project
 ├── timeline
 ├── tts traits
 ├── preview traits
 └── exporter traits

voicevox ─────── implements TtsEngine
preview-ffmpeg ─ implements PreviewRenderer
export-ymm4 ───── implements ProjectExporter
```

禁止：

```text
core → Tauri
core → Windows API
core → YMM4 schema
```

---

# 29. Error Model

```rust
pub enum AppError {
    WorkspaceNotFound,
    InvalidConfig,
    InvalidScript,
    UnknownSpeaker,

    VoicevoxUnavailable,
    VoicevoxSynthesisFailed,

    FfmpegUnavailable,
    PreviewRenderFailed,

    InvalidTemplate,
    TemplatePrototypeMissing,

    UnsupportedPlatform,
    Ymm4Unavailable,

    FileReadFailed,
    FileWriteFailed,
}
```

---

# 30. UX上のエラー

Mac：

```text
YukkuriMovieMaker4はmacOSでは利用できません。

VideoForgeでの生成処理は完了しています。

Windowsで編集する場合は、
YMM4 Handoff Bundleを作成してください。

[Create Bundle]
```

これは失敗扱いにしない。

---

# 31. Generation Pipeline

```rust
pub async fn generate(
    workspace: &Workspace,
    script_path: &Path,
) -> Result<GeneratedProject, AppError> {

    let script =
        script::parse(script_path)?;

    let config =
        workspace.load_config()?;

    let validated =
        validate(&script, &config)?;

    let audio =
        tts::synthesize_all(
            validated.dialogues,
            &config.tts,
        ).await?;

    let project =
        timeline::build(
            validated,
            audio,
            &config.video,
            &config.timeline,
        )?;

    project.save_project_ir()?;

    project.write_srt()?;

    preview.render(&project).await?;

    Ok(project)
}
```

---

# 32. Progress Events

```rust
pub enum GenerationStage {
    Parsing,

    Synthesizing {
        current: usize,
        total: usize,
    },

    BuildingTimeline,

    WritingProject,

    WritingCaptions,

    RenderingPreview,

    Completed,
}
```

Tauri UIとCLIの両方で同じevent sourceを利用する。

---

# 33. Cancellation

`tokio_util::sync::CancellationToken`を利用。

キャンセル可能地点：

- 各TTS生成前
- preview render前
- export前

途中生成物は、

```text
.generated-tmp/
```

へ出力し、成功時にatomic renameする。

---

# 34. 出力構造

```text
generated/
└── 001-ai-news/
    ├── source.md
    ├── project.vfp.json
    ├── manifest.json
    ├── captions.srt
    ├── preview.mp4
    │
    ├── assets/
    │   └── audio/
    │       ├── 001.wav
    │       ├── 002.wav
    │       └── 003.wav
    │
    └── ymm4/
        └── project.ymmp
```

`ymm4/` はWindows export時のみ生成。

---

# 35. manifest.json

```json
{
  "schema_version": 1,
  "generator_version": "0.1.0",
  "source": "scripts/001-ai-news.md",
  "project": "project.vfp.json",
  "preview": "preview.mp4",
  "generated_at": "2026-09-06T09:00:00+09:00",
  "platform": "macos-arm64"
}
```

---

# 36. Database

MVPではDBを使わない。

理由：

- Gitと相性が良い
- Agentから読みやすい
- 移植しやすい
- Mac/Windows間で同期しやすい
- バックアップしやすい

状態は、

```text
YAML
Markdown
JSON
assets
```

で管理する。

---

# 37. Gitとの関係

コミット推奨：

```text
videoforge.yaml
AGENTS.md
channel/
research/
scripts/
templates/
```

原則ignore：

```text
generated/
cache/
.generated-tmp/
```

必要なら`generated/project.vfp.json`だけ残す運用も選択可能。

---

# 38. セキュリティ

MVPではLLM APIをVideoForge本体から呼ばない。

通信先：

```text
localhost VOICEVOX
```

のみ。

Agentが外部LLMへ送る内容はAgent側の責務。

VideoForgeは、

```text
script → media project
```

に限定する。

---

# 39. テスト戦略

## Unit

### Script

- Front Matter
- speaker parse
- multiline
- invalid speaker
- UTF-8 Japanese

### Path

Windows style pathをCanonical IRへ保存できないこと。

macOS absolute pathをCanonical IRへ保存できないこと。

### Timeline

- ms計算
- gap
- WAV duration
- frame conversion

### Project IR

- serialize / deserialize
- schema version
- relative asset path

---

# 40. Cross-platform CI

GitHub Actions：

```text
matrix:
  - windows-latest
  - macos-latest
```

チェック：

```text
cargo test
cargo clippy
cargo fmt --check
pnpm test
pnpm build
```

YMM4実アプリを必要とするE2EはWindows専用manual/integration testとして分離。

---

# 41. Integration Test

VOICEVOXをmock可能にする。

```rust
struct FakeTtsEngine;
```

固定WAVを返す。

これによりCIでVOICEVOX本体を起動しなくても、

```text
Script
→ TTS
→ Timeline
→ IR
→ SRT
```

をテストできる。

---

# 42. YMM4 Fixture Test

実際のYMM4 templateをfixture化。

```text
fixtures/templates/ymm4/default.ymmp
```

検証：

- prototype発見
- clone
- Text
- Frame
- Length
- Layer
- FilePath
- unknown property preserve

---

# 43. Manual Acceptance Test — Windows

```text
1. sample.mdをGenerate
2. preview.mp4再生
3. export ymm4
4. project.ymmpをYMM4で開く
5. 音声再生
6. 字幕同期確認
7. 字幕を編集
8. audio clipを移動
9. 保存
```

---

# 44. Manual Acceptance Test — macOS

```text
1. VideoForge起動
2. doctor
3. VOICEVOX接続
4. sample.mdをGenerate
5. WAV生成
6. preview.mp4再生
7. project.vfp.json確認
8. YMM4 bundle生成
9. Windowsへbundleコピー
10. Windowsでexport ymm4
11. YMM4で開く
```

---

# 45. MVP Definition of Done

## Cross-platform

- [ ] Windows Tauri appが起動
- [ ] macOS Tauri appが起動
- [ ] CLIが両OSで動く
- [ ] Workspace init
- [ ] doctor
- [ ] Script validation
- [ ] VOICEVOX接続
- [ ] TTS生成
- [ ] WAV duration
- [ ] Timeline生成
- [ ] project.vfp.json生成
- [ ] SRT生成
- [ ] preview.mp4生成
- [ ] TTS cache
- [ ] cancellation

## Windows

- [ ] YMM4 template読込
- [ ] `.ymmp`生成
- [ ] YMM4起動
- [ ] YMM4上で編集可能

## macOS

- [ ] YMM4機能がCapabilityとして無効表示
- [ ] それ以外は正常動作
- [ ] YMM4 handoff bundle生成
- [ ] Windowsでbundleから`.ymmp` materialize可能

---

# 46. 開発順序

## Phase 0 — Spike

最初にYMM4 exporterだけ検証。

```text
fixture project.vfp.json
↓
template.ymmp
↓
project.ymmp
↓
YMM4で開く
```

Windowsで行う。

ここが最大の技術リスク。

---

## Phase 1 — Core

```text
Workspace
Script parser
Project IR
Timeline
```

---

## Phase 2 — VOICEVOX

```text
doctor
speaker list
synthesis
duration
cache
```

Windows/macOS両方。

---

## Phase 3 — CLI

```text
init
doctor
validate
generate
```

CLIでMVP pipelineを完成させる。

---

## Phase 4 — Preview

```text
IR
↓
FFmpeg
↓
preview.mp4
```

Windows/macOS両方。

---

## Phase 5 — YMM4 Exporter

```text
IR
↓
Template Patch
↓
Windows path materialize
↓
project.ymmp
```

---

## Phase 6 — Handoff

```text
Mac
↓
bundle
↓
Windows
↓
export ymm4
```

---

## Phase 7 — Tauri

完成済みCore/CLIの上にGUIを被せる。

---

# 47. 開発チケット

## EPIC 1 — Workspace

- VF-001 `videoforge init`
- VF-002 config loader
- VF-003 Workspace resolver
- VF-004 AGENTS.md generator

## EPIC 2 — Script

- VF-010 Markdown parser
- VF-011 Front Matter parser
- VF-012 speaker resolver
- VF-013 validation
- VF-014 unit tests

## EPIC 3 — Project IR

- VF-020 project schema
- VF-021 relative asset path
- VF-022 serializer
- VF-023 schema migration interface

## EPIC 4 — TTS

- VF-030 VOICEVOX health
- VF-031 speakers
- VF-032 audio_query
- VF-033 synthesis
- VF-034 WAV duration
- VF-035 TTS cache

## EPIC 5 — Timeline

- VF-040 dialogue scheduler
- VF-041 gap policy
- VF-042 caption generation
- VF-043 SRT exporter

## EPIC 6 — Preview

- VF-050 FFmpeg detection
- VF-051 FFmpeg command builder
- VF-052 preview render
- VF-053 cancellation

## EPIC 7 — YMM4

- VF-060 template parser
- VF-061 prototype resolver
- VF-062 TextItem patch
- VF-063 AudioItem patch
- VF-064 Windows path materializer
- VF-065 `.ymmp` writer
- VF-066 YMM4 launch

## EPIC 8 — Portable Bundle

- VF-070 bundle builder
- VF-071 manifest
- VF-072 Windows restore/materialize

## EPIC 9 — CLI

- VF-080 init
- VF-081 doctor
- VF-082 validate
- VF-083 generate
- VF-084 export
- VF-085 bundle

## EPIC 10 — Tauri

- VF-090 Editor screen
- VF-091 Settings
- VF-092 Timeline table
- VF-093 Generate progress
- VF-094 Preview
- VF-095 Capability display
- VF-096 Finder / Explorer open
- VF-097 YMM4 export action

---

# 48. MVPから外すもの

以下はv0.1に入れない。

- X API
- バズ検知
- Google Trends
- YouTube検索
- 自動投稿
- YouTube Analytics
- LLM API統合
- AIリサーチ
- 自動台本生成
- 自動ファクトチェック
- 画像検索
- AI画像生成
- サムネ生成
- 高度な立ち絵animation
- 自動感情推定
- BGM自動選択
- SE自動選択
- FCPXML
- DaVinci Resolve exporter
- Premiere exporter
- MCP server
- Cloud sync
- Login
- Billing

---

# 49. v0.2

MVPが動いてから追加。

優先：

1. Script DSL
2. 立ち絵差分
3. emotion
4. pause
5. image
6. BGM/SE
7. Human Slot
8. Preview改善
9. Script regeneration hooks

---

# 50. v0.3

Agent workflow。

```text
Trend
↓
Research
↓
Script
↓
VideoForge generate
↓
Human Review
↓
YMM4 / other editor
```

VideoForge自体はTrend detectorを所有せず、外部AgentからWorkspaceへ成果物を書かせる。

---

# 51. v0.4

Mac-native editable export候補。

```text
Final Cut Pro XML
OpenTimelineIO
DaVinci Resolve compatible workflow
```

ただしYMM4 exporterと混ぜない。

すべて、

```text
VideoProject IR
↓
Exporter
```

として追加する。

---

# 52. 将来のExporter Interface

```rust
#[async_trait]
pub trait ProjectExporter {
    fn id(&self) -> &'static str;

    fn capabilities(&self) -> ExportCapabilities;

    async fn export(
        &self,
        project: &VideoProject,
        destination: &Path,
    ) -> Result<ExportResult>;
}
```

実装：

```text
Ymm4Exporter
FinalCutExporter
ResolveExporter
PremiereExporter
```

---

# 53. 技術判断まとめ

| 項目 | 採用 |
|---|---|
| Architecture | Workspace-first |
| Automation interface | CLI-first |
| GUI | Tauri v2 |
| Frontend | React + TypeScript |
| Core | Rust |
| OS | Windows + macOS |
| Canonical format | VideoProject IR JSON |
| Script | Markdown |
| Config | YAML |
| DB | なし |
| TTS | VOICEVOX |
| Preview | FFmpeg |
| Editable Windows export | YMM4 `.ymmp` |
| Mac YMM4 workflow | Portable handoff |
| Agent integration | Files + CLI |
| LLM SDK | MVPではなし |
| Asset paths | Workspace-relative |
| Cache | OS cache directory |
| Test | Win/macOS CI |

---

# 54. 重要な設計判断

## 決定1

**`.ymmp`をSingle Source of Truthにしない。**

理由：

YMM4はWindows依存だから。

---

## 決定2

**VideoProject IRをSingle Source of Truthとする。**

その結果、

```text
Windows
macOS
YMM4
FFmpeg
将来のFinal Cut / Resolve
```

を同じprojectから生成できる。

---

## 決定3

**Agent APIを作らない。**

File + CLIをAgent Interfaceにする。

これによりClaude Code、Codex等に依存しない。

---

## 決定4

**Macでも生成結果を確認可能にする。**

YMM4が動かなくても、

```text
preview.mp4
```

まで生成する。

---

## 決定5

**YMM4のWindows absolute path問題はmaterialize時に解決する。**

MacでWindowsパスを推測しない。

---

# 55. 最初に作るコード

GUIではない。

最初はこのCLIを成立させる。

```bash
videoforge init demo

cd demo

videoforge generate scripts/sample.md
```

期待：

```text
generated/sample/
├── project.vfp.json
├── captions.srt
├── preview.mp4
└── assets/audio/
```

これをWindows/macOS両方で成功させる。

その後Windowsだけ、

```bash
videoforge export ymm4 \
  generated/sample/project.vfp.json
```

を実行。

```text
generated/sample/ymm4/project.ymmp
```

がYMM4で開ければMVPの技術検証完了。

---

# 56. 一文で定義

> **VideoForgeは、人間またはAIエージェントが書いたMarkdown台本を、音声・字幕・タイムラインを持つOS非依存のVideoProjectへ変換し、WindowsではYMM4編集プロジェクト、Windows/macOSではプレビュー動画として出力するローカル動画制作コンパイラである。**

---

# 57. MVPのスコープ境界

VideoForge v0.1が責任を持つ範囲：

```text
台本
│
▼
VideoForge
│
├─ Voice
├─ Caption
├─ Timing
├─ Project IR
├─ Preview
└─ YMM4 Export
```

責任を持たない範囲：

```text
ネタ探し
リサーチ
企画
投稿
収益分析
```

これらはAgent側。

この境界を崩さないことが、MVPを完成させる上で最重要。
