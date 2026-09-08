# Design 01

## 開始状態

初回。base は `main`、派生点は `52f5b862d`（#1730 取り込み後）。差分の基準は `docs/specs/issues-1731/requirements.md` の Current Behavior 節が記録する実装状態で、未コミットの変更はない。直前の Design はなく、open Thread もない。

## 変える部分

- 述語の値オブジェクトの新設: `Ref` / `And` / `Or` の3形で真偽値の合成だけを担う型を domain に置く。根拠: R-001〜R-003、`docs/specs/milestone-85/design.md` §6「真偽値の合成だけを担う値オブジェクトとしてコードで共通化する」。ルート: 型の形は design.md §6 の `Predicate<R> { Ref(R), And(Vec<Predicate<R>>), Or(Vec<Predicate<R>>) }` に従う。参照の解決は持たせない。
- 辺の `when.on` の述語化: `Rule::When` の `on` を文字列から述語に置き換え、YAML の `on` の値に文字列（単一参照）または `and` / `or` を唯一のキーとする map を受理する。根拠: R-001「`on` の値に、`and` を唯一のキーとする map を置き」、R-002、R-003、R-004「従来の `when` は、受理形、load 時の検査、実行時の遷移のいずれも変更前と同じ」。ルート: 下記「固定するルート」の YAML 形。
- 要素が空の述語の拒否: `and` / `or` の配列要素が空の定義を parse/shape 段で拒否する。根拠: R-011「要素が空の `and` / `or` を含む定義は load 時に Error Diagnostic になり、load されない」。ルート: 段は parse/shape。code / message は委任。
- 評価の共通化: 辺の遷移先決定に埋め込まれている boolean 評価を、述語の論理演算として述語側で行う。参照先が存在しないか boolean でない要素は false。根拠: R-001〜R-003、R-007「参照先の値が存在しない、または boolean でない参照は、false として評価される」。ルート: 委任。
- 型検査の共通化: 「各参照が boolean を指すか」の load 時検査を述語の全要素に適用する。根拠: R-006「述語に含まれる参照のいずれかが boolean を指さない場合、その定義は load 時に Error Diagnostic になり、load されない」。ルート: 委任。
- Lua 表面の述語: `r.all{ ... }` / `r.any{ ... }` を top-level の builder として追加し、`r.when` の `on` が Source または述語を受けるようにする。LuaLS 用 stub も同じ形にする。根拠: R-008「`r.all{ ... }` と `r.any{ ... }` がそれぞれ `and` / `or` の述語を返し」「`r.when` の `on` には Source または述語を渡せる」。ルート: 下記「固定するルート」の Lua 形。
- 定義の read model: usecase の Rule DTO の `when.on` が述語を運べるようにする。根拠: R-001〜R-003（`on` が文字列で表せなくなるため）。ルート: 委任。人間の指定「常に全体が最もシンプルに表現可能な形にする」に従う。詳細表示での and / or の表現は Non-goal。
- `docs/glossary/WORKFLOW.md`: 「rules と辺」節に `when.on` の述語の受理形・型検査・評価規則を、「Lua API」節に `r.all{}` / `r.any{}` と `r.when` の `on` の型を書く。根拠: R-010。ルート: 委任。
- `docs/glossary/DOMAIN.md`: 正規語の表に「述語（Predicate）」を1行足す。根拠: R-012。ルート: 定義は「辺と completion の判断に使う真偽値の論理式。原子は Artifact の required boolean field への参照で、`and` / `or` で合成する」の要旨、所有者は workflow。

## 固定するルート

- 辺の要素の骨格は現行の `when: { on, then }` と sibling `next` を維持し、述語は `on` の値に置く。`on` の値は「文字列（単一参照）」「`and` を唯一のキーとする map」「`or` を唯一のキーとする map」のいずれかで、`and` / `or` の値は配列、その要素にも同じ3形を置ける。`then` を `when` の外へ出す形や `and` / `or` を `when` の直下に置く形は採らない。理由: 辺の rules の構造の見直しは #1749 でまとめて行うと決めたため、今回は骨格を変えない。

    ```yaml
    rules:
      - when:
          on:
            and:
              - passed
              - or:
                  - clean
                  - skipped
          then: done
        next: fix
    ```

- Lua は `r.all{ ... }` / `r.any{ ... }` を `r.when` / `r.switch` と同じ top-level に置く。名前空間（`r.predicate.*`）や bracket 記法（`r["and"]`）は採らない。理由: `and` / `or` は Lua の予約語であり、`all` / `any` はその慣用名。述語は辺と delegate の2箇所でしか使わず、名前空間で括る語彙量にならない。

    ```lua
    r.when{
      on = r.all{ judge.passed, r.any{ judge.clean, judge.skipped } },
      on_true = done,
      next = fix,
    }
    ```

- 述語型の形は design.md §6 のとおり `Ref` / `And` / `Or` の3形とし、参照の解決は呼び出し側のスコープが行う。
- 要素が空の `and` / `or` は parse/shape 段で拒否する。
- 全体の指定: 「常に全体が最もシンプルに表現可能な形にする」。

## 変えないもの

- 辺の要素の骨格（`when: { on, then }` と sibling `next`、`switch: { on, cases }` と sibling `next`）。理由: #1749 で再検討するため、本 Issue では動かさない。
- `workflows/examples/full-cycle-development.yml` の辺。理由: 4箇所の `when` はいずれも単一参照で判断が足りており、and / or の例は `WORKFLOW.md` の「rules と辺」節が担うと決めた。

## 未確定・リスク

- 要素が空の述語の拒否は、YAML では shape 検査、Lua では builder の引数検査と経路が分かれる。R-008 は同じ定義上の誤りに同じ code / stage / message を求めるため、両経路の Diagnostic を揃える必要がある。
