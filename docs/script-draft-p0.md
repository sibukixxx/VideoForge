# 資料から台本案を作る P0

この機能は **外部AIを使う半自動経路** です。API呼び出し・課金・Web検索・
自動送信・動画生成は行いません。既存のエージェントによる台本作成を活かし、
根拠付きJSON回答の取り込みと検査を追加します。GUI追加は今回の範囲外です。

## できること・しないこと

| 項目 | P0の動作 |
|---|---|
| 資料と制作条件をAI用プロンプトにする | できる |
| AI回答のJSON構造、話者、台詞長を検査する | できる |
| `fact`の引用が入力資料に文字列として存在するか確認する | できる |
| 承認した回答を既存形式のMarkdown台本にする | できる |
| 台本から音声・字幕・`preview.mp4`を生成する | 既存の`validate` / `generate`で明示的に実行 |
| VideoForgeがLLM APIを直接呼ぶ | しない |
| ネタ検知、Web検索、Discord等への送信、動画公開 | しない |
| 真偽、法的安全性、引用権、論調の公平性を自動判定する | しない |

この境界により、外部AIや配信先に依存せず、Canonical IRである
`project.vfp.json`までの既存生成経路を変更しません。

## 手順

1. `videoforge init demo` でWorkspaceを作り、そのフォルダへ移動。
2. リポジトリの `fixtures/draft/brief.json` をWorkspaceにコピーし、テーマ・
   視聴者・目標秒数・資料本文を編集。日付不明は `unknown`。
3. `videoforge draft prompt brief.json > prompt.txt` を実行し、出力を外部AIへ渡す。
   資料全文が含まれるため、送信前に機密情報と利用権限を確認。
4. AIのJSON回答をWorkspace内の `response.json` として保存。
5. `videoforge draft check brief.json response.json` を実行。
6. 台詞・タイトル・引用の文脈・未確認事項を人間が確認。修正後は再チェック。
7. チェック結果の `review_hash` を指定して書き出す。

```sh
videoforge draft export brief.json response.json \
  --reviewed-hash <確認した版のreview_hash> \
  --reviewer takada --out scripts/reviewed-draft.md
videoforge validate scripts/reviewed-draft.md
# 実VOICEVOXとFFmpegが起動・利用可能な環境で、明示的に実行:
videoforge generate scripts/reviewed-draft.md
```

`draft check`は常にJSONをstdoutへ出します。成功時は終了コード0、構造上の問題が
ある場合は終了コード2です。`draft export`も、hash不一致や未解決事項がある場合は
終了コード2になり、台本を書き出しません。

全入力パスはWorkspace相対。出力は既存の `scripts/` 内で `.md` の新規ファイルだけ。
既存台本は上書きしません。MarkdownはRust側で生成し、既存parserで往復検査します。
台本から既存の生成処理が作る `project.vfp.json` の位置付けは変更しません。

## 入力: brief.json

```json
{
  "topic": "動画のテーマ",
  "audience": "想定視聴者",
  "target_seconds": 60,
  "sources": [
    {
      "id": "S1",
      "locator": "https://example.com/source-or-local-reference",
      "published_at": "2026-09-01",
      "checked_at": "2026-09-12",
      "text": "AIに渡してよい範囲の資料本文"
    }
  ]
}
```

- `target_seconds`は10〜1800秒。
- `sources`は1件以上必要で、`id`は重複不可。
- `locator`は出所を追跡するための識別子です。VideoForgeはURLへアクセスしません。
- 日付が確認できない場合は推測せず`unknown`を使います。
- `text`へ入れた資料全文は生成プロンプトに含まれます。
- 各入力ファイルの上限は1MBです。

## 外部AIの回答: response.json

外部AIには、`draft prompt`の出力を省略せず渡します。回答はコードフェンスや説明を
除いた、次の形のJSONオブジェクトだけを保存します。

```json
{
  "title": "台本タイトル",
  "dialogues": [
    {
      "speaker": "videoforge.yamlで許可された話者",
      "text": "1行120文字以下の台詞",
      "kind": "fact",
      "evidence": [
        {"source_id": "S1", "quote": "sources[].textに実在する短い引用"}
      ]
    }
  ],
  "unresolved": [],
  "material_requests": []
}
```

`kind`は`fact`、`opinion`、`question`のいずれかです。`fact`には1件以上の
`evidence`が必要です。資料不足、矛盾、確認できない疑惑は`unresolved`へ、画像や
図表など未準備の素材は`material_requests`へ残します。どちらかが空でない回答は
書き出せません。問題を解決して台本・資料・回答を更新し、再度チェックします。

## check結果の読み方

主なフィールドは次の通りです。

| フィールド | 意味 |
|---|---|
| `structurally_valid` | 自動検査を通過したか。内容の真偽を意味しない |
| `review_hash` | プロンプト、資料、回答、設定を結び付けるSHA-256 |
| `errors` | exportを止める問題 |
| `warnings` | 人間が確認すべき注意事項 |
| `estimated_seconds` | 毎秒5文字で計算した仮の長さ |
| `markdown` | export予定のMarkdown台本 |

人間は少なくとも、タイトル、全ての事実主張、引用の前後関係、数字・日付・単位、
話者、未確認事項、権利と公開リスクを確認します。確認中に`brief.json`、
`response.json`、`videoforge.yaml`または組み込みプロンプトが変わるとhashも変わります。

## 動画まで生成する

`draft export`は台本Markdownを作るだけです。次の既存経路を別途実行します。

```sh
videoforge validate scripts/reviewed-draft.md
videoforge doctor
videoforge generate scripts/reviewed-draft.md
open generated/reviewed-draft/preview.mp4  # macOS
```

生成物の正は`generated/reviewed-draft/project.vfp.json`です。`captions.srt`、
`preview.mp4`、音声ファイルはそこから派生する成果物です。動画を最後まで視聴し、
話者表示、実際のVOICEVOX音声、字幕、固有名詞、読み、間、尺を確認してから公開します。

## よくあるエラー

| 症状 | 原因と対応 |
|---|---|
| `unknown speaker` | `videoforge.yaml`のspeakerキーまたはaliasへ修正 |
| `facts require evidence` | factへ根拠を追加するか、事実でなければkindと表現を見直す |
| `missing source or non-verbatim evidence` | source IDと、資料本文に完全一致する短いquoteを確認 |
| `unresolved issues must be resolved` | 追加調査・表現修正後にunresolvedを更新。単に削除しない |
| `material requests must be resolved` | 素材要求を解決するか、未準備素材を前提にした台詞を削除 |
| `split text into at most 120 characters` | 台詞を複数dialogueへ分割 |
| `generated Markdown does not round-trip` | 台詞中の予約構文、改行、directive相当の記述を除去 |
| exportが終了コード2 | `check`を再実行し、最新hash・errors・warningsを確認 |
| 出力先が作れない | `--out scripts/<new-name>.md`を使用。既存ファイルは上書き不可 |

## 検証と承認の限界

- factは出典IDと資料内の正確な引用を必須にします。ただし、引用の存在は
  真実性・主張との意味的一致を証明しません。factをopinionに偽装することも
  ルール検査だけでは防げません。法的判断、権利確認、矛盾判定は人間が担当。
- unresolved / material_requestsが残っている場合は書き出し不可。
  消すだけで解決したことにはなりません。台本も修正して再確認してください。
- 話者、台詞長、予約構文、存在しない引用を検査。P0ではdirectiveを出力しません。
- 時間は毎秒5文字という仮定の参考値。±20%超は警告。実音声で測定するまで
  時間目標の達成とは扱いません。
- review_hashは資料・回答・設定・プロンプトの内容に紐付きます。変更後に古い
  hashで書き出せません。署名や本人認証ではなく、人間が確認したという申告です。
- 書き出し後の手動編集や既存 `generate` コマンドを強制的に止める機能ではありません。
  変更した台本は再レビューが必要。公開承認は動画の視聴後に別途行います。
- brief.json / response.jsonを保存して根拠を残してください。生成物へコピーされないため、
  台本単体での共有では根拠台帳は失われます。

## テスト

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p videoforge-core --test draft
cargo test -p videoforge-cli --test draft
cargo test --workspace
```

fixtureは合成資料であり、実AI出力の品質評価ではありません。2026-09-12時点で、
既存のサンプル台本から実VOICEVOX・FFmpegを経由した`preview.mp4`の生成と再生は
macOSで手動確認済みです。これは外部AIが作る台本の事実性・品質・費用・修正時間を
証明するものではないため、P1では実案件に近い資料を使って別途測定します。

## P1へ進む判断基準

完全自動化や自動公開へ進む前に、最低3種類の合成または公開資料で次を記録します。

- 初稿生成時間と、人間が承認できるまでの修正時間
- fact数、根拠不備数、重要な誤り・見落とし数
- 目標尺と実VOICEVOX音声尺の差
- 1本当たりのAI利用費（使用した外部AI側で計測）
- 最終動画で発見した話者、読み、字幕、テンポの問題

P1の第一候補は、AI API直結ではなく、brief/response/review記録を同じ案件単位で
保存し、上記評価を再現可能にすることです。品質が測れないまま量産・送信・公開を
自動化しません。
