# 3段階PNG口パク

VideoForgeはPSDを直接読みません。PSDTool等で同じキャンバス寸法・同じ配置の
完成済み透過PNGを3枚書き出し、VOICEVOX音声の振幅から口を切り替えます。

```text
assets/character/zundamon/
├── closed.png  # 口を閉じた完成画像
├── half.png    # 半開きの完成画像
└── open.png    # 全開の完成画像
```

口だけを切り抜いた画像ではなく、体・顔・口を合成した同寸法の3枚を用意します。
PSD原本や配布物はリポジトリへコミットせず、配布元の利用条件とクレジット要件を
確認してください。

## 台本

```markdown
@character zundamon[src=assets/character/zundamon/closed.png, mouth_half=assets/character/zundamon/half.png, mouth_open=assets/character/zundamon/open.png, x=0.78, y=0.56, scale=0.9]
ずんだもん:
音声の大きさに合わせて口が動くのだ。
```

`src`が閉じ口、`mouth_half`が半開き、`mouth_open`が全開です。口パクを使う場合は
3項目をすべて指定します。半開き・全開の片方だけを指定した場合やファイルが無い
場合はwarningになり、安全のため静止画表示へ戻ります。

## 処理

1. 既存TTS処理がVOICEVOX音声をWAVとして生成する。
2. 既存の50ms間隔RMS振幅解析を再利用する。
3. `mouth_open`を閉じ（`< 0.20`）、半開き（`< 0.55`）、全開に量子化する。
4. 同じ状態が続く区間を1つのcueへ圧縮する。
5. 素材参照とcueを`project.vfp.json`のCharacterClipへ保存する。
6. FFmpeg previewがcueの時間だけ該当PNGを透過合成する。

FFmpegが音声を独自解析するのではなく、口状態はCanonical IRに保存されます。同じIRを
別rendererやexporterが利用しても、同じタイミングを再現できます。これは音素解析では
なく振幅ベースなので、母音に合った口形状までは再現しません。

## 確認

```bash
videoforge validate scripts/sample.md
videoforge generate scripts/sample.md
open generated/sample/preview.mp4
```

確認項目:

- 背景が透け、PNG周囲に白や黒の矩形が出ない
- 無音区間で閉じ口になる
- 発話中に半開き・全開へ切り替わる
- 3枚の位置と寸法が一致し、切替時にキャラクターが跳ねない
- 字幕が立ち絵より前面に表示される
- `project.vfp.json`のcharacter clipに`mouth`と`cues`が記録される

口の動きが激しすぎる場合は、P1.1で平滑化や最小保持時間を追加します。まずは3段階が
音声と同期して再現可能に動くことを検証します。
