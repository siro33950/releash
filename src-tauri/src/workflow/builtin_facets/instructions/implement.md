承認済み Spec 3文書に基づいて {{project_name}} プロジェクトのコードを実装する。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、以下を参照する。

- `${spec_dir}/requirements.md`
- `${spec_dir}/behavior.md`
- `${spec_dir}/design.md`

## 基本方針

実装は requirements.md / behavior.md / design.md に従う。Spec は実装前の要求・振る舞い・設計境界であり、実装詳細ではない。design.md の Implementation Freedom に含まれる細部は、プロジェクトの既存パターンに従って実装時に判断してよい。

## プロセス

### 1. Spec の把握

- requirements.md から Goal / Scope / Non-goals / Requirements / Constraints / Success Criteria を把握する
- behavior.md から各 Rule のビジネスルール・状態変化を把握する
- design.md から Responsibility Boundaries / Contracts / State Ownership / Boundaries を把握する

### 2. 実装

- Spec の Scope を超えた機能を追加しない
- behavior.md の Rule をコードで満たす
- design.md の責務境界と contract を守る
- Spec にない明らかな品質問題を作り込まない
- 既存のアーキテクチャ、コーディング規約、テスト方針を守る

### 3. 自己検証

- requirements.md の各要求が満たされているか確認する
- behavior.md の各 Rule がコード上で成立するか確認する
- design.md の境界違反がないか確認する
- 必要なテストを追加・更新する
- lint / test / build を可能な範囲で実行する

## 完了報告

実装した内容、実行した検証、未実行の検証があれば理由を簡潔に報告する。
