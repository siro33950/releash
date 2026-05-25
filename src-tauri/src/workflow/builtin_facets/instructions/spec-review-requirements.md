{{project_name}} プロジェクトの `requirements.md` を内容品質の観点でレビューする。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、`${spec_dir}/requirements.md` を参照する。

## 基本方針

Spec review は実装詳細を見ない。requirements.md が、実装前に要求として十分かをレビューする。

## 検証項目

### 1. 目的と背景

- Goal が明確か
- Background が要求の理由として読めるか
- Goal と Background が矛盾していないか

### 2. Scope / Non-goals

- 今回含めることが明確か
- 今回含めないことが明確か
- Scope が過大または曖昧でないか

### 3. Requirements / Constraints

- 満たすべき要求が具体的に列挙されているか
- 要求上の制約が必要な範囲で書かれているか
- 実装方針、内部ファイル、関数名、テスト手順が混入していないか

### 4. Success Criteria / Open Questions

- 完了判断の条件が読めるか
- Open Questions が残る場合、実装に進めない未決定事項として具体的な質問になっているか

## 判定

- **LGTM**: 要求として実装前に必要な内容が揃い、Scope / Non-goals / Success Criteria が明確で、実装詳細が混入していない
- **NEEDS_FIX**: 要求漏れ、曖昧な Scope、Non-goals 欠落、未解決の重大 Open Questions、実装詳細の混入がある
