# Goal: #1327 rules を順序非依存 Diagnostic に移行

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1327 --repo siro33950/releash` で issue 本文を読むこと。

旧 regex `match`/`next` と node 直下 `cycle_guard` を廃止し、`when` / `switch` / `next` / `loop_guard` の順序非依存 rules に移行する。

## 実装内容

1. **schema**: rules 要素を tagged enum（`When { on, then }（+ 同要素の next）` / `Switch { on, cases }` / `LoopGuard { max_iterations, on_exhausted }` / `Next(<node>)`）として定義（syntax doc の要素形どおり、deny_unknown_fields、判別キー集合は互いに素）。旧 `TransitionRule(match/next)` と `CycleGuard`（node 直下）、`resets_cycle_for` を削除する。
2. **型付き検証**: `when.on` は自 node Artifact の boolean field、`switch.on` は enum field を bare 名で参照（#1325 の routing 可能 field 判定に接続）。`next` は catch-all。検証（load 時 Diagnostic）:
   - 排他: どの Artifact 値も 2 つ以上の rule に当たらない。
   - 網羅: 全 Artifact 値がいずれかに当たる（switch は enum 網羅 or next 必須。網羅済みなら next 禁止）。
   - P11: artifact 検証が失敗しうる node（command + artifact）が Contract field を参照する場合は next 必須。
   - ループ健全性: cycle を作る遷移には到達可能な `loop_guard` が必須。
   - `rules` の無い node は終端 node（WorkflowExecution 終了）。
3. **runtime**: `transition.rs` の regex 評価（RegexBuilder）・decide_next_node を、検証済み rules の一意遷移評価に置き換える。loop_guard は既存 step_execution_counts の反復カウントを流用して評価。field 不在は no-match（P11）。
4. **built-in 移行**: regex `match: NEEDS_FIX` 等で agent 本文を routing していた箇所を、「判定 node が enum/boolean field を持つ Artifact を `artifact:` で産出 → when/switch で分岐」に再設計する。verdict 用の schemas 宣言を各 built-in に追加し、instructions facet に構造化出力の提出を指示する文面を入れる。aggregate の then/else（fanout 内、#1330 担当）は現状のまま通す。
5. **DTO / protocol / frontend**: rules 型を新形式に更新（表示のみ。UI は D7 で YAML 編集化済み）。

## 削除対象

- `TransitionRule`（match/next）、regex routing（RegexBuilder 評価経路）、node 直下 `cycle_guard`、`resets_cycle_for`
- 式言語・比較演算の類（存在しないことを確認）

## テスト

- when / switch / next / loop_guard の受理と、排他・網羅・switch enum 抜け・catch-all 必須（P11）・到達可能 loop_guard の無い cycle の拒否。
- 任意の Artifact 値で遷移先がちょうど 1 つに定まる property test（proptest を dev-dependency に追加してよい）。
- rules 無し node が終端になること。loop_guard 超過で on_exhausted に遷移すること。
- 旧 `rules.match` / regex routing / node 直下 `cycle_guard` / `resets_cycle_for` を受理しない regression test。
- built-in が新 rules で load / 実行できること。
