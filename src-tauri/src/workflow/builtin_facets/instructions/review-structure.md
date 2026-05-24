{{project_name}} プロジェクトのコード変更を構造品質の観点でレビューする。

## 入力

- 入力で渡される Spec ファイル
- 実装ステップからのコード変更

## ファクトチェック義務

推測で指摘しない。報告前に必ず実コードを読んで検証する。

## スコープルール

| スコープ | 説明 | FAIL対象 |
|---------|------|---------|
| scope:diff | 今回の変更で導入された問題 | はい |
| scope:touched | 変更ファイル内の既存問題 | 報告のみ |
| scope:external | 変更対象外の問題 | 報告しない |

## 担当範囲（観点の一意分配）

本モジュールは **関数・クラス・モジュール単位** の構造品質を担当する。
レイヤー間の責務境界（Domain / Usecase / Infrastructure 等）は review-architecture が担当する。

## 検証手順

### 1. 抽象化とモジュール性

- コードが適切な粒度で分割されているか
- 抽出すべき重複コードがないか（DRY）
- 設計が可能な限りシンプルか（KISS）

### 2. 責務と凝集度（関数・クラス・モジュール単位）

対象: 関数・クラス・モジュール単位の責務分離

- 各関数・クラス・モジュールが単一の明確な責務を持つか
- 関連する要素が同じ関数・クラス・モジュール内にグループ化されているか

※ レイヤー間の責務境界（Domain / Usecase / Infrastructure 等）の検証は review-architecture（レイヤー責務）が担当。

### 3. 結合度

- モジュール間の依存関係が最小化されているか
- 不要な密結合がないか

### 4. 循環依存

- コンポーネント間の循環依存がないか

### 5. デッドコード

- scope:diffに到達不能なコードパス、未使用の関数、型、変数がないか
- 報告前にコード検索で検証する

### 6. 未使用インポート・変数

- scope:diffに未使用のインポートや変数がないか

### 7. API・インターフェース設計

- インターフェースの粒度が適切か
- 公開APIが最小限か

## 指摘フォーマット

各指摘にはファイル:行、問題の説明、修正提案を含める。

## 判定

- **LGTM**: scope:diffの全構造チェックがパス
- **NEEDS_FIX**: scope:diffのいずれかが要改善

## 構造化出力の提出

判定（`review-verdict` Contract に従う JSON）は、step 完了時に `releash workflow output submit` で engine に提出する。

```sh
releash workflow output submit <run_id> \
  --step <step_name> \
  --type review-verdict \
  --json '{"verdict":"LGTM"}'   # または NEEDS_FIX の場合は findings を含める
```

提出が成功するまで step は完了として扱われない。失敗時は `releash workflow output validate` でフォーマットを確認してから再提出する。
