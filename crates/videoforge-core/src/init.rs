//! `videoforge init` (design §7.1, §6).

use std::path::{Path, PathBuf};

use crate::config::default_config_yaml;
use crate::error::AppError;
use crate::workspace::{AGENTS_FILE, CONFIG_FILE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    pub root: PathBuf,
    pub created: Vec<PathBuf>,
}

pub const SAMPLE_SCRIPT: &str = r#"---
title: VideoForge サンプル台本
template: default
---

# 導入

霊夢:
VideoForgeのサンプル台本です。
この台本から音声と字幕、タイムラインが生成されます。

魔理沙:
Markdownで台本を書いて、CLIを実行するだけだぜ。

霊夢:
生成結果は generated フォルダに出力されます。
"#;

pub fn agents_md(project_name: &str) -> String {
    format!(
        r#"# VideoForge Agent Instructions

このディレクトリは VideoForge Workspace (`{project_name}`) です。
AIエージェント / 人間は **ファイルの編集** と **CLIの実行** だけで動画プロジェクトを生成します。

## 動画を生成するとき

1. `scripts/` に Markdown 台本を作成する（形式は下記）
2. `videoforge validate scripts/<file>.md` を実行する
3. エラーがなければ `videoforge generate scripts/<file>.md` を実行する
4. `generated/<slug>/manifest.json` を確認する
5. 必要なら台本を修正して再生成する

## Windows で YMM4 プロジェクトが必要な場合

```bash
videoforge export ymm4 generated/<slug>/project.vfp.json
```

macOS では YMM4 を起動できないため、Windows へ渡す handoff bundle を作る:

```bash
videoforge bundle ymm4 generated/<slug>/project.vfp.json
```

## 台本フォーマット

```markdown
---
title: 動画タイトル
---

霊夢:
一つ目のセリフ。

魔理沙:
二つ目のセリフ。
```

- `話者名:` の行でセリフを開始する（話者名は `videoforge.yaml` の `speakers` のキーまたは alias）
- 続く行がセリフ本文（複数行可）
- `#` で始まる行は見出し（コメント扱い）
- `@pause` などの directive は v0.1 では未対応（warning になる）

## 環境確認

```bash
videoforge doctor
```

VOICEVOX Engine が `http://127.0.0.1:50021` で起動している必要があります。

## 触ってはいけないもの

- `generated/` と `.generated-tmp/` は生成物。手で編集せず、台本を直して再生成する
- `project.vfp.json` が正であり、`.ymmp` は派生物
"#
    )
}

const TEMPLATE_README: &str = r#"# YMM4 templates

Place a YukkuriMovieMaker4 project here as `default.ymmp` to enable
`videoforge export ymm4` (Windows).

The exporter uses a *template patch* strategy: it looks for prototype items in
the template, identified by their `Remark` (備考) field, clones them for each
dialogue and patches only `Text` / `Frame` / `Length` / `FilePath`.

Required prototypes (create them once in YMM4 and set the Remark):

| Remark                    | Item type          | Used for                    |
|---------------------------|--------------------|-----------------------------|
| `VF_PROTO_AUDIO`          | 音声アイテム        | one per dialogue audio clip |
| `VF_PROTO_CAPTION_<KEY>`  | テキストアイテム    | captions of speaker `<KEY>` |
| `VF_PROTO_CAPTION`        | テキストアイテム    | fallback caption            |
| `VF_PROTO_CHARACTER_<KEY>`| 立ち絵アイテム      | optional, per speaker       |

`<KEY>` is the upper-cased speaker key from `videoforge.yaml` (`REIMU`, `MARISA`).
All other template content is preserved untouched.
"#;

const GITIGNORE: &str = r#"# VideoForge outputs
generated/
.generated-tmp/
cache/
"#;

pub fn init(dir: &Path, name: Option<&str>) -> Result<InitReport, AppError> {
    let root = dir.to_path_buf();
    let config_path = root.join(CONFIG_FILE);
    if config_path.exists() {
        return Err(AppError::InvalidConfig {
            path: config_path,
            reason: "workspace already initialized (videoforge.yaml exists)".into(),
        });
    }
    let project_name = name
        .map(str::to_string)
        .or_else(|| {
            root.canonicalize()
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        })
        .or_else(|| root.file_name().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "my-channel".into());

    let mut created = Vec::new();
    let mut mkdir = |rel: &str| -> Result<(), AppError> {
        let p = root.join(rel);
        std::fs::create_dir_all(&p).map_err(|e| AppError::write(&p, e))?;
        created.push(p);
        Ok(())
    };
    for d in [
        "scripts",
        "assets/character",
        "assets/background",
        "assets/image",
        "assets/bgm",
        "assets/se",
        "templates/ymm4",
        "generated",
    ] {
        mkdir(d)?;
    }

    let mut write = |rel: &str, content: &str| -> Result<(), AppError> {
        let p = root.join(rel);
        std::fs::write(&p, content).map_err(|e| AppError::write(&p, e))?;
        created.push(p);
        Ok(())
    };
    write(CONFIG_FILE, &default_config_yaml(&project_name))?;
    write(AGENTS_FILE, &agents_md(&project_name))?;
    write(".gitignore", GITIGNORE)?;
    write("scripts/sample.md", SAMPLE_SCRIPT)?;
    write("templates/ymm4/README.md", TEMPLATE_README)?;
    for keep in [
        "assets/character",
        "assets/background",
        "assets/image",
        "assets/bgm",
        "assets/se",
        "generated",
    ] {
        write(&format!("{keep}/.gitkeep"), "")?;
    }

    Ok(InitReport { root, created })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    #[test]
    fn creates_workspace_layout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        let report = init(&root, None).unwrap();
        assert_eq!(report.root, root);
        for f in [
            "videoforge.yaml",
            "AGENTS.md",
            "scripts/sample.md",
            "templates/ymm4/README.md",
            ".gitignore",
        ] {
            assert!(root.join(f).is_file(), "{f}");
        }
        for d in ["assets/background", "generated", "templates/ymm4"] {
            assert!(root.join(d).is_dir(), "{d}");
        }
        let ws = Workspace::open(&root).unwrap();
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.name, "demo");
        assert!(std::fs::read_to_string(root.join("AGENTS.md"))
            .unwrap()
            .contains("videoforge generate"));

        // second init refuses
        assert!(init(&root, None).is_err());
    }
}
