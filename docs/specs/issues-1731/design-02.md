# Design 02

## 開始状態

base は `main`、派生点は `52f5b862d`（#1730 取り込み後）。直前の Design は `docs/specs/issues-1731/design-01.md` で、その全項目が worktree 上に未コミットの変更として実装済みである。この実装を今周の開始状態とし、R-001〜R-012 / B-001〜B-012 を満たすことは人間が確認した。`requirements.md` と `behavior.md` は今周変更しない。

この周までの Thread の扱いは次のとおりである。

- 見送り（`[DEFERRED]` で resolve 済み）: 574f7cc3（Rule handle 反復による build 時の述語展開量が host arena の上限計上に含まれない）、0baeb041（domain の `WhenRule` の serde derive が gateway の `Predicate` serde 実装に依存する）。
- 修正（`[FIX_POLICY]` 付きで open）: 2562433f、85f0eade、cef1203c、4559df5f。今周で変える部分はこの4件である。

## 変える部分

- 空の `and` / `or` を拒否する規則の所有者を domain に一本化する: 現在は YAML の `parse_predicate`（`predicate_wire.rs`）と Lua の `call_predicate`（`lua/mod.rs`）がそれぞれ要素配列の空を直接判定し、domain の `Predicate` はこの規則を持たない。規則を domain の `Predicate` に置き、両 adaptor はそれを通す。根拠: Thread 2562433f、R-011「要素が空の `and` / `or` を含む定義は load 時に Error Diagnostic になり、load されない」、B-011、R-008「同じ定義上の誤りには同じ code、同じ stage、同じ message の Diagnostic が返る」、`docs/architecture/DOMAIN.md`「規則は domain が所有する」「一つの概念に一つの表現」。ルート: 下記「固定するルート」の domain 構築。Lua の build 時に `Predicate<usize>` を `Predicate<String>` へ写す箇所（`build_rule_predicate`）は空判定を持たず、要素は `call_predicate` で受理済みである。この箇所が domain の構築を通すかは委任。
- host arena の容量受入判定と WFS010 生成を1つの実装にする: `ensure_arena_budget`（1件追加前の判定）と `predicate_value`（述語 handle 再利用時に size 件追加前の判定）が、同じ上限値と同じ WFS010 文言で容量受入判定を別々に実装している。これを1つの実装に集約し、両者がそれを経由する。根拠: Thread 85f0eade、`docs/architecture/README.md`「同じ操作の実装は 1 つに集約する」。ルート: 委任。制約: `MAX_HOST_ARENA_ENTRIES` の値と WFS010 の意味（Lua 定義が builder 値の上限を超えた）を変えない。境界値を含め、変更前に受理された定義は受理され、拒否された定義は拒否される。
- `LuaData` から Source index を解決する操作を1つの実装にする: Source handle → Node → Input の順に試す解決の連鎖が、`call_child` の inputs 処理、`required_source`、`predicate_value` の3箇所に重複している。先の2箇所は変更前からの重複で、`predicate_value` が3箇所目である。これを1つの実装に集約し、3箇所がそれを使う。根拠: Thread cef1203c、R-008「`r.when` の `on` には Source または述語を渡せる」、`docs/architecture/README.md`「同じ操作の実装は 1 つに集約する」。ルート: 委任。変更前からの重複2箇所へ手を入れる範囲も委任。呼び出し元ごとの Diagnostic 表現（`type_error` / `host_field_error`）は各呼び出し元に残してよい。制約: 3箇所の受理結果と Diagnostic（code / message / field）は変更前と同じ。design-01「`r.when` の `on` は Source または述語を受ける」は維持する。
- 未追跡ファイル `Deserialize` を削除する: リポジトリ直下にある 0 バイトの未追跡ファイルを削除する。`git add` しない。根拠: Thread 4559df5f（要求・振る舞い・design-01 のいずれの変更対象にも対応しない）。ルート: 削除。

## 固定するルート

- design-01 の固定ルートを全て維持する。辺の要素の骨格 `when: { on, then }` と sibling `next`、`on` の値は「文字列」「`and` を唯一のキーとする map」「`or` を唯一のキーとする map」の3形で要素も同じ3形、Lua は `r.all{ ... }` / `r.any{ ... }` を `r.when` / `r.switch` と同じ top-level に置く、述語型は `Ref` / `And` / `Or` の3形で参照の解決は呼び出し側のスコープが行う、要素が空の `and` / `or` は parse/shape 段で拒否する、全体の指定「常に全体が最もシンプルに表現可能な形にする」。
- 空の `and` / `or` の拒否規則は domain の `Predicate` が所有する。`Predicate` に `Result` を返す `and` / `or` の構築（コンストラクタ）を置き、要素が空なら不適合を返す。YAML の `parse_predicate` と Lua の `call_predicate` はこの構築を通して `And` / `Or` を作り、返った不適合を code `WFS002` / stage `parse_shape` / message「predicate and/or must contain at least one element」に写す。adaptor 側の要素配列の空を直接判定する分岐は消す。`Predicate` の公開 variant `Ref` / `And` / `Or` は維持し、型の変更で空の `And` / `Or` を構築不能にすることは求めない。理由: 受理規則は domain が所有し、両表面はそれを通す形にする。variant の形と parse/shape 段での拒否は design-01 で固定済みであり、この周では所有者だけを整える。

## 変えないもの

- design-01 の「変えないもの」を維持する。辺の要素の骨格（`when` / `switch` と sibling `next`）、および `workflows/examples/full-cycle-development.yml` の辺（4箇所の `when` は単一参照のまま）。
- `requirements.md`（R-001〜R-012）と `behavior.md`（B-001〜B-012）。理由: 人間が完成した実装を確認し、追加の論点なしに確定した。
- Lua 評価環境の上限。`MAX_HOST_ARENA_ENTRIES` の値と WFS010 の意味を変えない。理由: `AGENTS.md`「Lua の評価環境の上限を緩めない」。
- 両表面の Diagnostic の一致（R-008）。空の述語、Source 以外の値、arena 上限超過のいずれも、code / stage / message は変更前と同じ。
- 見送り Thread 2件の対象。Rule handle 反復による build 時の述語展開量の計上（574f7cc3）と、domain の `WhenRule` の serde derive が gateway の serde 実装に依存する構造（0baeb041）は、この周で変えない。理由: 人間が blocking でないと判断し、後者は定義全体の wire 境界整理として本 Issue の範囲外とした。

## 未確定・リスク

なし
