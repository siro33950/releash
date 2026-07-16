# Goal: #1337 Workflow YAML 文法の最終 cleanup と正本化

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1337 --repo siro33950/releash` で issue 本文を読むこと。

完成した実装を突き合わせて残骸を削除し、`docs/workflow-yaml-syntax.md` を正本化する。本 goal は milestone 82 の最終 wave であり、#1322〜#1335 が全てマージ済みであることを前提とする。

## 実装内容

1. **残骸の掃討**: issue 記載の削除対象一覧（`type:` 系 / approval node / bash node / parallel_children / aggregate・all_match・any_match / output_contract・input_contracts / pass_output_from・pass_previous_response・workflow_variables / rules.match・regex routing・式言語 / node 直下 cycle_guard / reject・rerun / fanout 固有 failure policy / tasks[]・releash task・global tasks / Trigger・timer 混入）を、実装 / built-in / examples / docs / fixtures / frontend 全体で grep し、残っていれば削除する。旧構文が loader / Diagnostic で拒否されることを fixture test で網羅確認する。
2. **残すべき構文の過不足確認**: issue の「残すべき構文」リスト（kind block、共通 field、gate/facets、command 標準結果、stdout-JSON Artifact、schemas/artifact/input、request/node/item 参照、fanout matrix、rules 4 種と検証、Diagnostic 5 段階境界）が実装・docs・examples で過不足なく説明できることを確認し、欠けがあれば実装または文書を補う。
3. **正本例の修正**: `docs/examples/full-pipeline.yml` を新文法の完成形として整える:
   - `fix_one` の `artifact: fix_result` に対応する `fix_result` Contract を `schemas:` に宣言する。
   - `permission: read` を現行許可値（ask/edit/full）に修正する（D5）。
   - **routing 参照 field（`lgtm` / `all_lgtm` / `has_open` / `verdict`）に `required` を宣言する**（D2 の routing 制約を満たすため。plan.md リスク5）。
   - 実際に loader で load / validate が通る fixture test（Diagnostic ゼロ）に加え、**「実行できる」の検証**として session を stub 化した engine 統合テストで full-pipeline の全遷移経路（command routing・fanout・approval gate・loop_guard・switch）を通す。
4. **`docs/workflow-yaml-syntax.md` の正本化**: 「設計案」ステータスを外し、「未確定」「懸念」「検討事項」節を実装済みの決定（D2 schemas subset / D3 単一名前空間 / P3 child rules 無視 / P11 missing-field 意味論 / permission 3 値 / 空 items / 単一 input / failure は resume）に置き換えて、完成形文法の正本として読める状態にする。**Contract 節に「routing 参照 field は required 必須」を、rules 節に「catch-all `next` は when/switch と同一要素の sibling キー、単独 next 要素は判別 rule なしの無条件遷移のみ」（design.md §6 の正規形）を明記する**。grammar / validation / runtime behavior / execution trigger の境界が文書上も分かれていることを確認する。hang した command が stall observation の対象外である点、shell への `{{ }}` 補間の quoting 制約も既知の制約として文書化する。
5. **stale 文書の処理**: `docs/workflow-engine-model-boundary.md` は旧 north star（WorkflowRun / type: 語彙、存在しないモジュールパス）である。新モデル完成を反映して全面改訂するか削除するかを判断し、削除する場合は理由を PR 説明に明記する（evolution-plan / GLOSSARY / syntax doc への集約を推奨）。evolution-plan のマイルストーン節も完了状態を反映して整合させる。
6. **横断完了条件の最終確認**: built-in 12 本と full-pipeline.yml が新文法だけで load / 実行できること、旧 YAML / 旧 NDJSON / 旧 workflow state 互換が外部 API と loader に残っていないこと、一時 adapter が残っていないことを確認し、確認結果を報告に列挙する。

## テスト

- docs / examples / built-in / fixtures に旧構文・スコープ外構文が残っていないことの grep / fixture test。
- 旧構文が loader / Diagnostic で拒否されることの網羅 regression suite（削除対象一覧の全項目）。
- full-pipeline.yml が Diagnostic ゼロで load できる fixture test。
- 文法ドキュメントに記載の全構文が fixture でカバーされていることの突き合わせ（レポートとして報告）。
