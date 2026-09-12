# Design 03

## 開始状態

直前の Design は `docs/specs/issues-1767/design-02.md`。その周の実装が作業ツリーに未コミットで入った状態を基準にする。

- 差分の基準: base ブランチ `main`、派生点 commit 8f6a107a（`release: v0.4.12 (#1756)`）。
- 未コミットの変更は `docs/glossary/WORKFLOW.md`、`src-tauri/src/adaptor/gateway/workflow/lua/mod.rs`、同 `mod_test.rs`、同 `stubs.rs`、同 `stubs_test.rs`、`src-tauri/src/domain/workflow/value_objects/definition.rs`、`src-tauri/src/infrastructure/lua/evaluator.rs`、および `docs/specs/issues-1767/` の文書。
- Design 02 の「変える部分」4件（無名 Session の自己 Artifact 供給、delegate の `inputs` の順序による定義不一致、生成 stub の型と実行時 API の一致、非Session handle からの delegate 宣言の検証）は実装済みである。
- この周までに解消となった Thread: 90a8b38f-3003-4eb5-ac6e-8e89e011e366（`inputs` の順序。R-009 の更新と R-016 / B-018 / B-019 の追加で決着し、更新後の要求は開始状態の実装が満たすため resolve 済み）。7b6d4c5b-ddd1-4c25-9c57-71d9434abe10、cb7f8c30-4cb0-4f25-af2b-3158395c99a3、004e51c5-4da0-4fa3-90e1-a69d4c5ed6aa は前周で解消済みである。
- `[FIX_POLICY]` を付けた open Thread が3件ある。

## 変える部分

- `SessionDelegate` の等価規則の domain での検証: 手書きの等価規則が実装に残る場合、`inputs` の順序だけが異なる2値が等価であることと、`child` / `when` / `max_iterations` / `inputs` の内容が異なる2値が等価でないことを、Lua / YAML の loader fixture を経由せず domain のテストで確認できるようにする。規則そのものが不要になる実装を採る場合はその規則を残さない。開始状態では `src-tauri/src/domain/workflow/value_objects/definition.rs:418-436` の `impl PartialEq for SessionDelegate` が `inputs` を `(name, raw)` で sort して比較する独自の規則を持つ一方、`definition_test.rs` で `SessionDelegate` を構築するのは580行の `has_child_executions` の分類テストだけであり、等価規則の検証は無い。根拠: R-009「delegate の `inputs` は `<パラメータ名>` から `<Source>` への対応であり、両表面で同じ対応を書いていれば、`inputs` を書いた順序は同一性の条件に含まない。」、Thread 55d5fb10-7423-4ce7-84b3-d1dcbaf115dd。ルート: 委任。
- 無名 Session の命名を domain の正準規則に従わせる: delegate の `inputs` で自己 Artifact を供給する無名 Session の名前が、自己供給を使わない無名 Node と同じ domain の規則で決まり、到達しない Node / Input の宣言の有無や順序を変えても到達可能なグラフが同じなら構築された Node の名前が変わらないようにする。R-003 / B-004 の自己供給は引き続き Error Diagnostic なしで load できる。開始状態では `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:1812-1834` が `push_node` の添字から `node-{index}` を作って `register_explicit` するため、domain の `NodeNamespace::register_synthesized`（`definition.rs:209-215`）が定める `<合成子名>#<index>` から外れ、`validation.rs:979-986` の `NodeNamespace::is_synthesized` による delegate の自己供給元の分類にも合致しない。根拠: R-003、B-004（いずれも Session の `name` の有無も命名規則も条件にしていない）、Thread 75877fd6-3221-4b8d-8d5d-00553d6fbf49。ルート: 委任。
- 無名 Session の名前解決の走査を有界にする: 該当 Session ごとに `host.nodes` 全件と `host.inputs` 全件から予約名集合を作り直さず、到達しない draft の件数を増やしても名前解決の走査量がそれに比例して増幅されないようにする。開始状態では `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs:1818-1828` が該当 Session ごとに全 draft から予約名集合を作り直し、`visit_node` の children ループ（同 `:1688-1703`）が Node ごとに `assign_node_name` を呼ぶ。node 数上限（`definition.rs:13`）は build 後の validation で検査されるためこの処理より後になる。根拠: R-003、B-004（自己供給を求めるがこの走査を要求していない）、Thread 79bbe3a3-219b-4a69-8c07-4a61ca5bf500。ルート: 委任。Thread 75877fd6-3221-4b8d-8d5d-00553d6fbf49 と同じ分岐が対象だが受入条件が異なるため統合せず、一つの変更で両方を満たしてよい。

## 固定するルート

今周に新たに固定する実装上の指定は無い。Design 01 で固定した次のルートは今周も維持する。

- delegate の宣言箇所は Lua の `completion` table ではなく Session handle のメソッド。
- 生成 stub の出力先は `.releash/releash.lua`。
- 文書の更新先は `docs/glossary/WORKFLOW.md` の「Lua」節と「Lua API」節。
- テストは `src-tauri/src/adaptor/gateway/workflow/lua/` に置く。

## 変えないもの

なし。

## 未確定・リスク

- 自動判断（Q-001、R-011）: 同じ Session handle への2回目以降の delegate 宣言は、YAML に対応する誤りが存在しないため、Lua 表面の shape の誤りとして既存の `WFS002` / `parse_shape` を再利用する。新しい `code` は追加せず、`message` の文言は固定していない。
- 自動判断（Q-002、R-015）: Artifact Contract 直下に `delegate` field を宣言した Session でも、Lua の `work.delegate` は handle のメソッドへ解決する。帰結として、その field を Lua の値参照の供給元にはできない。
- 自動判断（R-009、R-016）: delegate の `inputs` の同一性は `<パラメータ名>` から `<Source>` への対応の一致を指し、書いた順序を含まない。Lua の table は `pairs` の走査順が言語仕様で規定されず記述順を復元できないため、YAML の記述順へ一致させる読み方は成立しない。YAML 側の記述順の保持は R-016 として維持する。
- 未決のまま残した要求は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
