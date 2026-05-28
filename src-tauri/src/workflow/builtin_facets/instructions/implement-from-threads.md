{{project_name}} プロジェクトの Open Thread を集約して必要な実装を行い、各 Thread に対応反映（resolve または申し送り Comment）まで完結させる。

## 入力

- タスク（任意の自由文。実装対象の絞り込み等の補足指示があれば）: {{task}}
- Open Thread 一覧: 後述の手順で `review list` から取得する

## 基本方針

- 推測で結論しない。Thread の `review get` / `review history` 出力を実際に読み、根拠を持って判断する
- ロジックは Rust 側に配置し、フロントエンドはインターフェースに徹する
- 実装着手前に必ず実装計画を人間に提示し、合意を得る。合意なしに編集を開始しない
- Thread の対応見送り判断は必ず根拠を1文以上添えて申し送り Comment を投稿する（無言で resolve / 放置しない）
- 既存仕様（Spec ファイル・既存テスト）と矛盾する Thread 要求があれば、矛盾を計画で明示し人間判断を仰ぐ

## プロセス

### 1. Open Thread 一覧の取得

- `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json`
- 必要に応じて `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json`
- 必要に応じて `{{path_alias.releash}} review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json`

### 2. 実装方針の集約

- 各 Thread の `review get` 出力（本文・対象範囲・現在の Stance・履歴）を踏まえる
- 複数 Thread の衝突・依存関係を整理する

### 3. 実装計画の提示と合意取得

- 以下のテンプレートに従って実装計画を出力する
- 出力後、人間の合意が取れるまで実装には着手しない
- 修正指示があれば計画を改訂して再提示する

#### 実装計画テンプレート

```markdown
## 実装計画

### 全体方針
<Open Thread を集約した実装方針を 1〜3 文で>

### 変更対象
- `<対象ファイル / モジュール>`: <変更内容の要点>
  - 満たす Thread: `<thread-id>` ... (複数可)
- ...

### 対応見送り Thread
- `<thread-id>`: <見送る理由（implement step で申し送り Comment を投稿する根拠）>
- 見送りがなければ `なし`

### Thread 間の衝突 / 依存
- <衝突点とその解消方針、または依存関係に基づく実装順序>
- なければ `なし`

### 確認事項
- <人間に明示的に判断を求めたい点があれば列挙>
- なければ `なし`
```

### 4. 実装

- 合意済み計画に沿って実装する

### 5. Thread への対応反映

- 要求を満たした → `{{path_alias.releash}} review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome <outcome> --summary "<対応要約>" --json`
- 対応見送り / 部分対応 → `{{path_alias.releash}} review comment <thread-id> --session-id "$RELEASH_SESSION_ID" --content "<申し送り内容>" --json`
- 申し送り Comment は根拠を1文以上含める

### 6. 処理結果サマリの出力

以下のテンプレートで最後に実装と Thread 対応の最終報告を出力する。

#### 処理結果サマリテンプレート

```markdown
## 処理結果サマリ

### 実装内容
- `<変更したファイル>`: <変更内容>
- ...

### Thread 処理結果
| thread-id | action | outcome / 申し送り |
|---|---|---|
| `<id>` | `resolved` | `<outcome>` / `<summary>` |
| `<id>` | `commented` | `<申し送り内容>` |

### 残課題 / 申し送り
- <次フェーズへの引き継ぎがあれば>
- なければ `なし`
```

## 注意事項

- `outcome` の値は Thread の解決状況を表す自由文（例: `resolved`, `wontfix`, `duplicate`）。Thread の文脈に合わせて適切なものを選ぶ
- 同意・異議は専用フラグではなく Comment 本文で表現する
