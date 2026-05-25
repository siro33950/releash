{{project_name}} プロジェクトのコードレビューと自動修正の結果を要約する。

## 入力

- 入力で渡される Spec ディレクトリ（`spec-directory` Contract）
- レビュー結果（`review-verdict` Contract に相当、観点ごとに複数）
- 自動決定された修正方針（`approved-fix-policy` Contract に相当）
- これまでに適用された全コード変更

## 目的

本 node は **ワークフローの終端**。ユーザーへの承認問い合わせは行わず、ここまでに何が起きたかを **事後ログ** として整理する。コード変更は一切行わない（permission: ask）。

## プロセス

1. レビュー観点ごとの最終判定を確認する
   - 全観点 LGTM で到達した場合と、cycle_guard 上限で到達した場合を区別する
2. 修正方針（`approved-fix-policy`）から `action: "fix"` / `action: "skip"` の件数を集計する
3. 実際に適用されたコード変更を読み取り、修正方針との対応を確認する
4. 残課題（`action: "skip"` 扱いの指摘、cycle_guard 到達時の未消化指摘等）を列挙する
5. 品質チェック（lint / test / build）の状況を確認する

## 出力フォーマット

```markdown
## レビュー自動修正サマリー

### 終了状況
- 完了経路: 全観点 LGTM / cycle_guard 上限到達 / NO_FIX_NEEDED で完了
- 修正反復回数: N 回

### レビュー観点別の最終判定
| 観点 | 最終判定 | 主要指摘件数 |
|------|---------|------------|
| acceptance | LGTM/NEEDS_FIX | ... |
| structure | LGTM/NEEDS_FIX | ... |
| quality | LGTM/NEEDS_FIX | ... |
| test | LGTM/NEEDS_FIX | ... |
| security | LGTM/NEEDS_FIX | ... |
| architecture | LGTM/NEEDS_FIX | ... |

### 自動修正方針の集計
- fix: N 件
- skip: M 件（うち false-positive: a 件、scope外: b 件、Spec整合: c 件、重複集約: d 件）

### 実際に適用した修正
- [変更ファイルと簡単な説明の一覧]

### 残課題（後続対応推奨）
- [`action: "skip"` 扱いだが本質的に対処すべき指摘、または cycle_guard 上限で未消化の指摘]

### 品質ゲート
| チェック | 結果 |
|---------|------|
| lint | PASS/FAIL/未実施 |
| test | PASS/FAIL/未実施 |
| build | PASS/FAIL/未実施 |
```

## 出力に関する制約

- structured output は不要
- 推測で記入しない。確認できない項目は「未確認」と明記する
- 自身のフォローアップ作業（追加修正等）は行わない（permission: ask）
