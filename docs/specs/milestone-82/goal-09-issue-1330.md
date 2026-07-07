# Goal: #1330 aggregate 廃止と reducer node 移行

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1330 --repo siro33950/releash` で issue 本文を読むこと。

fanout / rules から集約機構を完全に排除する。

## 実装内容

1. **aggregate 削除**: `ParallelAggregate`（all_match / any_match / then / else）を schema / domain / mapper / runtime（parallel.rs の評価、failure_policy.rs の ModelRefusal→aggregate 委譲）/ event（FanoutCompleted の aggregate_result field）/ DTO / protocol / frontend から削除する。fanout 完了後の遷移は fanout node 自身の通常 `rules` だけで決まる。
2. **collect / ReduceStrategy 削除（P8）**: `CollectConfig`（from / reduce: Last|Concat|Grouped|AnyNeedsFix|AllPassed）、OutputCollected event、関連 runtime を削除する。
3. **reducer node パターン**: fanout の Artifact（子 Artifact 配列）を `inputs:` で受けた通常 node（command / session）が boolean / enum Artifact に畳み、通常 rules で分岐する経路を fixture で確立する（full-pipeline.yml の `judge` node パターン。jq を使う command reducer と session reducer の両方を fixture 化）。
4. **built-in 移行**: aggregate を使う built-in は 03_review と 03_full-review のみ（02_implement 系は parallel_children のみで aggregate 不使用）。既存 aggregate は全て then == else（`all_match: ".*"` で両分岐が同一 node）で分岐機能を果たしていないため、**reducer 挿入ではなく fanout node の通常 `rules`（next）化で足りる**。reducer node パターンの確立は実装内容 3 の fixture 側で行う。必要な schemas 宣言と instructions facet の文面更新を含む。
5. **失敗時の扱い**: 子の一部失敗の扱いは fanout 固有 policy を持たず #1335 の resume に委ねる（aggregate 委譲していた ModelRefusal は通常の node 失敗として扱う）。

## 削除対象

- `aggregate` / `all_match` / `any_match` / `ParallelAggregate`
- `collect` / `CollectConfig` / `ReduceStrategy` / OutputCollected event
- failure_policy の aggregate 委譲分岐

## テスト

- fanout 結果（子 Artifact 配列）を command / session reducer で boolean / enum に畳み、通常 rules で分岐できること。
- 子 Artifact 配列が fanout の Artifact になること（#1329 の確立を regression 維持）。
- 旧 `aggregate` / `all_match` / `any_match` / `collect` を受理しない regression test。
- built-in（レビュー系）が reducer 構成で load / 実行できること。
