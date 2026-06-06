{{project_name}} のフルレビューで残った全 Open Thread を、**方針決定 Step（decide_policy）で各 Thread に付与された方針 Comment** に従って実装し、各 Thread を resolve する。

## 入力

- タスク（任意の自由文。実装対象の絞り込み等の補足指示があれば）: {{task}}
- 全 Open Thread と各 Thread に付いた方針 Comment（既に decide_policy Step で投稿済み）

## 前提

- 本 Step は **方針決定 Step の後段** に位置する。各 Open Thread には方針決定 Step が投稿した方針 Comment（方針＋根拠）が必ず付いている
- 本 Step では **新たに方針を決め直さない**。方針 Comment の内容を実装に翻訳することに徹する
- 方針 Comment と矛盾する実装が必要だと判断した場合は、計画提示の「確認事項」で明示し人間判断を仰ぐ（独断で方針を変更しない）

## プロセス

### 1. Open Thread と方針 Comment の取得

- `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json`
- 各 Thread に対し `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` で本文・履歴を取得
- **history の中から方針 Comment（`方針：` `根拠：` を含む）を必ず特定する**

### 2. 実装計画の提示と合意取得

- 下記テンプレートで計画を提示し、人間が approve するまで実装に着手しない
- 修正指示があれば計画を改訂して再提示する

#### 実装計画テンプレート

```markdown
## 実装計画

### 各 Thread の方針 Comment と実装対応
- `<thread-id>` / `<file>:<line-range>`
  - 方針 Comment（要約）: <方針 Comment の方針部分を 1 文で>
  - 実装内容: <対象ファイル・モジュール / 具体的な変更点>
- ...

### 実装順序
- <Thread 間の依存・衝突に基づく実装順序、または「順不同」>

### Thread 間の衝突 / 依存
- <衝突点と解消方針 / 依存関係>
- なければ `なし`

### 確認事項
- <方針 Comment と整合しない実装が必要、追加情報が必要、など人間判断を求めたい点があれば列挙>
- なければ `なし`
```

### 3. 実装

- 合意済み計画に沿って一括実装する

### 4. 各 Thread を resolve

- 各 Thread を `{{path_alias.releash}} review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome <outcome> --summary "<対応要約>" --json` で resolve する
- `--outcome` は解決状況を表す自由文（例: `resolved`, `wontfix`, `duplicate`）
- 「対応見送り」も resolve として扱い、根拠を `--summary` に含める
- 申し送り Comment は投稿しない

### 5. 処理結果サマリの出力

```markdown
## 処理結果サマリ

### 実装内容
- `<変更したファイル>`: <変更内容>
- ...

### Thread 処理結果
| thread-id | outcome | summary |
|---|---|---|
| `<id>` | `<outcome>` | `<summary>` |
```

## 禁止事項

- 方針 Comment を読まずに実装を開始しない
- 方針 Comment と矛盾する実装を、計画提示・合意なしに行わない
- Thread を Open のまま残さない（全 Thread を resolve する。方針 Comment が「対応見送り」を示している場合も resolve）
