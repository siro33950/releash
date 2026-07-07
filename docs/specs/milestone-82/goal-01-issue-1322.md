# Goal: #1322 NodeDefinition kind block 移行

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1322 --repo siro33950/releash` で issue 本文を読むこと。

旧 `type:` を廃止し、kind block（`command` / `session` / `fanout` のちょうど 1 つ）を持つ NodeDefinition に移行する。

## 実装内容

1. **schema**（`src-tauri/src/adaptor/gateway/workflow/schema.rs`）:
   - `NodeType` enum と `type:` field を削除し、NodeDefinition を「`name` + kind block ちょうど 1 つ + 共通 field」に再構成する。
   - kind は Rust の enum（`NodeKind = Command(..) | Session(..) | Fanout(..)`）で表現し、0 個 / 2 個以上を型で表現不能にする（syntax doc「文法健全性の担保」節の方針。custom Deserialize で判別し、kind block 0 / 2 個以上は load 時 Diagnostic）。
   - `command:` は shell command scalar。
   - `session:` は `{ model?, permission, gate?(auto|approval・省略時 auto), facets?{ policy?, knowledge?, instruction? } }`。旧 node 直下の flat な `policy:` / `knowledge:` / `instruction:` 参照を `session.facets` に集約する。gate の必須化と approval 挙動の完遂は #1324 の担当（本 goal では構文と「gate: approval なら承認待ち」の既存挙動への配線のみ）。
   - `fanout:` は暫定的に `parallel_children` / `aggregate` を内包する（**field 名のみ維持**。中身の置換は #1329/#1330）。ただし子要素は design.md §2 の `InterimChild` に従い、**旧 ChildNodeDefinition から `type:` と flat facet を除去した形**（`{ name, model?, permission?, facets{...}, output_contract? 等 }`、子は暗黙に session 扱い）にする。受け入れ基準「旧 `type:` が schema / built-in に残らない」は子要素にも適用される。
   - `artifact` / `inputs` / `input` / `rules` の**位置**を Node 共通 field として確保する（意味の実装は #1325/#1326/#1327。既存 `output_contract` / `input_contracts` / `pass_output_from` / `pass_previous_response` / `rules(match/next)` / `cycle_guard` / `resets_cycle_for` / `collect` は共通 field 位置に現状のまま残す）。
   - `deny_unknown_fields` 相当で kind ごとの不許可 field を拒否する（command block に facets、session block に command 等）。
   - `inline_prompt` field を削除する（新文法に存在しない。使用している built-in / facet があれば instruction facet に移す）。
2. **旧 type: の全廃**: `type: agent` → `session`（gate: auto）、`type: approval` → `session`（gate: approval）、`type: bash` → `command`、`type: parallel` → `fanout` の対応で、domain（`domain/workflow/value_objects/definition.rs`）、mapping（`domain_mapping.rs`）、validation（`services/validation.rs`）、runtime の kind 分岐（`runtime_engine_impl.rs`、`transition.rs` の decide_turn_complete_action、approval_runtime、parallel_runtime）、event log / projection（NodeStarted 等に node_type が載る場合）、usecase DTO（`usecase/workflow/dto.rs`）、presenter / protocol（`adaptor/protocol/workflow.rs`）、frontend 型（`src/types/workflow.ts` の NodeType、`workspace-tree.ts` の stepType）を kind block 語彙に揃える。
3. **built-in 12 本**（`adaptor/gateway/workflow/builtin/*.yml`）を kind block 記法へ機械移行する（担当外表現は中身を変えない）。
4. **UI（D7）**: StepEditor / WorkflowEditor のフォーム編集を廃止し、YAML テキスト編集（Monaco）+ 保存時に backend の validation / Diagnostic 結果を表示する構成に置き換える。WorkflowDetail 等の表示コンポーネントは kind block 語彙で表示する。フォーム編集用コンポーネント・テストは削除する。
5. **未定義参照の Diagnostic**: 未定義 node 参照・未定義 Contract（現行は facet contract）参照が load 時にエラーになることを既存 validation の枠内で維持する（staged Diagnostic 化は #1323）。

## 削除対象（本 goal 完了時に残っていないこと）

- `NodeType` enum、YAML の `type:` field、`type: agent|bash|approval|parallel` の受理経路
- node 直下 flat の `policy:` / `knowledge:` / `instruction:`（session.facets へ）
- `inline_prompt`
- StepEditor 等のフォーム編集コンポーネント（D7）

## テスト

- valid / invalid fixture: kind block ちょうど 1 個だけ受理。0 個・2 個以上・kind ごとの不許可 field・未定義参照の拒否。
- 旧 `type: agent` / `type: bash` / `type: approval` / `type: parallel` を受理しない regression test。
- built-in 12 本が新 schema で load / validate できる fixture test。
- 既存 runtime テスト（tests.rs）を kind block 語彙へ更新し、agent 実行 / approval 待ち / parallel 展開の挙動が変わらないこと（挙動非変更のリファクタであることを test で担保）。
