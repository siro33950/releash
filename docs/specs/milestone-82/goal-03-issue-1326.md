# Goal: #1326 Artifact 入力と参照規約

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1326 --repo siro33950/releash` で issue 本文を読むこと。

旧入力注入（`pass_output_from` / `pass_previous_response` / `workflow_variables`）を `inputs:` に置き換え、`request` / `item` / `{{ ... }}` の参照規約を実装する。

## 実装内容

1. **inputs:**: node 共通 field `inputs: [<node名> | request, ...]` を実装。参照先の Artifact（#1325 の保管機構）を session の prompt 組み立て（`prompt_rendering.rs`）へ JSON として注入する。未定義 Artifact 参照は Diagnostic。
2. **request 予約 Artifact**: 起動時 task 自由文字列を初回 Artifact `request`（String scalar・予約名）にする。
   - `StartRunCommand.task` を request として event log に載せる（RunStarted への field 追加。P4 により在庫互換は不要）。
   - `request` は Node 名ではなく予約 Artifact 名として解決し、`inputs: [request]` と `{{ request }}` が同じものを指す。
   - 未指定時は空文字列 request とする（P12）。
   - `request` / `item` を Node 名として使う定義は Diagnostic。
3. **item 参照**: `item` / `item.<field>` を fanout child scope でのみ有効な予約参照として resolve 検査に実装する（runtime での実バインドは #1329。ここでは scope 検査と renderer 対応まで）。
4. **template 補間の統一**: renderer（`variable_renderer.rs`）を `{{ request }}` / `{{ <node>.<field> }}` / `{{ item.<field> }}` のみ受理する実装に置き換える。D6 により `{{ task }}` / `{{ project_name }}` / `{{ path_alias.* }}` / `{{ vars.* }}` / workflow `variables:` セクション / storage.rs の undefined-variable 検証 / WorkflowState.workflow_variables を全廃する。未定義 Artifact path・scope 違反（fanout child 外の item 等）は Diagnostic。
5. **built-in 移行**: pass_output_from / pass_previous_response / variables 依存を `inputs:` + `{{ <node>.<field> }}` に書き換える。特に authoring→implement の spec_dir 連携は「authoring node が spec_dir field を持つ Artifact を産出（schemas に宣言）→ 後続 node が inputs + `{{ <node>.spec_dir }}` で参照」に再設計する（D6）。instructions facet 本文の旧 template 変数も書き換える。
6. **DTO / protocol / frontend**: pass_output_from / pass_previous_response / workflow_variables を全層から撤去。Artifact path / scope の解決は Rust 側で行い、UI は解決済み結果の表示のみ。

## 削除対象

- `pass_output_from` / `pass_previous_response`（schema / domain / renderer / prompt_rendering / DTO / protocol / frontend / built-in）
- workflow の `variables:` セクション、`workflow_variables`（state / projection / protocol）、`{{ vars.* }}` / `{{ task }}` / `{{ project_name }}` / `{{ path_alias.* }}`、SYSTEM_TEMPLATE_VARIABLES、contract 出力からの workflow_variables 抽出（extract_workflow_variables_from_contract_output）

## テスト

- `inputs: [request]` / `inputs: [<node>]` の注入、`{{ request }}` / `{{ <node>.<field> }}` / `{{ item.<field> }}` の展開。
- `request` / `item` の scope 違反・未定義 Artifact path が Diagnostic になること。
- start の task 文字列が request Artifact になり `inputs: [request]` で読めること（未指定 = 空文字列）。
- 旧 `{{task}}` / `pass_output_from` / `pass_previous_response` / `variables:` を受理しない regression test。
- built-in（特に authoring→implement 連携）が新参照で load / 実行できること。
