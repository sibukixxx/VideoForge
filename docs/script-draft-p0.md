# 資料から台本案を作る P0

この機能は **外部AIを使う半自動経路** です。API呼び出し・課金・Web検索・
自動送信・動画生成は行いません。既存のエージェントによる台本作成を活かし、
根拠付きJSON回答の取り込みと検査を追加します。GUI追加は今回の範囲外です。

## 手順

1. `videoforge init demo` でWorkspaceを作り、そのフォルダへ移動。
2. リポジトリの `fixtures/draft/brief.json` をWorkspaceにコピーし、テーマ・
   視聴者・目標秒数・資料本文を編集。日付不明は `unknown`。
3. `videoforge draft prompt brief.json` を実行し、出力を外部AIへ渡す。
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

全入力パスはWorkspace相対。出力は既存の `scripts/` 内で `.md` の新規ファイルだけ。
既存台本は上書きしません。MarkdownはRust側で生成し、既存parserで往復検査します。
台本から既存の生成処理が作る `project.vfp.json` の位置付けは変更しません。

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

fixtureは合成資料であり、実AI出力の品質評価ではありません。
Rust環境がない実装環境では上記は未実行です。実AIでの台本品質・費用・修正時間、
実VOICEVOXからMP4までの視聴確認も別途必要です。
