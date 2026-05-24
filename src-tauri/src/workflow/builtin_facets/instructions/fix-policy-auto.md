# Fix Policy Auto Decision

入力で渡されたレビュー結果（6観点 reviewer の `review-verdict`）と Spec ファイルを読み込み、実装の修正方針を **エージェント自身が** 策定する。
ユーザーへの承認問い合わせは行わず、最初の応答で `releash workflow output submit` により構造化出力を提出する。

ファイル編集は一切行わない（permission: readonly）。

## プロセス

### 1. 入力の読み込み

- 入力に含まれる 6観点 reviewer 各々の `review-verdict` を取得する
- 各 `review-verdict.findings` を全件抽出する
- 入力に含まれる Spec ファイルパスを参照する（要求・振る舞い定義の文脈把握のため）

### 2. 妥当性検証（各 finding ごと）

各 finding について以下を判定し、内部メモを作成する:

| 観点 | 判定基準 |
|------|---------|
| ファクト | `line` の指す箇所を実コードで確認し、指摘内容が事実かを検証する。コード上で再現できない指摘は `false-positive` 扱い |
| スコープ | 今回の変更（scope:diff）に紐づく問題か。`scope:touched` / `scope:external` は記録のみで `action: skip` 寄り |
| 仕様整合 | Spec の要求・振る舞い定義に違反する指摘は採用しない（指摘と Spec の両立が可能なら採用） |
| 重複 | 複数 reviewer が実質同じ箇所を指摘している場合は、最も具体的な1件に集約する |

### 3. action 自動決定ルール

検証結果をもとに、各 finding の `action` を以下の規則で決定する:

| 条件 | action | 備考 |
|------|--------|------|
| `severity: "error"` かつファクト・スコープ整合 | `fix` | 必ず修正対象 |
| `severity: "warning"` かつファクト・スコープ整合 | `fix` | 原則修正対象。明確な理由があれば `skip` |
| `severity: "info"` | `skip` | 原則修正対象外（任意改善） |
| ファクトチェックで再現できない（false-positive） | `skip` | `rationale` に「ファクト未確認」と明記 |
| `scope:touched` / `scope:external` | `skip` | `rationale` にスコープ理由を明記 |
| Spec 違反になる修正 | `skip` | `rationale` に Spec 整合理由を明記 |
| 重複指摘の集約先以外 | `skip` | `rationale` に「他 finding に集約」と明記 |

「ついでに直す」追加項目は **挙げない**（自動実行のため、明示的にレビューで挙がっていない事項は触らない）。

### 4. policy 全体方針の生成

`policy` フィールドには以下を含む短い自由文（4〜6 行程度）を入れる:
- 修正の優先順位（error 先行・テスト/lint 後実行など）
- 採用方針の要旨（スコープ・Spec 整合の扱い）
- 棄却した false-positive の件数（あれば）

## 出力

策定した方針を `approved-fix-policy` Contract に従う JSON にして `releash workflow output submit` で提出する。

```sh
releash workflow output submit <run_id> \
  --step <step_name> \
  --type approved-fix-policy \
  --json '{"review_step":"code_review_parallel","policy":"...","findings":[...]}'
```

- `findings` は元のレビュー結果から **棄却分も含めて** 全件残す（fix 側で `action: "skip"` を解釈する）
- `line` は元の review-verdict から引き継ぐ（無い場合は省略）
- 「ついでに直す」項目は加えない
- `review_step` は `code_review_parallel`
- 提出が成功するまで step は完了として扱われない。失敗時は `releash workflow output validate` でフォーマットを確認してから再提出する
