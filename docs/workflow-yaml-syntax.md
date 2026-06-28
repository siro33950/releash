# Workflow YAML 構文（設計案）

この文書は、ワークフローエンジン発展計画（[`workflow-engine-evolution-plan.md`](./workflow-engine-evolution-plan.md)）で目指す完成形の Workflow YAML 構文を定義する。語彙は [`architecture/GLOSSARY.md`](./architecture/GLOSSARY.md) に従う。

完成形の例は [`examples/full-pipeline.yml`](./examples/full-pipeline.yml) を参照。

> ステータス: 設計案。未実装の構文を含む。末尾の「未確定」を参照。

## Node

- Node は `name` ＋ **kind ブロックをちょうど1つ**（`command` / `session` / `fanout`）＋ 共通フィールド（`artifact` / `inputs` / `rules`）で構成する。
- kind ブロックが0個または2個以上は load 時 Diagnostic。

### command

```yaml
- name: run_tests
  command: "cargo test"
```

- 値はシェルコマンド（スカラー）。
- 標準結果として常に `ok`（exit==0 の boolean）/ `exit_code` / `stdout` / `stderr` / `duration` を持つ。routing は `ok` を使える。
- `artifact: <Contract>` を付けると、**stdout を JSON として parse し Contract で検証**して追加フィールドを Artifact にする。

### session

```yaml
- name: review_opus
  session:
    model: claude-opus-4-8
    permission: read
    gate: auto              # auto | approval（session は必須）
    facets:
      policy: reviewing
      knowledge: releash-review
      instruction: review-diff
  artifact: review_verdict
```

- `session` 直下 = 実行設定（`model` / `permission` / `gate`）。
- `gate`: `auto`（自動で完了判定）か `approval`。session では必須。
- `gate: approval` は **session が承認されるまで完了しない**。session は対話式なので、承認しなければ人間がそのまま指示を続けて直させる。承認で完了→次へ。**却下や再実行という別操作は無い**。
- `facets`: 再利用部品の参照（`policy` / `knowledge` / `instruction`）。本文は別途定義し名前で参照する。
- session の `artifact` は agent が **CLI（`releash workflow output submit`）で提出**する（command の stdout-JSON と対になる経路）。

### fanout

```yaml
- name: review
  fanout:
    child:                  # 1つ、または複数の Node 参照
      - review_opus
      - review_gpt
    # items: <配列>         # 任意。child を配列ぶん展開
```

- `child`: 展開する Node の名前。普通の Node を参照する。ただし **child の `rules` は無視される**（child は leaf＝artifact を返すだけで遷移しない）。
- `items`: 任意。リテラル配列、または前 Node の配列フィールド参照。各要素が子の `input` に入る。
- 組合せ:
  - `child` 複数 / `items` なし … 別 Node を並列実行
  - `child` 1つ / `items` あり … その Node を配列ぶん展開（件数は実行時に決まる＝動的）
  - `child` 複数 / `items` あり … マトリクス（item × child）
- fanout の `artifact` は **子 artifact の配列**。
- 結果でまとめて分岐したい場合は、配列を畳んで boolean を出す Node（command 等）を挟む。fanout / rules に集約機構（aggregate / all / any）は持たない。
- **定義と供給の分担**: 型を定義するのは child（`input: <Contract>` ＝ パラメータ）。fanout は `items` で**供給**するだけ（map 呼び出しに相当）。child の `input` が1つなら束ね先は自明。
- 検証: items の要素型が child の `input` 型と一致するか load 時に検査。
- **空 items（0件）**: 子0個で fanout は完了。artifact は空配列。特別扱いせず通常 rules で遷移する。
- **child は単一 input のみ**。複数の入力が要るデータは child（command / session）が CLI から取得する。fanout は複数 input の束ねを扱わない。
- **子の一部失敗**: fanout 固有の failure policy は持たない。中断は Resume（後述マイルストーン）で再開する。
- **並列度の上限**: 現状スコープ外。

## 共通フィールド

- `artifact: <Contract 名>` … この Node が産出する Artifact（成果物。Contract で型付け）。
- `inputs: [<node>, ...]` … 他 Node の Artifact を入力として受ける（呼び出し元の指定不要）。
- `input: <Contract 名>` … この Node の**パラメータ**（型）。fanout の child のとき、各要素がここに入る（関数の引数に相当）。
- `rules:` … 遷移（下記）。

## Artifact（参照規約）

各 Node は `artifact:` で **1つの Artifact** を産出し、**Node 名で参照する**。参照は次の1形に統一する。

- `<node>` … その Node の Artifact 全体（fanout は子 artifact の配列）。
- `<node>.<field>` … その Artifact のフィールド。
- `item` / `item.<field>` … fanout で展開された現在の要素。
- `request` … 起動時の `"<task>"`（**初回 Artifact**・**String**・予約名）。人間が書く自由文字列なので scalar（String）の Artifact を許す（Contract は object に限らない）。node が産むものではないが、他 Artifact と同じく `inputs: [request]` で受け、`{{ request }}` で参照する。
- rules の `on:` / `switch.on:` … **自分の artifact** のフィールドを bare 名で参照（`<this node>.<field>` の略）。
- テンプレート補間は **二重波括弧 `{{ ... }}`**。パスは参照と同じ（`{{ request }}` / `{{ <node>.<field> }}` / `{{ item.<field> }}`）。二重にするのは、command 本文の literal な単一 `{}`（jq / shell のブレース）と衝突させないため。

参照パスに Contract 名は出さない（Node の artifact は1つなので Node 名がそのまま Artifact）。

## rules（遷移）

`rules` は「この Node の抜け方」の集合。**順序非依存**で、全体を load 時に検証する。

`rules` の無い node（遷移先を持たない node）に到達したら、WorkflowExecution はそこで**終了**する（終端 node）。

要素の形:

- `when: { on: <boolフィールド>, then: <node> }` ＋ 同じ要素の `next: <node>`（偽のときの行き先）
- `switch: { on: <enumフィールド>, cases: { 値: <node>, ... } }`
- `loop_guard: { max_iterations: <n>, on_exhausted: <node> }`（cycle 上限。超過で `on_exhausted` へ）
- `next: <node>`（無条件 ＝ どれにも当たらない残り＝ catch-all）

```yaml
  rules:
    - when: { on: passed, then: done }
      next: fix
```

```yaml
  rules:
    - switch:
        on: verdict
        cases:
          SHIP: done
          HOLD: list_threads
          ESCALATE: escalate
```

```yaml
  rules:
    - loop_guard: { max_iterations: 3, on_exhausted: give_up }
    - next: run_tests
```

検証ルール（load 時 Diagnostic）:

- **排他**: どの artifact 値も2つ以上の rule に当たらない。
- **網羅**: 取り得る全 artifact 値がいずれかに当たる（残りは `next` が覆う。switch は enum を網羅 or `next` 必須）。
- **ループ健全性**: cycle を作る遷移には、到達可能な `loop_guard` が必須。
- 式言語は持たない。比較・計算（`count > 0` 等）や配列の集約は Node 側で boolean / enum に導出してから routing する。

## Contract / schemas

```yaml
schemas:
  review_verdict:
    type: object
    properties:
      lgtm:
        type: boolean
```

- `schemas:` で名前付き Contract を宣言する。
- `artifact: <名前>` でその Contract の Artifact を産出。routing が見る `on` フィールドは Contract に宣言された boolean / enum であること。
- 配列の要素型を他所（fanout child の `input` など）から参照する場合、要素型は inline でなく**名前付き Contract** にして `items: <名前>` で参照する。同じ型は producer の artifact と consumer の input が同じ名前を参照する（定義は1か所）。

## Task（global 作業リスト）

Task は WorkflowExecution が持つ **global な作業リスト**。固定 schema で、workflow では定義しない。

```text
tasks[]              # 予約 global 名
  - id
  - description
  - done             # boolean（完了したか）
```

- **書き込みは CLI のみ**（実行中に agent / 人間が CLI 経由で積む・更新する）。workflow YAML から書かない。
- **workflow からは read のみ**。`tasks` を fanout の元にできる。
- `tasks` は予約語（Node を `tasks` と名付けられない）。
- `releash workflow start <wf> "<task>"` の `"<task>"` は `tasks` には入らない。初回 Artifact `request`（String）になる（Artifact 節参照）。

```yaml
- name: work_each
  fanout:
    child: do_task
    items: tasks       # global の作業リストを展開
```

## Trigger（起動設定・別レイヤー）

Trigger は「いつ起動するか」の設定であり、WorkflowDefinition には含めない。workflow を名前で参照する別物として持つ。

```yaml
triggers:
  - workflow: full-pipeline
    on: schedule
    cron: "0 9 * * 1-5"
  - workflow: full-pipeline
    on: external
    event: sentry.alert
```

## 例

完成形の全体例は [`examples/full-pipeline.yml`](./examples/full-pipeline.yml) を参照。

## 文法健全性の担保

この構文定義（文法）そのものが一意・無矛盾であることを、次で機械的に担保する。個々の workflow が valid かの検証（load 時 Diagnostic）とは別レイヤー。

- **形式化**: 構文は散文でなく代数的型（sum 型）として定義する。`schema.rs` の Rust 型がその形式定義であり、コンパイルが通ること＝構造的無矛盾の証明になる。
- **不正構文を表現不能にする**: kind と rule を optional field でなく enum にする。
  - `NodeKind = Command | Session | Fanout` → 「kind ブロックはちょうど1つ」が型で保証（0個 / 2個は表現できない）。
  - `Rule = When | Switch | LoopGuard | Next` → 「各 rule はいずれか1つ」が型で保証。
- **一意性（曖昧でない）**: 各構文が唯一の判別子を持つ。kind = ブロック名（`command` / `session` / `fanout`）、rule = キー名（`when` / `switch` / `loop_guard` / `next`）。判別子キー集合は互いに素にし、tagged enum + `deny_unknown_fields` で parse を一意にする。
- **充足可能性（非空）**: 文法の制約同士が両立し、valid な program が少なくとも1つ存在することを確認する（例: 「switch は enum 網羅 or `next` 必須」と「網羅済みなら `next` 禁止」が矛盾しない）。

この4点で「文法が一意・完全・無矛盾」を保証する。workflow instance の決定性 / 停止性 / 型安全 / 参照整合は、別途 load 時 Diagnostic（rules 節・各種検証）で担保する。

## 決定済み（旧・未確定）

- fanout child の input は**単一のみ**。複数の入力が要るデータは child が CLI から取得する。
- 空 items（0件）= 子0個で fanout 完了・artifact 空配列・通常 rules で遷移。
- 子の一部失敗 = 固有 failure policy なし。Resume で再開する。
- 並列度制御 = 現状スコープ外。
- `request` は String（scalar Artifact を許す）。
