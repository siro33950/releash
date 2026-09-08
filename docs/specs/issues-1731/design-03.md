# Design 03

## 開始状態

base は `main`、派生点は `52f5b862d`（#1730 取り込み後）。直前の Design は `docs/specs/issues-1731/design-02.md` で、その「変える部分」4件（空の `and` / `or` を拒否する規則の domain への一本化、host arena の容量受入判定と WFS010 生成の集約、`LuaData` から Source index を解決する操作の集約、未追跡ファイル `Deserialize` の削除）が worktree 上に未コミットの変更として実装済みである。この実装を今周の開始状態とする。`requirements.md` と `behavior.md` は今周変更しない。

この周までの Thread の扱いは次のとおりである。

- 解消済み（resolve 済み）: 2562433f、85f0eade、cef1203c、4559df5f。design-02 の「変える部分」として実装された。
- 見送り（`[DEFERRED]` で resolve 済み）: 574f7cc3（Rule handle 反復による build 時の述語展開量が host arena の上限計上に含まれない）、0baeb041（domain の `WhenRule` の serde derive が gateway の `Predicate` serde 実装に依存する）、02eff41b（詳細表示で `and` / `or` を含む辺が文字列化されない。`requirements.md` の Non-goal「workflow 定義の詳細表示（UI）での `and` / `or` を含む辺の表現」）。
- 修正（`[FIX_POLICY]` 付きで open）: 708c7e91。今周で変える部分はこの1件である。

## 変える部分

- 単一 Source を渡す `r.when` の host arena 計上を base に揃える: 現行の `call_when` は `predicate_value` を経由し、Source を `Predicate::Ref` に包む時点で `predicate_entries` を 1 件計上した後、Rule 追加前に容量を再検査する。base `52f5b862d` は call 入口の検査の後、Source の index を `RuleDraft::When` に格納して Rule 1件だけを積む。このため、総数 99,999 件の状態から既存 Source を `on` に渡す `r.when` は base で受理され、現行では WFS010 になる。単一 Source を `on` に渡す `r.when` が host arena に計上する件数を、base と同じ Rule 1件に揃える。根拠: R-004「単一の boolean field 参照を `on` の値として書く従来の `when` は、受理形、load 時の検査、実行時の遷移のいずれも変更前と同じである」、B-004「受理と拒否は本変更前と同じである」、Thread 708c7e91。ルート: 委任。計上をどこで分けるか（`call_when` で Source を直接 `Predicate::Ref` に包み `predicate_value` の Ref 計上を通さない、`predicate_value` に計上有無を渡す、など）の選択は Agent が行う。制約: `r.all` / `r.any` の要素としての Ref の計上（Vec 上の実体）と、述語 handle 再利用時の複製サイズの計上は現状維持する。`MAX_HOST_ARENA_ENTRIES` の値と WFS010 の意味を変えない。`mod_test.rs:380-383` の期待値（単一 Source の `when` で `predicate_entries == 1`、`arena_entries` の +1）は実装値であり、base の件数に合わせて見直す。揃える境界（B-004 の具体化）: 総数 99,999 件の状態から既存 Source を `on` に渡す `r.when` は base と同じく受理されて総数 100,000 件になり、総数 100,000 件からの同じ呼び出しは base と同じく WFS010 になる。`r.all` / `r.any` の要素 Ref と述語 handle 再利用の計上件数は変更前と同じである。

## 固定するルート

- design-01 / design-02 の固定ルートを全て維持する。辺の要素の骨格 `when: { on, then }` と sibling `next`、`on` の値は「文字列」「`and` を唯一のキーとする map」「`or` を唯一のキーとする map」の3形で要素も同じ3形、Lua は `r.all{ ... }` / `r.any{ ... }` を `r.when` / `r.switch` と同じ top-level に置く、述語型は `Ref` / `And` / `Or` の3形で参照の解決は呼び出し側のスコープが行う、要素が空の `and` / `or` は parse/shape 段で拒否しその規則は domain の `Predicate` が所有する、全体の指定「常に全体が最もシンプルに表現可能な形にする」。
- design-02「変える部分」の host arena 集約に付した制約「境界値を含め、変更前に受理された定義は受理され、拒否された定義は拒否される」は、「変更前」を design-02 開始状態ではなく base `52f5b862d`（本 Issue の変更前）として読み替えて維持する。理由: R-004 / B-004 の「変更前」は base を指し、design-02 の記述はこの1点で R-004 と食い違っていた。R-004 を正とする。
- 今周の変更の計上の分け方については、固定する実装上の指定なし。

## 変えないもの

- design-01 / design-02 の「変えないもの」を維持する。辺の要素の骨格（`when` / `switch` と sibling `next`）、`workflows/examples/full-cycle-development.yml` の辺（4箇所の `when` は単一参照のまま）。
- `requirements.md`（R-001〜R-012）と `behavior.md`（B-001〜B-012）。理由: 人間が追加の論点なしと判断した。今周の open Thread 2件（708c7e91 / 02eff41b）のいずれも Requirements / Behavior の変更を要しない。708c7e91 の修正は R-004 / B-004 が既に要求する受理境界へ実装を揃えるものである。
- Lua 評価環境の上限。`MAX_HOST_ARENA_ENTRIES` の値と WFS010 の意味を変えない。理由: `AGENTS.md`「Lua の評価環境の上限を緩めない」。Ref を一律 0 件にする案は、同一 Source を Lua 上限内で多数並べた述語の複製が arena 上限を逃れるため採らない。
- 両表面の Diagnostic の一致（R-008）。空の述語、Source 以外の値、arena 上限超過のいずれも、code / stage / message は変更前と同じ。
- 見送り Thread 3件の対象。Rule handle 反復による build 時の述語展開量の計上（574f7cc3）、domain の `WhenRule` の serde derive が gateway の serde 実装に依存する構造（0baeb041）、詳細表示での `and` / `or` を含む辺の表現（02eff41b）は、この周で変えない。理由: 前2件は design-02 で人間が blocking でないと判断し、02eff41b は `requirements.md` の Non-goal と design-01「定義の read model」で詳細表示を Non-goal と明記した決定を維持する。
- design-02 で委任し実装済みの範囲（host arena 容量判定の集約方法、Source index 解決の集約方法と変更前からの重複2箇所へ手を入れた範囲、`build_rule_predicate` が domain の構築を通すか）。理由: 実装済みで対応 Thread は解消済みであり、今周の変更対象ではない。

## 未確定・リスク

なし
