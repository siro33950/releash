# Goal: #1323 文法健全性 / Diagnostic front-end

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1323 --repo siro33950/releash` で issue 本文を読むこと。

WorkflowDefinition の検証を parse/shape → resolve → typecheck → control-flow の段階に分け、structured Diagnostic（code / span / message）として返す front-end に統合する。

## 実装内容

0. **Spike（最初に実施・結果を報告に含める）**: serde_saphyr / saphyr で YAML span（行・列）を取得する方法を確定する。推奨: typed load（serde_saphyr）に加えて saphyr AST を並行 parse し、YAML path → span の map を構築する二段方式（P9）。span が取れない階層は最近傍 node の span に fallback。
1. **Diagnostic 型の刷新**: 既存 `DiagnosticItem`（severity/message のみ、`adaptor/gateway/workflow/diagnostics.rs`）を、`{ code, severity, span?, message, stage }` を持つ新 Diagnostic に置き換える。code は安定識別子（例: parse/shape=WFS***, resolve=WFR***, typecheck=WFT***, control-flow=WFC*** の系統で採番し、テストで固定）。Diagnostic は validation result であり lifecycle state にしない。
2. **段階分割**: #1322/#1325/#1326/#1327 で実装済みの検証（validation.rs の ValidationError 群、schema の構造検査、参照解決、routing 型検査、排他・網羅・ループ検査）を 4 段階の pipeline に再編する:
   - parse/shape: YAML 構文、unknown field、kind block 個数、kind ごとの許可 field、旧 field の拒否
   - resolve: node 名、Contract 名、Artifact path、予約名 `request` / `item` の scope
   - typecheck: `when.on` boolean / `switch.on` enum、fanout `items` と child `input` の型一致（#1329 実装後に接続。現段階では検査枠だけ用意）、`artifact` / `input` の Contract 存在
   - control-flow: 終端 node、到達不能 node、cycle と loop_guard、rules の排他・網羅
3. **無効 workflow は runtime に入らない**: load 境界（storage / definition_repository / builtin / start 経路）で Diagnostic ありなら実行を拒否する。
4. **UI / CLI は表示のみ**: diagnose 系 Tauri command と automation panel の Diagnostic 表示を新形式（code/span/message）に更新。Monaco の YAML 編集（D7）に span を使ったインラインマーカー表示を付ける。CLI の validate / list 系出力も新 Diagnostic を表示するだけにする。frontend に検証ロジックを持たない。
5. **fixture suite**: `valid/` と `invalid/` の YAML fixture ディレクトリを新設し、invalid fixture は期待 Diagnostic code を固定して検証する。既存の schema.rs 内 inline fixture テストを fixture suite に整理統合する。

## 削除対象

- 旧 `DiagnosticItem` 形式（severity/message のみ）と、その frontend 型（`src/types/workflow.ts` の対応部分）
- validation 結果を lifecycle state / 文字列 error 経由で流していた経路（Diagnostic に一本化）

## テスト

- 4 段階それぞれが structured Diagnostic（code/span/message）を返すことを段階別に検証。
- valid / invalid fixture suite（invalid は Diagnostic code 固定）。
- property test: validator が valid とした workflow では任意の routing 対象値に対して遷移先がちょうど 1 つ。
- built-in 12 本が Diagnostic ゼロで load できること。
