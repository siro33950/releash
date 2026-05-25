{{project_name}} プロジェクトの `behavior.md` を内容品質の観点でレビューする。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、以下を参照する。

- `${spec_dir}/requirements.md`
- `${spec_dir}/behavior.md`

## 基本方針

Spec review は実装詳細を見ない。behavior.md が requirements.md の要求を、実装詳細なしの観測可能な振る舞いとして表現できているかをレビューする。

## 検証項目

### 1. Requirements Coverage

- requirements.md の各要求に対応する Rule / Scenario があるか
- requirements.md にない振る舞いが behavior.md で追加されていないか

### 2. Rule / Scenario の明確性

- 各 Rule がビジネスルールまたは観測可能な状態変化として一意に読めるか
- Given / When / Then が actor 視点の言葉で書かれているか
- 主観的な表現が判断基準なしに使われていないか

### 3. 実装詳細の混入

以下が混入していたら NEEDS_FIX とする。

- UI 部品名やクリック手順
- API、DB、Tauri command、WebSocket message などの技術呼び出し
- レスポンスコード、具体的画面文言、テスト assertion
- ファイル名、関数名、内部型、実装手順

## 判定

- **LGTM**: requirements.md の要求が behavior.md に過不足なく表現され、実装詳細が混入していない
- **NEEDS_FIX**: 要求カバレッジ漏れ、scope creep、曖昧な Rule、実装詳細の混入がある
