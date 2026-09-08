# Design 01

## 開始状態

初回。base は `main`、派生点は `94963548d`（#1731 取り込み後）。branch `feat/issues/1732` は派生点と同一 commit で、未コミットの変更は `docs/specs/issues-1732/` の新規文書だけである。差分の基準は `docs/specs/issues-1732/requirements.md` の Current Behavior 節が記録する実装状態で、直前の Design はなく、open Thread もない。

Current Behavior が記録する挙動の現行の所在は次のとおりである。

- `completion` の型は domain の `NodeCompletion`（`src-tauri/src/domain/workflow/value_objects/definition.rs`）で、`Auto`（既定）/ `Approval` の serde enum である。YAML の deserialize（`RawNodeBody`）、gateway の schema 型（domain 型の再エクスポート）、実行木 root の開始事実に保存する定義 snapshot の serde（`node_fact.rs` の `workflow_definition_snapshot_serde`。serialize 時は `Auto` を省略する）は、すべてこの一つの型を通る。
- 承認の実行時判定は `NodeDefinition::requires_approval_completion()` の一箇所で、`services/transition.rs` と `entities/workflow_execution` が参照する。
- Lua は `lua/mod.rs` の `parse_completion` が `nil` を `Auto`、handle `r.completion.approval` を `Approval` とし、それ以外を `WFS002`（`field 'completion' must be Completion`）にする。stub は `lua/stubs.rs` が `<workflows_dir>/.releash/releash.lua` へ生成し、`ReleashCompletion` / `ReleashCompletionModule` と各 Node options の `completion? ReleashCompletion` を注釈する。
- 定義の read model は `usecase/workflow/dto.rs` の `NodeCompletionDto`（`auto` / `approval`）で、frontend の `src/types/workflow.ts` が同じ文字列 union を持つ。badge は `src/components/panels/automation/WorkflowDetail.tsx`、設定画面の説明文は `src/components/panels/SettingsModal.tsx` にある。
- YAML で `completion` に map を書いたときの `WFS002` は serde の deserialize 失敗を `workflow shape error:` に続けた message であり、Lua の `WFS002` は上記の固定文である。開始状態では両表面の message は一致していない。

## 変える部分

- `NodeCompletion` 型の意味の変更: domain の `completion` の型を「要求の有無」を表す型に変え、`auto` という値を持たせない。YAML の deserialize と定義 snapshot の serde はこの型を通るため、`require: approval` を持つ map の受理と、`completion` 省略時の「要求なし」はここで決まる。根拠: R-001「`completion` は map として宣言でき、`require: approval` を持つ map を…全4種の Node で受理する」、R-003「`completion` を宣言しない Node は、本来の完了条件を満たした時点で完了する」、Issue #1732 本文「`NodeCompletion` 型を『要求の有無』に変える」。ルート: 型の意味だけ固定（下記「固定するルート」）。配置・表現は委任。
- YAML 表面の `completion` の受理形: `require: approval` を持つ map を全4種の Node で受理し、文字列形式（`approval` / `auto`）、`approval` 以外の `require`、`require` 以外のキー、要素を持たない map を load 時の Error Diagnostic にする。根拠: R-001 / B-001、R-004「文字列形式の `completion: approval` と `completion: auto` は load 時に Error Diagnostic になり」/ B-004、R-005「`completion` を書くなら要求を一つ以上持つ」/ B-005。ルート: 委任。Diagnostic の code / stage / message は委任し、YAML と Lua で同じ値を使うことだけが要求（R-006）。
- 承認の判定の接続: `requires_approval_completion()` の判定を、新しい型の「`require: approval` を持つか」に置き換え、実行時の承認経路は動かさない。根拠: R-002「この挙動は、変更前に `completion: approval` を宣言した Node の挙動と同じである」/ B-002、R-003 / B-003。ルート: 委任。
- Lua 表面の `completion` の受理形: `completion = { require = r.completion.approval }` の table を受理し、既存 handle `r.completion.approval` を `require` の値として存続させる。table で包まない `completion = r.completion.approval` と、R-005 が YAML で拒否する誤りの Lua 表記は、YAML と同じ code / stage / message の Error Diagnostic にする。生成 stub の型注釈も table 形に合わせる。根拠: R-006「YAML の `completion` map と同形の table `completion = { require = r.completion.approval }` で…同じ受理結果と同じ完了の挙動になる」/ B-006、`requirements.md` Context「Lua 表面の受理形を変えると stub も同時に変わる」。ルート: 表記の形は下記「固定するルート」で固定。table の解析の実装方法は委任。
- 定義の read model の `completion`: DTO から `auto` の値をなくし、`require: approval` の宣言の有無を変更後の受理形と一致する形で示す。frontend の型定義もそれに合わせる。根拠: R-010「廃止した `auto` は read model の値として残らない」/ B-010。ルート: 委任。JSON の具体的な形は委任。
- UI の文言: workflow 定義詳細の Node の badge と、設定画面の Approval auto-approve の説明文を、`completion: approval` の文字列形式の引用から `require: approval` の宣言を示す文言に変える。根拠: R-010 / B-010「設定画面の Approval auto-approve の説明文は `completion: approval` の文字列形式を引用せず」。ルート: 委任。
- 正本サンプルと builtin の書き換え: `workflows/examples/full-cycle-development.yml` の2箇所（`spec_confirmation` / `implementation_confirmation`）と、builtin 5本の7箇所（`01_author-spec` 1、`02_implement-existing-spec` 1、`04_review-fix-policy-manual` 2、`06_handle-pr-review` 1、`06_handle-pr-review-manual` 2）の `completion: approval` を、`completion:` の下に `require: approval` を置く map 形（`docs/specs/milestone-85/design.md` §2.3 の形）に書き換える。`03_full-review` / `04_review-fix-policy` / `05_review-fix` には宣言がなく変更箇所はない。根拠: R-007「builtin 8本と正本サンプルは本変更後も Diagnostic ゼロで load できる」/ B-007。ルート: 委任。
- `docs/glossary/WORKFLOW.md`: Node 共通 field の表の `completion` 行、「Session」節の例と `completion: auto` / `completion: approval` の説明、「completion」節の Node 種別ごとの表、「Lua」節の例の `completion = r.completion.approval`、「Lua API」の表の Node builder の `completion?` と `r.completion.approval` の行を、変更後の受理形と Lua の書き方に合わせる。根拠: R-008「`auto` / `approval` の文字列形式を前提とする記述は残らない」/ B-008。ルート: 委任。
- `docs/glossary/DOMAIN.md`: 正規語の表の `completion` 行を、Node の完了に対する要求の集合であり、要求として `require: approval` を持ち、要求を書かないことが自動完了を意味する定義にする。根拠: R-009 / B-009。ルート: 委任。

## 固定するルート

- Issue #1732 本文の指定: `NodeCompletion` 型を「要求の有無」に変える。粒度は型の意味だけで、型の配置、表現（enum か struct か、Option か）、DTO の形は委任する。理由: `auto` を廃止し、要求を書かないことが自動完了を意味する構造を型で表す。関係: R-001（map としての受理）、R-003（省略時の自動完了）、R-010（read model に `auto` を残さない）。
- Lua 表面の受理形（Q-001 の決定）: YAML の `completion` map と同形の table で受理し、値は既存 handle `r.completion.approval` とする。粒度は表記の形まで。table で包まない `completion = r.completion.approval` は受理しない。理由: YAML と同じ map 構造で書け、既存 handle と生成 stub の型注釈を残せ、旧表記の誤りを YAML の文字列形式と同じ Diagnostic に揃えられ、#1734 は同じ table に `delegate` を足すだけで済む。関係: R-006 / B-006。

    ```lua
    local implement = r.session{
      provider = r.provider.claude,
      facets = { instruction = f.instruction.implement },
      completion = { require = r.completion.approval },
    }
    ```

## 変えないもの

- 承認の実行時経路（Approve / Reject の操作、Approval auto-approve 設定、WaitingApproval の状態遷移、承認事実の記録）と、各 Node 種別の本来の完了条件（Session の二信号、Command の process 終了、Fanout の全 child 決着、Sequence の終端到達）。実装上は `requires_approval_completion()` を参照する `services/transition.rs` と `entities/workflow_execution` の遷移をそのまま使う。理由: 本 Issue は宣言の構造だけを変える。
- 旧形式（文字列 `completion`）の解釈経路と自動移行を追加しない。YAML / Lua の loader にも、定義 snapshot の serde 経路にも、文字列形式の fallback を置かない。理由: 用語集「未知 field、旧形式、互換 alias は受理しない」と #1744「旧記法の実行規則や自動移行は追加しない」を維持する（Q-002）。

## 未確定・リスク

- 文字列形式の拒否、`approval` 以外の `require`、未知キー、空 map の拒否は、YAML では serde の shape 検査、Lua では table 解析と経路が分かれる。開始状態では YAML の message が serde 生成、Lua の message が固定文で一致していない。R-006 は同じ定義上の誤りに同じ code / stage / message を求めるため、両経路の Diagnostic を揃える必要がある。
