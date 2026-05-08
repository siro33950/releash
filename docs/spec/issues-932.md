## 要求

**種別**: 改善
**ゴール**: ワークフローエンジンの cycle_guard（ループ上限）到達時に、Failed終了ではなく指定したステップへ遷移できるようにする。併せてガードのカウントリセット機構を追加し、ビルトインワークフローのガード設定を見直す。
**背景**: 現状、cycle_guard 到達時はワークフロー全体が Failed で終了し回復手段がない。ループ上限に達しても内容が問題なければユーザー判断で先に進みたいケースがあり、Interactive ステップに遷移して「先に進む」等を選択可能にすることでワークフロー全体のやり直しを防ぐ。

### 詳細

#### 1. cycle_guard に `on_exhausted` オプションを追加

現状の cycle_guard は `max_iterations` のみで、上限到達時は一律 Failed になる。
遷移先ステップを指定できる `on_exhausted` を追加し、設定されていれば Failed ではなくそのステップに遷移する。
未設定の場合は現行通り Failed で終了する。

```yaml
cycle_guard:
  max_iterations: 2
  on_exhausted: plan_approval  # 任意。未設定なら現行通り Failed
```

#### 2. ステップ定義に `resets_cycle_for` を追加

特定のステップに到達した際に、指定したステップの cycle_guard カウントをリセットする。
on_exhausted で approval ステップに遷移した後、ユーザー判断でループに戻す場合にカウントがリセットされている必要がある。

```yaml
- name: plan_approval
  mode: interactive
  resets_cycle_for:       # このステップに到達したら指定ステップのカウントをリセット
    - plan_fix
```

#### 3. `plan_fix` のガード設定変更

`spec-driven-development.yml` の `plan_fix` ステップ:
- `cycle_guard.max_iterations` を 3 → 2 に変更
- `on_exhausted: plan_approval` を追加（上限到達時は既存の承認ステップへ）

#### 4. `fix`（コードレビュー修正）にガードを追加

`fix` ステップにはガードが未設定。新規追加する:
- `cycle_guard.max_iterations: 3`
- `on_exhausted: implementation_approval`（上限到達時は既存の承認ステップへ）

#### 5. approval ステップの判断を fix / review に伝播

approval ステップ（`plan_approval` / `implementation_approval`）でユーザーが NEEDS_FIX で拒否した場合、その判断（無視してよい観点、修正方針等）を後続の fix や review が参照できるようにする。
既存の `output_contract: review-verdict` の出力を `pass_output_from` で伝播する。

- `fix` の `pass_output_from` に `implementation_approval` を追加
- `code_review_parallel` の各子ステップの `pass_output_from` に `implementation_approval` を追加
- `plan_fix` の `pass_output_from` に `plan_approval` を追加（既に `plan_review_parallel` 経由で到達するが、approval の判断も渡す）
- `plan_review_parallel` の各子ステップの `pass_output_from` に `plan_approval` を追加

#### 6. ビルトインワークフローの適用イメージ

```yaml
# plan_fix: レビュー修正ループ（上限2回、超過時は承認ステップへ）
- name: plan_fix
  mode: auto
  pass_output_from:
    - plan_review_parallel
    - plan_spec
    - plan_approval             # ユーザーの判断（無視する観点、修正方針）
  cycle_guard:
    max_iterations: 2
    on_exhausted: plan_approval

# plan_approval: 到達時に plan_fix のカウントをリセット
- name: plan_approval
  mode: interactive
  output_contract: review-verdict
  pass_output_from:
    - plan_spec
    - plan_review_parallel      # レビュー結果（on_exhausted経由時に判断材料として必要）
  resets_cycle_for:
    - plan_fix
  rules:
    - match: NEEDS_FIX
      next: plan_fix

# plan_review の各子ステップ: ユーザーの判断を参照
- name: plan_review_completeness  # （他の子ステップも同様）
  pass_output_from:
    - plan_spec
    - plan_approval             # ユーザーの判断（無視してよい観点等）

# fix: コードレビュー修正ループ（上限3回、超過時は承認ステップへ）
- name: fix
  mode: auto
  pass_output_from:
    - code_review_parallel
    - plan_spec
    - implementation_approval   # ユーザーの判断（無視する観点、修正方針）
  cycle_guard:
    max_iterations: 3
    on_exhausted: implementation_approval

# implementation_approval: 到達時に fix のカウントをリセット
- name: implementation_approval
  mode: interactive
  output_contract: review-verdict
  pass_output_from:
    - plan_spec
    - implement
    - code_review_parallel      # レビュー結果（on_exhausted経由時に判断材料として必要）
  resets_cycle_for:
    - fix
  rules:
    - match: NEEDS_FIX
      next: fix

# code_review の各子ステップ: ユーザーの判断を参照
- name: code_review_acceptance  # （他の子ステップも同様）
  pass_output_from:
    - plan_spec
    - implement
    - implementation_approval   # ユーザーの判断（無視してよい観点等）
```

#### 7. `fix_quality_check` の既存ガードを削除

`fix` にガードを追加するため、`fix_quality_check` の既存ガード（max_iterations: 5）は不要。削除する。

## 振る舞い定義

```gherkin
Feature: ワークフローのループ上限回復

  Rule: ループ上限到達時に遷移先ステップを指定できる
    Scenario: on_exhausted 設定済みステップがループ上限に達する
      Given ステップの cycle_guard に on_exhausted が設定されている
      When ループ回数が max_iterations に達する
      Then on_exhausted で指定されたステップに遷移する

    Scenario: on_exhausted 未設定のステップがループ上限に達する
      Given ステップの cycle_guard に on_exhausted が設定されていない
      When ループ回数が max_iterations に達する
      Then ワークフローは Failed で終了する

  Rule: ステップ到達時に指定ステップのループカウントをリセットできる
    Scenario: resets_cycle_for 設定のステップに到達する
      Given ステップに resets_cycle_for が設定されている
      When そのステップに到達する
      Then 指定されたステップの cycle_guard カウントが 0 にリセットされる

    Scenario: リセット後にループ上限まで再実行できる
      Given あるステップの cycle_guard カウントがリセットされた
      When そのステップへの遷移が発生する
      Then max_iterations 回まで再実行できる

  Rule: on_exhausted の遷移先が cycle_guard 超過状態の場合は連鎖的に遷移する
    Scenario: 遷移先も cycle_guard 超過で on_exhausted 設定済みの場合は更に遷移する
      Given ステップ A の on_exhausted がステップ B を指定している
      And ステップ B も cycle_guard 超過状態で on_exhausted がステップ C を指定している
      When ステップ A がループ上限に達する
      Then ステップ C に遷移する

    Scenario: 遷移先が cycle_guard 超過で on_exhausted 未設定の場合は Failed で終了する
      Given ステップ A の on_exhausted がステップ B を指定している
      And ステップ B は cycle_guard 超過状態で on_exhausted が設定されていない
      When ステップ A がループ上限に達する
      Then ワークフローは Failed で終了する

Feature: ビルトインワークフローのループ制御

  Rule: 計画修正ループは上限2回で承認に遷移する
    Scenario: plan_fix が2回実行され plan_approval へ遷移する
      Given plan_fix がレビュー指摘の修正を実行している
      When plan_fix が 2 回実行される
      Then plan_approval に遷移する

  Rule: コード修正ループは上限3回で承認に遷移する
    Scenario: fix が3回実行され implementation_approval へ遷移する
      Given fix がコードレビュー指摘の修正を実行している
      When fix が 3 回実行される
      Then implementation_approval に遷移する

  Rule: 承認ステップ到達でループカウントがリセットされる
    Scenario: plan_approval で plan_fix のカウントがリセットされる
      Given plan_fix のループが上限に達した
      When plan_approval に到達する
      Then plan_fix のループカウントがリセットされる

    Scenario: implementation_approval で fix のカウントがリセットされる
      Given fix のループが上限に達した
      When implementation_approval に到達する
      Then fix のループカウントがリセットされる

  Rule: 承認ステップの判断が修正・レビューに伝播される
    Scenario: plan_approval の判断が計画修正・レビューに伝播される
      Given ユーザーが plan_approval で修正方針を判断した
      When plan_fix または plan_review_parallel の各子ステップが実行される
      Then ステップの実行コンテキストに plan_approval の出力（review-verdict）が含まれる

    Scenario: implementation_approval の判断がコード修正・レビューに伝播される
      Given ユーザーが implementation_approval で修正方針を判断した
      When fix または code_review_parallel の各子ステップが実行される
      Then ステップの実行コンテキストに implementation_approval の出力（review-verdict）が含まれる

  Rule: fix-review サイクルの上限制御は fix ステップに集約される
    Scenario: fix_quality_check は独自の cycle_guard を持たない
      Given fix ステップに cycle_guard が設定されている
      When fix ループ内で fix_quality_check が実行される
      Then fix_quality_check に独自の cycle_guard は適用されない

Feature: pass_output_from の参照制約

  Rule: pass_output_from は定義済みの全ステップを参照できる
    Scenario: 定義順で後方のステップを pass_output_from で参照できる
      Given ステップ A が pass_output_from にステップ B を指定している
      And ステップ B はステップ A より後に定義されている
      When ワークフローのバリデーションが実行される
      Then バリデーションが成功する

    Scenario: 後方参照先の出力が未生成の場合は空として扱われる
      Given ステップ A が pass_output_from にステップ B を指定している
      And ステップ B はまだ実行されていない
      When ステップ A が実行される
      Then ステップ B の出力は空として扱われる

Feature: on_exhausted の安全性

  Rule: on_exhausted の参照先はバリデーションで検証される
    Scenario: on_exhausted が存在しないステップを参照するとバリデーションエラーになる
      Given ステップ A の on_exhausted が存在しないステップ名を指定している
      When ワークフローのバリデーションが実行される
      Then UnknownOnExhausted エラーが返される

  Rule: on_exhausted の循環参照はバリデーションで検出される
    Scenario: on_exhausted が循環する構成はバリデーションエラーになる
      Given ステップ A の on_exhausted がステップ B を指定している
      And ステップ B の on_exhausted がステップ A を指定している
      When ワークフローのバリデーションが実行される
      Then CircularOnExhausted エラーが返される

  Rule: resets_cycle_for の参照先はバリデーションで検証される
    Scenario: resets_cycle_for が存在しないステップを参照するとバリデーションエラーになる
      Given ステップ A の resets_cycle_for に存在しないステップ名が含まれている
      When ワークフローのバリデーションが実行される
      Then UnknownResetsCycleFor エラーが返される

    Scenario: resets_cycle_for が cycle_guard を持たないステップを参照するとバリデーションエラーになる
      Given ステップ A の resets_cycle_for に cycle_guard が設定されていないステップ B が含まれている
      When ワークフローのバリデーションが実行される
      Then ResetsCycleForNonGuardedStep エラーが返される
```

## 実装仕様

**対応方針**: 振る舞い定義（ループ上限回復・カウントリセット・承認判断の伝播）を実現するために、ワークフロースキーマ（`CycleGuard`, `Step`）にフィールドを追加し、エンジンの遷移ロジック（`apply_transition`）で `on_exhausted` による代替遷移と `resets_cycle_for` によるカウントリセットを処理する。ビルトインワークフロー YAML の設定変更で計画・コード修正ループの制御を反映する。

### 対象コンポーネント

#### A. スキーマ変更 — `src-tauri/src/workflow/schema.rs`

1. **`CycleGuard` 構造体** (L142-145): `on_exhausted: Option<String>` フィールドを追加
   - `#[serde(default, skip_serializing_if = "Option::is_none")]` で後方互換性を確保
   - ループ上限到達時の遷移先ステップ名を保持。未設定なら現行通り Failed

2. **`Step` 構造体** (L12-41): `resets_cycle_for: Option<Vec<String>>` フィールドを追加
   - `#[serde(default, skip_serializing_if = "Option::is_none")]` で後方互換性を確保
   - このステップに到達した際に、指定ステップの `step_execution_counts` をリセット

#### B. エンジン変更 — `src-tauri/src/workflow/engine.rs`

3. **`CycleGuardResult::Exceeded` バリアント** (L213-220): `on_exhausted: Option<String>` フィールドを追加
   - ガード超過時に代替遷移先の情報をエンジンに伝達

4. **`check_cycle_guard` メソッド** (L362-397): `Exceeded` 返却時に `guard.on_exhausted.clone()` を含める

5. **`apply_transition` 関数** (L2608-2661): 2つのロジック追加
   - **on_exhausted 処理**: `CycleGuardResult::Exceeded` かつ `on_exhausted: Some(target)` の場合、Failed にせず `apply_transition(exec, &target)` を再帰呼び出しして代替ステップへ遷移する。`on_exhausted: None` の場合は現行通り Failed。再帰呼び出しにはステップ数を上限とする深度制限を設け、超過時は Failed とする（バリデーションで循環は検出されるが、防御的に制限する）
   - **resets_cycle_for 処理**: `CycleGuardResult::Allowed` で遷移成功した後（カウント increment の後）、遷移先ステップの `resets_cycle_for` を確認し、指定されたステップ名の `step_execution_counts` エントリを `remove` してリセット

#### C. バリデーション変更 — `src-tauri/src/workflow/validation.rs`

6. **`ValidationError` enum**: 3つのバリアントを追加
   - `UnknownOnExhausted { step, target }`: `on_exhausted` が存在しないステップを参照
   - `UnknownResetsCycleFor { step, target }`: `resets_cycle_for` が存在しないステップを参照
   - `CircularOnExhausted { cycle }`: `on_exhausted` の遷移チェーンが循環を形成（`cycle` は循環に含まれるステップ名のリスト）
   - `ResetsCycleForNonGuardedStep { step, target }`: `resets_cycle_for` が `cycle_guard` を持たないステップを参照

7. **`validate` 関数**: 各ステップの検証に追加
   - `cycle_guard.on_exhausted` の参照先が `transition_target_names`（トップレベルステップ名）に存在するか検証
   - `resets_cycle_for` の各参照先が `transition_target_names` に存在するか検証
   - `resets_cycle_for` の各参照先が `cycle_guard` を持つステップであるか検証（持たない場合は `ResetsCycleForNonGuardedStep` エラー）
   - `on_exhausted` の循環参照検出: `on_exhausted` の遷移チェーンを辿り、同じステップが2度出現した場合（循環）を `CircularOnExhausted` エラーとする

8. **`pass_output_from` の順序制約を緩和**: `pass_output_from` の参照先検証を `preceding_step_names`（先行ステップのみ）から `referenceable_step_names`（全定義済みステップ名）に変更する（L258, L364 の2箇所、および並列子ステップの検証 L247-261）。approval ステップの出力を後続の fix/review が参照する要求5のパターンでは、`pass_output_from` が定義順で後方のステップを参照する必要がある。出力が未生成の場合は空として扱われる（エンジン側の既存動作）

#### D. ビルトインワークフロー変更 — `src-tauri/src/workflow/builtin/spec-driven-development.yml`

9. **計画修正ループの変更**:
   - `plan_fix` (L66-78): `max_iterations: 3` → `2` に変更、`on_exhausted: plan_approval` を追加、`pass_output_from` に `plan_approval` を追加
   - `plan_approval` (L80-91): `resets_cycle_for: [plan_fix]` を追加、`pass_output_from` に `plan_review_parallel` を追加
   - `plan_review_*` 各子ステップ (L32-59): `pass_output_from` に `plan_approval` を追加

10. **コード修正ループの変更**:
   - `fix` (L163-171): `cycle_guard: { max_iterations: 3, on_exhausted: implementation_approval }` を追加、`pass_output_from` に `implementation_approval` を追加
   - `fix_quality_check` (L173-183): `cycle_guard` を削除（`max_iterations: 5` を除去）
   - `implementation_approval` (L185-197): `resets_cycle_for: [fix]` を追加、`pass_output_from` に `code_review_parallel` を追加
   - `code_review_*` 各子ステップ (L109-157): `pass_output_from` に `implementation_approval` を追加

### 後方互換性

- 新規フィールドはすべて `Option` + `#[serde(default)]` のため、既存の永続化済み `WorkflowState`（`workflow_definition` フィールドに `Workflow` を含む）のデシリアライズに影響しない
- 既存のユーザー定義ワークフロー YAML も `on_exhausted` / `resets_cycle_for` 未設定で動作が変わらない

### 影響するテスト

- **schema.rs テスト**: `on_exhausted` / `resets_cycle_for` を含む YAML のパースと、未設定時のデフォルト値検証
- **validation.rs テスト**: `on_exhausted` / `resets_cycle_for` の参照先検証（正常系・存在しないステップ参照のエラー系）、`resets_cycle_for` が `cycle_guard` を持たないステップを参照した場合のエラー検出、`on_exhausted` 循環参照の検出、`pass_output_from` の後方参照が許可されること
- **engine.rs テスト**:
  - `check_cycle_guard` が `on_exhausted` 付きの `Exceeded` を返すこと
  - `apply_transition` で `on_exhausted` 設定済みステップのガード超過時に代替ステップへ遷移すること
  - `apply_transition` で `on_exhausted` 未設定のガード超過時に従来通り Failed になること
  - ステップ到達時に `resets_cycle_for` で指定ステップのカウントがリセットされること
  - リセット後にループ上限まで再実行できること（リセット→再ループ→上限到達のシナリオ）
