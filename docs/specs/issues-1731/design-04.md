# Design 04

## 開始状態

base は `main`、派生点は `52f5b862d`（#1730 取り込み後）。直前の Design は `docs/specs/issues-1731/design-03.md` で、その「変える部分」1件（単一 Source を渡す `r.when` の host arena 計上を base と同じ Rule 1件に揃える）が `lua/mod.rs` の `call_when` に worktree 上の未コミットの変更として実装済みである。この実装を今周の開始状態とする。`requirements.md` と `behavior.md` は今周変更しない。

この周までの Thread の扱いは次のとおりである。

- 解消済み（resolve 済み）: 2562433f、85f0eade、cef1203c、4559df5f（design-02 の「変える部分」）、708c7e91（design-03 の「変える部分」）。
- 見送り（`[DEFERRED]` で resolve 済み）: 574f7cc3（Rule handle 反復による build 時の述語展開量が host arena の上限計上に含まれない）、0baeb041（domain の `WhenRule` の serde derive が gateway の `Predicate` serde 実装に依存する）、02eff41b（詳細表示で `and` / `or` を含む辺が文字列化されない。`requirements.md` の Non-goal）。
- 本 Issue に起因しない（`[OUT_OF_SCOPE]` で resolve 済み）: 1369b099（先行 span 解析の資源境界）、34fd2ed4（定義 DTO が実行用 domain 定義から構築される経路）、1e2a7b06（旧インライン mod tests の配置）。いずれも base `52f5b862d` に同じ構造があり、今周の変更対象ではない。
- 修正（`[FIX_POLICY]` 付きで open）: 366a075a、49efa63d。今周で変える部分はこの2件である。

## 変える部分

- YAML の shape 検査が `when.on` を複製して `parse_predicate` に渡している深い clone をなくす: `diagnostics.rs` の `check_rules_shape` は `rule_obj.get("when").and_then(|when| when.get("on"))` で得た `&Value` を `on.clone()` で複製して `predicate_wire.rs` の `parse_predicate(value: Value)` に渡し、`Err` だけを扱う。shape エラーがない経路では、その後の `WorkflowDefinitionYaml` への deserialize が `Predicate<String>` の `Deserialize` 実装経由で同じ `parse_predicate` を再実行する。この複製をなくす。根拠: Thread 366a075a、R-008「同じ定義上の誤りには同じ code、同じ stage、同じ message の Diagnostic が返る」、R-011「要素が空の `and` / `or` を含む定義は load 時に Error Diagnostic になり、load されない」、B-011。ルート: 下記「固定するルート」の `parse_predicate` の受け方。制約: 空の `and` / `or`、`and` / `or` 以外のキー、配列でない値、参照文字列でも map でもない値に対する `WFS002` の code / stage（`parse_shape`）/ message / field（`rules.when.on`）/ span は変更前と同じ。design-02 の固定ルート「空の `and` / `or` の拒否規則は domain の `Predicate` が所有し、YAML の `parse_predicate` はその構築を通す」を維持する。有効な述語の受理結果と実行時の遷移は変わらない。`Ref` の文字列の所有の取り方、shape 検査と `Deserialize` 実装の共有方法など、これ以外の内部の細部は委任。
- 単一 Source の解決失敗を `WFS002` へ写す変換を1つの実装にする: `lua/mod.rs` の `call_when`（1117-1124）と `predicate_value`（1200-1207）が、`source_index` の失敗を `host_field_error("WFS002", PredicateShapeError::InvalidPredicate, location, field)` へ写す同じ式を別々に持つ。差は field 引数が `"on"` か呼び出し元から渡る値かだけである。この変換を1つの実装にし、両箇所がそれを使う。根拠: Thread 49efa63d、`docs/architecture/README.md`「同じ操作の実装は 1 つに集約する」、R-008「`r.when` の `on` には Source または述語を渡せる」、R-004 / B-004 / B-008。ルート: 下記「固定するルート」の計上の位置。実装の形（関数名・シグネチャ・置き場所）と、`predicate_value` の述語 handle の index 不在（1186-1193）を同じ `WFS002` `InvalidPredicate` へ写す箇所を同じ実装に含めるかは委任。制約: 単一 Source の `r.when`、`r.all` / `r.any` の要素、述語 handle 再利用のそれぞれで、受理結果と Diagnostic の code（`WFS002`）/ message / field（`"on"` / `"predicate element"`）は変更前と同じ。host arena の計上件数と WFS010 の境界（総数 99,999 件から既存 Source を `on` に渡す `r.when` は受理されて総数 100,000 件になり、総数 100,000 件からの同じ呼び出しは WFS010）は design-03 実装後の現状と同じ。

## 固定するルート

- design-01〜03 の固定ルートを全て維持する。辺の要素の骨格 `when: { on, then }` と sibling `next`、`on` の値は「文字列」「`and` を唯一のキーとする map」「`or` を唯一のキーとする map」の3形で要素も同じ3形、Lua は `r.all{ ... }` / `r.any{ ... }` を `r.when` / `r.switch` と同じ top-level に置く、述語型は `Ref` / `And` / `Or` の3形で参照の解決は呼び出し側のスコープが行う、要素が空の `and` / `or` は parse/shape 段で拒否しその規則は domain の `Predicate` が所有する（YAML の `parse_predicate` と Lua の `call_predicate` はこの構築を通す）、design-02 の host arena 集約の制約における「変更前」は base `52f5b862d` を指す（design-03）、全体の指定「常に全体が最もシンプルに表現可能な形にする」。今周の2件はいずれもこれらの解除・変更を伴わない。
- `parse_predicate` は `&Value` を受ける形にし、`check_rules_shape` の shape 検査と `predicate_wire.rs` の `Predicate<String>` の `Deserialize` 実装の両方がそれを通す。述語木の構築・破棄と、shape 検査後に YAML 全体を `WorkflowDefinitionYaml` へ再 deserialize する2パス構造（`Value` → typed）は変えない。理由: 述語の構築を1回にするにはこの2パス構造自体の変更が要り、本 Issue の範囲を超える。人間は Thread 366a075a の修正範囲を、固定ルート（空判定は domain の `Predicate::and` / `or` を通す）と両立する「深い clone をなくす」に限定した。
- `source_index` の失敗を `WFS002` `InvalidPredicate`（field 付き）へ写す変換は1つの実装にし、`call_when` と `predicate_value` の両方がそれを使う。計上（`predicate_entries` の増減と `ensure_arena_budget` の呼び出し）はその実装の外に残す。`call_when` の単一 Source は `Ref` を計上せず、`predicate_value` の要素 `Ref` は計上する現状を変えない。理由: 計上の分け方は design-03 で委任し実装済みで、対応 Thread 708c7e91 は解消済みである。集約するのは Diagnostic への変換だけとし、design-03 で確定した計上件数と WFS010 の境界を動かさない。

## 変えないもの

- design-01〜03 の「変えないもの」を維持する。辺の要素の骨格（`when` / `switch` と sibling `next`）、`workflows/examples/full-cycle-development.yml` の辺（4箇所の `when` は単一参照のまま）、`workflows/*.yml`（builtin 8本）。
- `requirements.md`（R-001〜R-012）と `behavior.md`（B-001〜B-012）。理由: 人間が design-03 実装後の現行実装を確認し、追加の論点なしと判断した。今周の open Thread 2件はいずれも Requirements / Behavior の変更を要しない。
- Lua 評価環境の上限。`MAX_HOST_ARENA_ENTRIES` の値と WFS010 の意味を変えない。理由: `AGENTS.md`「Lua の評価環境の上限を緩めない」と R-004。
- 両表面の Diagnostic の一致（R-008）。空の述語、Source 以外の値、arena 上限超過のいずれも、code / stage / message は変更前と同じ。今周の2件はいずれも Diagnostic の code / stage / message / field / span を変えない。
- 見送り Thread 3件の対象。Rule handle 反復による build 時の述語展開量の計上（574f7cc3）、domain の `WhenRule` の serde derive が gateway の serde 実装に依存する構造（0baeb041）、詳細表示での `and` / `or` を含む辺の表現（02eff41b）は、この周で変えない。理由: design-02 / design-03 で人間が blocking でない、または Non-goal と判断した決定を維持する。
- design-02 / design-03 で委任し実装済みの範囲（host arena 容量判定の集約方法、Source index 解決の集約方法と変更前からの重複2箇所へ手を入れた範囲、`build_rule_predicate` が domain の構築を通すか、単一 Source の `r.when` の計上の分け方）。理由: 実装済みで対応 Thread は解消済みであり、今周の変更対象ではない。

## 未確定・リスク

なし
