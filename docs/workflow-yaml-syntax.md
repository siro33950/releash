# Workflow YAML 構文

この文書は Releash が受理する `WorkflowDefinition` YAML の正本である。語彙は [`architecture/GLOSSARY.md`](./architecture/GLOSSARY.md)、エンジン全体の方針は [`workflow-engine-evolution-plan.md`](./workflow-engine-evolution-plan.md) を正とする。完成形の例は [`examples/full-pipeline.yml`](./examples/full-pipeline.yml) を参照。

## 文法・検証・実行・起動の境界

この文書では、次の四つを分けて扱う。

| 境界 | 所有するもの | 所有しないもの |
| --- | --- | --- |
| YAML grammar | root、Contract、Node、rule の受理形 | 参照先の存在、型、到達性 |
| load-time validation | Diagnostic、名前解決、型検査、control-flow 検査 | WorkflowExecution の lifecycle state |
| runtime behavior | command / session / fanout の実行、Artifact 生成、遷移、stop / resume | workflow をいつ起動するか |
| execution trigger | UI / CLI / API から typed command で WorkflowExecution を起動 | WorkflowDefinition YAML の構文 |

タイマーや外部イベントとの連携は execution trigger 側の責務であり、WorkflowDefinition には書かない。root や node の未知 field は受理されない。

## Root

```yaml
name: example
description: workflow の説明
builtin: false

schemas: {}
nodes:
  - name: done
    command: "true"
```

- `name`: 必須。一意な workflow 名。先頭は ASCII 英数字、以降は ASCII 英数字・`-`・`_` のみ。
- `description`: 必須の文字列。
- `builtin`: 任意。既定値は `false`。
- `schemas`: 任意。名前付き Contract の map。既定値は空。
- `nodes`: 必須の非空配列。先頭の Node が entry。Node 名は workflow 内で一意で、`request` と `item` は使えない。
- 1 workflow は最大 256 Node、1 fanout は最大 64 child 参照。

## Node

Node は `name`、**ちょうど一つの kind block**、共通 field で構成する。

```yaml
- name: example_node
  command: "true"       # command / session / fanout のちょうど一つ
  inputs: [request]      # 任意。Artifact 全体への依存
  rules: []              # 任意。空なら終端
```

kind block が無い、または複数ある Node は parse / shape Diagnostic になる。kind を選ぶ別の discriminator field は無い。

### command

```yaml
- name: run_tests
  command: "cargo test"
```

`command` の値は `/bin/sh -c` に渡す非空の scalar 文字列である。worktree を cwd として一度実行し、常に次の予約 field を持つ Artifact を作る。

| field | 型 | 意味 |
| --- | --- | --- |
| `ok` | boolean | `exit_code == 0` かつ、`artifact` 指定時は stdout-JSON の Contract 検証にも成功 |
| `exit_code` | integer | process の exit code |
| `stdout` | string | 標準出力 |
| `stderr` | string | 標準エラー |
| `duration` | integer | 実行時間（ms） |

`artifact: <Contract>` がある場合、stdout 全体を JSON として parse して Contract で検証する。

- 成功時は予約 field と Contract field を**単一の Artifact 名前空間**に合成する。
- JSON parse または Contract 検証の失敗時は予約 field だけを保存し、`ok` を `false` にする。Node 自体は完了し、rules を評価する。
- Contract が予約 field と同名の property を宣言することはできない。
- process の起動自体に失敗した場合だけ Node は infrastructure failure になる。

### session

```yaml
- name: review
  session:
    model: claude-opus-5
    permission: ask       # ask | edit | full
    gate: approval        # auto | approval
    facets:
      policy: reviewing
      knowledge:
        - releash-thread
        - releash-review
      instruction: review-diff
  artifact: review_verdict
```

- `model`: 任意。指定時は登録済み model を参照する。
- `permission`: 必須。許可値は `ask` / `edit` / `full` の三つ。
- `gate`: 必須。`auto` または `approval`。
- `facets`: `policy` / `knowledge` / `instruction` の名前参照。session は少なくとも一つの facet 参照を持つ。
- `knowledge` は単一参照なら `knowledge: releash-review` の scalar、複数参照なら上の例のような配列で書ける。配列の宣言順は保持され、同じ参照を複数回書いた場合も重複排除しない。`policy` と `instruction` は単一の scalar 参照だけを受理する。
- user message の facet 部分は、全 Knowledge 本文を宣言順に並べ、その後に Instruction 本文を置いて、それぞれを `\n\n` で連結する。つまり上の例は `releash-thread` の本文、`releash-review` の本文、`review-diff` の本文の順になる。
- `artifact` がある session は、同じ Contract に対する検証済み Artifact の提出が完了するまで Node 完了にならない。提出と repair は共通の Contract 機構を使う。
- `gate: auto` は Artifact 条件を満たした後に自動完了する。
- `gate: approval` は Artifact 条件を満たした後も人間の承認まで `waiting_approval` に留まる。承認しない場合は同じ session に追加指示できる。別の却下・再実行操作は持たない。
- `artifact` の無い session は Artifact を産出しないため、他 Node から Artifact として参照できず、判別 rule も持てない。

### fanout

```yaml
- name: review_all
  fanout:
    child:
      - review_opus
      - review_gpt
    items: plan.targets
  rules:
    - next: summarize
```

- `child`: 必須の非空な Node 名参照。scalar 一つまたは配列で書ける。参照先は通常の top-level `command` / `session` Node。
- child は leaf 専用。workflow の entry、通常 rule の遷移先、fanout kind の Node にはできない。
- child に `rules` が宣言されていても Diagnostic にはしない。fanout が child として実行している間はその rules を無視し、child は Artifact を返すだけである。
- `items`: 任意。literal 配列、または `<node>.<field>` 形式で参照する配列 field。`<node>` 全体、`request`、`item` は `items` に書けない。
- `inputs` は fanout 親には書けない。fanout 親の `artifact` も宣言できず、子 Artifact の配列が暗黙の Artifact になる。
- fanout の判別 rule は配列 field を持たないため、`when` / `switch` は使えない。`next` と `loop_guard` を使う。

展開は次の matrix で決まる。

| `child` | `items` | 展開 | child の `input` |
| --- | --- | --- | --- |
| 複数 | 無し | child ごとに一回 | 宣言しない |
| 一つ | 有り | item ごとに同じ child | 必須。items 要素型と同じ Contract |
| 複数 | 有り | item × child の直積 | 全 child で必須。items 要素型と同じ Contract |

child の parameter は `input: <Contract>` 一つだけである。複数 field が必要なら一つの Object Contract にまとめる。fanout が `items` を供給し、実行中の値は `item` / `item.<field>` で参照する。

`items` が空配列なら child は一つも起動せず、fanout は空配列 Artifact で完了して通常どおり遷移する。子の一部失敗に固有の failure policy は無い。中断した WorkflowExecution を resume すると、完了済み child Artifact を再利用し、未確定 child だけを再実行する。

## 共通 field

| field | 意味 | 主な制約 |
| --- | --- | --- |
| `artifact: <Contract>` | Node が産出する Artifact の Object Contract | command / session 用。fanout は暗黙の配列 Artifact |
| `input: <Contract>` | fanout child として受ける単一 parameter の型 | `items` の要素 Contract と一致必須 |
| `inputs: [request | <node>, ...]` | Artifact 全体への依存宣言 | session は prompt に JSON を追加、command は依存宣言のみ。fanout 親には不可 |
| `rules` | Node 完了後の遷移 | 無い、または空なら終端 |

`inputs` には field path を書かない。command で field 値を使う場合は `inputs` で producer を宣言し、command 文字列内で `{{ <node>.<field> }}` を参照する。

## Artifact と参照

Artifact 参照は次の五つの形だけを持つ。

| 形 | 意味 | scope |
| --- | --- | --- |
| `request` | 起動時の String scalar Artifact | 全 Node |
| `<node>` | Node の Artifact 全体 | `inputs` と template |
| `<node>.<field>` | Node Artifact の field | template、fanout `items` |
| `item` | 現在の fanout item 全体 | fanout child の template |
| `item.<field>` | 現在の fanout item の field | fanout child の template |

`request` は Node が産出するものではなく、WorkflowExecution の start command が作る読み取り専用 Artifact である。`request` 用の明示的な `schemas` 宣言は不要で、同名 Contract の宣言もできない。start 時に request が省略された場合は空文字列になる。

command 文字列と facet 本文では二重波括弧で参照を補間する。

```yaml
command: "printf '%s' '{{ request }}'"
```

String はそのまま、それ以外の値は JSON serialize して置き換える。参照 path は一階層の field までで、Contract 名は path に含めない。fanout child の Artifact は親 fanout の配列にだけ入り、child 名では参照できない。

## Contract / `schemas`

`schemas` は次の keyword だけを持つ JSON Schema subset である。

- `type`
- `properties`
- `required`
- `items`
- `enum`

対応する型は `object` / `array` / `string` / `boolean` / `integer` / `number`。`string` Contract は scalar `string` としても宣言できる。subset 外の keyword は受理しない。

```yaml
schemas:
  work_item:
    type: object
    properties:
      path:
        type: string
      approved:
        type: boolean
      verdict:
        type: string
        enum: [SHIP, HOLD]
    required: [path, approved, verdict]

  work_items:
    type: array
    items: work_item
```

- `artifact:` に指定できるのは Object Contract だけ。fanout の配列 Artifact には Contract 名を宣言しない。
- `input:` には任意の Contract を指定できる。
- 配列の `items` は inline schema ではなく、同じ `schemas` 内の名前付き Contract を参照する。
- `required` の各 field は `properties` に存在しなければならない。
- `string` の `enum` は一つ以上の文字列を持つ。
- Object の `properties` にない field も受理する。`properties` は宣言済み field の型検証に使い、未宣言 field を拒否する設定は持たない。
- routing が参照する Contract field は、`properties` への宣言に加えて **`required` に含まれていなければならない**。`when.on` は required boolean、`switch.on` は required string enum に限る。
- command の予約 field `ok` は Contract 宣言なしで常に boolean routing field として使える。

## `rules`

`rules` は順序に依存しない正規形として load 時に検証される。一つの Node が持てるのは、最大一つの判別 rule、最大一つの `loop_guard`、および正規形で許される catch-all だけである。

### `when`

```yaml
rules:
  - when: { on: passed, then: done }
    next: fix
```

`on` は自 Node の Artifact にある required boolean field の bare 名。`true` なら `then`、`false` または実行時に field が無ければ sibling `next` に進む。`when` の `next` は必須で、**同じ配列要素内の sibling key**として書く。

### `switch`

```yaml
rules:
  - switch:
      on: verdict
      cases:
        SHIP: done
        HOLD: fix
    next: escalate
```

`on` は自 Node の Artifact にある required string enum field の bare 名。`cases` の key は enum 値だけを使う。

- 全 enum 値を cases が覆い、field の存在が実行前に保証される経路では `next` を書いてはならない。
- cases が非網羅なら、同じ配列要素の sibling `next` が必須。
- command が stdout-JSON の Contract field を参照する場合は、validation 失敗で field が存在しない可能性がある。そのため cases が enum を網羅していても sibling `next` が必須。
- field が実行時に無い、または case に一致しない場合は no-match として sibling `next` に進む。

### `next`

```yaml
rules:
  - next: done
```

単独の `next` 要素は、`when` / `switch` が同じ Node に無い場合だけ使える無条件遷移である。判別 rule の catch-all は単独要素に分離せず、必ずその `when` / `switch` と同一要素の sibling key にする。

### `loop_guard`

```yaml
rules:
  - loop_guard:
      max_iterations: 3
      on_exhausted: give_up
      reset_on: review_round
  - next: run_tests
```

`max_iterations` は 1 以上。遷移先 Node に guard がある場合、対象 Node の開始された実行回数（開始済み attempt 数）が上限に達していれば、その Node を再実行せず `on_exhausted` へ進む。cycle には、その cycle 上で到達可能な `loop_guard` が少なくとも一つ必要。

`reset_on` は任意で、同じ Workflow 内の Node 名を指定する。指定 Node が正常完了するたびに新しいカウント範囲を開始し、guard 対象 Node への遷移可否は、直近の正常完了より後に開始された同 Node の実行回数だけで判定する。指定 Node がまだ正常完了していない場合は Workflow 開始以降を範囲とする。失敗、中断、abort、実行開始だけでは範囲をリセットしない。

fanout Node を `reset_on` に指定した場合は、個々の child の完了ではなく、全 child の完了を含む fanout Node 自体の正常完了を境界とする。`reset_on` を省略した既存構文は、Workflow 実行全体の累計回数を使う従来の挙動を維持する。

### control-flow の制約

- 判別 rule は Node ごとに最大一つ。複数 `when`、複数 `switch`、`when` と `switch` の併用は不可。
- 任意の Artifact 値について遷移先がちょうど一つになるよう、排他性と網羅性を検証する。
- rule target と fanout child は存在する Node を参照する。
- 先頭 Node から通常遷移または fanout child 参照で到達できない Node は不可。
- fanout child は leaf 制約に従い、通常遷移の target にできない。
- `rules` が無い、または空の Node に到達すると WorkflowExecution は完了する。
- 比較、計算、配列集約の式言語は無い。command / session で boolean または enum field に畳んでから routing する。

## Diagnostic pipeline

Diagnostic は WorkflowDefinition の validation result であり、WorkflowExecution の lifecycle state ではない。概念上は次の五段階に分かれる。外部 `Diagnostic.stage` では parse と shape を同じ `parse_shape` stage として返す。

| 概念段階 | `Diagnostic.stage` | 責務 |
| --- | --- | --- |
| parse | `parse_shape` | YAML scanner / parser が構文を読めるか |
| shape | `parse_shape` | root と field、kind 個数、rule 要素形、unknown field |
| resolve | `resolve` | Node / Contract / Artifact path、`request` / `item` scope の名前解決 |
| typecheck | `typecheck` | Contract、routing field、fanout items / input、kind 別制約の型検査 |
| control-flow | `control_flow` | 排他、網羅、到達性、cycle / loop guard、fanout child leaf 制約 |

Rust backend が `code` / `stage` / `span` / `message` を返し、UI は表示だけを担当する。Error Diagnostic が一つでもある WorkflowDefinition は実行できない。

## Runtime と execution trigger

WorkflowDefinition の先頭 Node から実行を開始し、各 Node の確定 Artifact と rules だけで決定論的に遷移する。状態変更は backend の typed command を唯一の入口とし、UI / CLI / local API は同じ usecase を呼ぶ。

- start は workflow 名、Worktree、String request、permission mode を受け取る。
- `gate: approval` の承認、Artifact 提出、abort、stop、resume は WorkflowExecution / NodeExecution を対象にする。
- stop または crash / stale / orphan で `interrupted` になった WorkflowExecution だけを resume できる。確定済み NodeExecution の次から再開し、session は再アタッチせず新しく開始する。
- YAML は起動時刻、周期、外部イベント購読を定義しない。

## 既知の制約

- `{{ ... }}` の shell 補間は quoting や escaping を自動で行わない。String はそのまま、それ以外は JSON 文字列として埋め込まれるため、引用符、改行、shell metacharacter を含む値は command を壊したり意図しない shell 解釈を招きうる。workflow author が利用箇所に合う quoting を行い、信頼できない値を shell syntax に直接連結しないこと。stdin / 一時ファイル等の安全な Artifact ABI は現行文法の対象外。
- **信頼境界の注意**: command に補間される Artifact 値には、`request`（人間入力）だけでなく session（agent 出力）・前段 command 出力・fanout item が含まれる。agent 出力は Contract の JSON Schema 検証を通るが shell metacharacter はサニタイズされない。外部コンテンツ（PR / review comment 等）を処理した agent が細工した文字列を Artifact として出力し、それを下流 command node が補間すると、開発者マシン上でユーザー権限の shell が実行されうる。command node へ補間する参照が agent 由来 Artifact を含む場合は、この間接的な実行経路を前提に quoting するか、判断材料としてのみ session 内で扱い command に直接補間しないこと。
- command に YAML で指定する timeout は無い。abort / stop / アプリ終了は process group を停止するが、hang した command は agent session の stall observation 対象外であり、自動 stall 判定では止まらない。
- fanout の parallelism 上限、fanout 固有 retry / fail-fast、Node ごとの timeout は authoring syntax に持たない。

## 文法健全性

YAML deserialize 先の Rust 型が文法の形式定義である。

- `NodeKind = Command | Session | Fanout` と private raw shape の変換で、kind block がちょうど一つであることを保証する。
- `Rule = When | Switch | LoopGuard | Next` と許可 key 集合で、各 rule 要素の判別形を一意にする。
- unknown field を拒否し、互換 alias や正規化 layer を持たない。
- 文法で表現できることと、resolve / typecheck / control-flow で valid であることを分離する。

これにより grammar の一意性は型で、参照整合・型安全・遷移の決定性と loop 健全性は load-time Diagnostic で担保する。
