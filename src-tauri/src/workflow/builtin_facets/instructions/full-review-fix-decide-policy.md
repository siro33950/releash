{{project_name}} のフルレビューで残った全 Open Thread に対し、**各 Thread の修正方針を決定し、方針 Comment として Thread に投稿する**。実装は次の Step（implement）で行うため、本 Step では実装に着手しない。

## 入力

- タスク（任意の自由文。方針決定の補足指示があれば）: {{task}}
- 全 Open Thread（reviewer 指摘 + verifier 分類）

## 出力

- 各 Open Thread に方針 Comment（方針＋根拠）が投稿された状態
- 全 Thread に投稿し終えたら approve で次 Step（implement）へ進む

## プロセス

### 1. Open Thread の取得とグルーピング

- `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json`
- 各 Thread に対し `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` で本文・履歴を取得
- 各 Thread の verifier 分類（複数 verifier 出力）を比較し、以下の 2 グループに分ける：
  - **一致グループ**：両 verifier が同分類で、かつ方針が自明な Thread（例: 両者 VERIFIED で対応 / 両者 REFUTED で wontfix）
  - **割れグループ**：verifier 間で分類が割れている、または同一分類でも判断要素を含む Thread

### 2. 一致グループの一括処理

- 一致グループの全 Thread を一覧で提示する：

  ```
  ## 一致グループ方針案 (<件数>件)

  ### Thread <thread-id> [<観点>] <file>:<line-range>
  - verifier 分類: <双方 一致した分類>
  - 方針案: <1 文>
  - 根拠: <verifier 出力の要約 1〜2 文>

  ### Thread <thread-id> ...
  ```

- 人間が **一括 approve** したら、各 Thread に方針 Comment を投稿する：
  - `{{path_alias.releash}} review comment <thread-id> --session-id "$RELEASH_SESSION_ID" --content "方針：<決定した方針>\n根拠：<verifier 判定要約>" --json`
- 一致グループでも個別に reject されたものがあれば、その Thread は割れグループに回す

### 3. 割れグループの逐次処理

- 割れグループは Thread を **1 件ずつ** 提示する：

  ```
  ## 割れ Thread <thread-id> [<観点>] <file>:<line-range>
  - reviewer 指摘: <Thread 本文の要約>
  - verifier 出力: <verifier 名>=<分類> / <根拠要約> / <verifier 名>=<分類> / <根拠要約>
  - 論点: <verifier 間の対立点 / 判断要素>
  - 方針案: <1 文>
  - 根拠: <提案する根拠>
  ```

- 人間と必要に応じて議論し、合意（approve）を得てから、その Thread に方針 Comment を投稿する：
  - `{{path_alias.releash}} review comment <thread-id> --session-id "$RELEASH_SESSION_ID" --content "方針：<決定した方針>\n根拠：<議論を踏まえた根拠>" --json`
- 投稿後、次の割れ Thread に進む

### 4. 完了確認

- 全 Open Thread に方針 Comment が投稿されたことを確認する：
  - `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json` で再取得
  - 各 Thread の history に方針 Comment（`方針：` `根拠：` を含む）が存在することを確認
- 完了報告：

  ```
  ## 方針決定 完了

  ### 一致グループで一括処理した Thread (<件数>件)
  - `<thread-id>`: <方針 1 文>
  - ...

  ### 割れグループで個別処理した Thread (<件数>件)
  - `<thread-id>`: <方針 1 文>
  - ...
  ```

- 人間が approve したら次 Step（implement）へ進む

## 方針 Comment のフォーマット

```
方針：<決定した方針を 1 文または数文で>
根拠：<verifier 判定要約 / 議論で出た論点 / 採用理由>
```

- 「対応する」も「対応見送り」も方針として明示する。「resolve するか Open に残すか」ではなく「実装でどう扱うか」を書く
- 実装ヒント（変更箇所候補・具体的手法）は含めない。実装 Step の判断領域

## 禁止事項

- 実装の着手・コード変更
- Thread の `review resolve`（resolve は実装 Step の責務）
- 方針 Comment 以外の Comment 投稿
- 方針未決の Thread を残したまま approve に進む
