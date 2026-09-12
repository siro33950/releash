# Design 02

## 開始状態

直前の Design は `docs/specs/issues-1767/design-01.md`。その周の実装が作業ツリーに未コミットで入った状態を基準にする。

- 差分の基準: base ブランチ `main`、派生点 commit 8f6a107a（`release: v0.4.12 (#1756)`）。
- 未コミットの変更は `docs/glossary/WORKFLOW.md`、`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs`、同 `mod_test.rs`、同 `stubs.rs`、同 `stubs_test.rs`、`src-tauri/src/infrastructure/lua/evaluator.rs`、および `docs/specs/issues-1767/` の文書。
- Design 01 の「変える部分」のうち、Session handle の `delegate` メソッド（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:721-727` の Session 限定の束縛と `call_delegate`）、`child` 段の走査、`require` との and 合成、delegate だけから参照される child の到達、生成 stub の注釈、用語集の更新は実装済みである。
- この周までに解消・見送りとなった Thread は無い。`[FIX_POLICY]` を付けた open Thread が4件ある。

## 変える部分

- 無名 Session の自己 Artifact 供給: `name` を省略した合成子配下の Session でも、delegate の `inputs` に親自身の Artifact（handle そのもの、`work.<field>...`、`work.child.<field>...`）を与えた定義が Error Diagnostic なしで load でき、同じ配線を書いた YAML 定義と同じ解決になる。名前を省略した fixture を含むテストを置く。開始状態では、無名 Node に `NodeNamespace::register_synthesized` が `#` を含む合成名を割り当て（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:1800-1827`）、`source_ref` がその名前を `InputSourceRef` の raw 値にするため（同 `:2029-2046`）、`FieldPath::from_reference` が `#` の段を拒否して `InputWiringKind::InvalidSourceFormat` から `WFR007` になる。加えて `src-tauri/src/domain/workflow/services/validation.rs:979-986` は delegate の `sibling_names` から合成名を除外している。根拠: R-003、B-004（いずれも Session の `name` の有無を条件にしていない）、Thread 7b6d4c5b-ddd1-4c25-9c57-71d9434abe10。ルート: 委任。
- delegate の `inputs` の順序による定義不一致: `inputs` が複数件で YAML 側のキー記述順がキー昇順と異なる場合も、Lua と YAML の `WorkflowDefinition` が同一になる。その順序差を含む定義同一性テストを置く。開始状態では `LuaTableData.entries` が `BTreeMap` であるため `parse_inputs` の走査順がキー昇順になり（`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:1117-1141`）、`SessionDelegate.inputs` は `Vec<(String, InputSourceRef)>` で順序が等価比較に影響する。YAML 側は `InputsMapSeed` が読み取り順で push する。根拠: R-009、B-011、Thread 90a8b38f-3003-4eb5-ac6e-8e89e011e366。ルート: 委任。
- 生成 stub の型と実行時 API の一致: 生成 stub の型が Session と非Session の実行時 API の差を表し、Command / Sequence / Fanout の builder の戻り値型に呼び出し可能な `delegate` と `child` 段が付かない。再生成でその形になることをテストで確認できる。開始状態では `ReleashNode` が `delegate` と `child` を持ち、`ReleashModule` の `command` / `session` / `fanout` / `sequence` の4 builder すべてが `ReleashNode` を返す（`src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs:62-67,186-189`）。根拠: R-013、B-015、および実行時の成立範囲を Session handle に限る R-015 / R-005、Thread cb7f8c30-4cb0-4f25-af2b-3158395c99a3。ルート: 委任。
- 非Session handle からの delegate 宣言の検証: Command / Sequence / Fanout の handle で `delegate{...}` を呼ぶ Lua 入力が受理されないことを、Lua 入力から観測できるテストを `src-tauri/src/adaptor/gateway/workflow/lua/` に置く。開始状態の `mod.rs:720-727` は `NodeDraftKind::Session` のときだけ `FN_DELEGATE` を束縛し、非Session では同じキーが Artifact field の Source になるが、追加済みの delegate fixture は owner を常に名前付き Session としており、この分岐を通らない。根拠: R-015（この周に主語を Session handle へ狭めた）、B-017、`docs/architecture/TEST.md:26` の `adaptor/gateway/` 必須、Thread 004e51c5-4da0-4fa3-90e1-a69d4c5ed6aa。ルート: 委任。

## 固定するルート

今周に新たに固定する実装上の指定は無い。Design 01 で固定した次のルートは今周も維持する。

- delegate の宣言箇所は Lua の `completion` table ではなく Session handle のメソッド。
- 生成 stub の出力先は `.releash/releash.lua`。
- 文書の更新先は `docs/glossary/WORKFLOW.md` の「Lua」節と「Lua API」節。
- テストは `src-tauri/src/adaptor/gateway/workflow/lua/` に置く。

## 変えないもの

- 生成 stub の出力先 `.releash/releash.lua`。理由: Design 01 で固定したルートであり、今周の Thread cb7f8c30-4cb0-4f25-af2b-3158395c99a3 は型の表し方だけを対象とする。

## 未確定・リスク

- 自動判断（Q-001、R-011）: 同じ Session handle への2回目以降の delegate 宣言は、YAML に対応する誤りが存在しないため、Lua 表面の shape の誤りとして既存の `WFS002` / `parse_shape` を再利用する。新しい `code` は追加せず、`message` の文言は固定していない。
- 自動判断（Q-002、R-015）: Artifact Contract 直下に `delegate` field を宣言した Session でも、Lua の `work.delegate` は handle のメソッドへ解決する。帰結として、その field を Lua の値参照の供給元にはできない。
- 未決のまま残した要求は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
