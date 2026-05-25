{{project_name}} プロジェクトの `design.md` を内容品質の観点でレビューする。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、以下を参照する。

- `${spec_dir}/requirements.md`
- `${spec_dir}/behavior.md`
- `${spec_dir}/design.md`

## 基本方針

Spec review は実装詳細を見ない。design.md が、behavior.md を実現するための設計境界として十分かをレビューする。存在確認だけで合格させない。

## 検証項目

### 1. Behavior Coverage

- behavior.md の各 Rule が design.md の設計方針に対応しているか
- design.md が behavior.md にない新要求を勝手に追加していないか

### 2. Key Decisions

- 実装前にレビューすべき設計判断が明示されているか
- 採用理由が書かれているか
- 必要な箇所で代替案が検討されているか

### 3. Responsibility / State / Contract

- Responsibility Boundaries で担当すること / しないことが明確か
- Contracts が外部から見える契約として十分か
- State Ownership が曖昧でないか
- Data / Communication Flow が主要な境界を追える粒度か

### 4. Boundary Quality

- 越えてはいけない責務境界が明確か
- 既存の要求制約と矛盾していないか
- 実装者が判断不能になるほど粗すぎないか

### 5. Over-detail

以下が混入していたら NEEDS_FIX とする。

- 実装順序
- ファイルごとの編集手順
- helper 関数名
- 関数内処理
- 疑似コード
- 詳細な型定義やシグネチャ
- テストケース名
- security 実装の詳細

## 判定

- **LGTM**: design.md が behavior.md を実現する設計境界として十分で、責務・契約・状態 owner・flow・boundary が内容としてレビュー可能であり、実装詳細に入りすぎていない
- **NEEDS_FIX**: 内容不足、責務の穴、owner の曖昧さ、contract 不足、behavior 対応漏れ、過剰な実装詳細がある
