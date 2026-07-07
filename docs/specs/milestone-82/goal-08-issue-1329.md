# Goal: #1329 parallel_children を fanout.child に移行

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1329 --repo siro33950/releash` で issue 本文を読むこと。

fanout が普通の NodeDefinition を名前参照する形に移行する。

## 実装内容

1. **schema**: `fanout: { child: <node名> | [<node名>...], items?: <literal 配列 | <node>.<field> 参照> }` に置き換え、#1322 で暫定内包した `parallel_children`（ChildNodeDefinition）を削除する。aggregate は fanout 内に暫定残置（削除は #1330）。child は top-level の command / session node を参照する（fanout の child に fanout は不可 = 再帰禁止を維持）。child 側は `input: <Contract>` でパラメータ型を宣言する（定義=child、供給=fanout）。
2. **検証**: 未定義 child 参照（WFR001）/ fanout child の leaf 違反（design.md §6 R7 = WFC006: child への通常遷移・entry・fanout の入れ子）/ items と input の不整合（WFT003: 要素型不一致・items 無しで child が input 宣言・items 有りで child が input 未宣言）を Diagnostic に。`items:` は参照文法のうち `<node>.<field>` 形とリテラル配列のみ（design.md §5）。child の `rules` は fanout 実行中は無視する（P3。Diagnostic にしない）。child は単一 `input` のみ。
3. **runtime**: parallel_runtime.rs / parallel.rs を fanout 展開に再実装（design.md §8.3）:
   - child 複数 / items なし = 別 node を並列、child 1 + items = 配列展開（件数は実行時決定）、child 複数 + items = マトリクス（item × child）。
   - 各展開は個別 NodeExecution として event log / projection / UI に出す。**fanout 専用 event は作らない**: 親も通常の NodeStarted/NodeCompleted を出し、子は NodeStarted.fanout_parent（parent_node / parent_attempt / item_index / child_index）で親に紐づく。Parallel* variant は削除（P4 により在庫互換不要）。
   - **NodeExecution の識別**: engine 採番の `node_execution_id` を NodeStarted で発行し、read model の第一級 id にする。session / command の実行環境に `RELEASH_WORKFLOW_EXECUTION_ID` / `RELEASH_NODE_EXECUTION_ID` を注入する（design.md §5。並走する同名 child への approve / output submit のアドレス基盤）。
   - `item` / `item.<field>` を child scope の入力・template 参照にバインドする（#1326 の参照規約に接続）。
   - items 0 件は子 0 個で fanout 完了、artifact は空配列、通常 rules で遷移。
   - fanout の Artifact = 子 Artifact の配列（要素: session child = 検証済み Artifact、`artifact:` 無し session child = null、command child = 標準結果 ∪ Contract fields。順序は items 順 × child 宣言順）。親の ArtifactProduced（contract = null）として記録する。**child の Artifact は親の配列にのみ格納し、node 名 map に載せない**（child 名の inputs/{{ }} 参照は WFR003）。
4. **built-in 移行**: parallel_children を使う built-in（02_implement / 03_review / 03_full-review / 05_review-fix 系）の子定義を top-level node + `fanout.child` 参照に書き換える。
5. **DTO / protocol / frontend**: ChildNodeDefinition / parallel_children / ParallelStepState 表示を fanout / child NodeExecution 表示に置き換える（step→node の全面 rename は #1331）。

## 削除対象

- `parallel_children` / `ChildNodeDefinition`（schema / domain / mapper / DTO / protocol / frontend / built-in）
- Parallel* の event variant 名（Fanout 語彙へ）
- MAX_PARALLEL_CHILDREN 等の parallel 語彙の定数（fanout 語彙へ改名して維持）

## テスト

- child 複数 / child 1 + items / child 複数 + items の 3 パターンの展開と、子が個別 NodeExecution になること。
- items 0 件（空配列 artifact で完了・通常遷移）。
- items 要素型と child input 型の不一致 Diagnostic。
- fanout 実行中に child の rules が無視されること。
- 旧 `parallel_children` を受理しない regression test。
