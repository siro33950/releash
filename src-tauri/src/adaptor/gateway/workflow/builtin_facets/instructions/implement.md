Spec 3文書に基づいて {{project_name}} プロジェクトのコードを実装する。

## 入力

以下を参照する。

- `docs/specs/{{project_name}}/requirements.md`
- `docs/specs/{{project_name}}/behavior.md`
- `docs/specs/{{project_name}}/design.md`

## 基本方針

実装は requirements.md / behavior.md / design.md に従う。Spec は実装前の要求・振る舞い・設計判断であり、実装詳細ではない。design.md に書かれていない細部は、プロジェクトの既存パターンに従って実装時に判断してよい。

## プロセス

### 1. Spec の把握

- requirements.md から Goal / Background / Users・Actors / Requirements / Constraints / Scope / Non-goals を把握する
- behavior.md から各 Rule のビジネスルール・状態変化を把握する
- design.md から The actual design（Architecture / Interface / Data Model / Database / UI/UX / Algorithm / Infra）/ Alternatives Considered / Cross-cutting concerns / Risks を把握する

### 2. 全体設計

- Spec を踏まえて、実装の全体方針を設計する
- 変更対象ファイル・モジュールを特定する
- 既存パターン・アーキテクチャ規約との整合を確認する
- 複数要求・複数 Rule の依存関係を整理する

### 3. 実装

- 全体設計に沿って実装する
- Spec の Scope を超えた機能を追加しない
- behavior.md の Rule をコードで満たす
- design.md の設計判断を守る
- Spec にない明らかな品質問題を作り込まない
- 既存のアーキテクチャ、コーディング規約、テスト方針を守る

### 4. 自己検証

- requirements.md の各要求が満たされているか確認する
- behavior.md の各 Rule がコード上で成立するか確認する
- design.md の設計判断とコードが整合するか確認する

## 完了報告

実装した内容、実行した検証、未実行の検証があれば理由を簡潔に報告する。
