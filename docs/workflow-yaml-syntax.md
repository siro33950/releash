# Workflow 定義構文

この文書は Releash が受理する workflow 定義の正本である。YAML と Lua は load 後に同じ `WorkflowDefinition` になり、実行・事実ログ・read model・resume に定義形式の違いは残らない。語彙は [`architecture/GLOSSARY.md`](./architecture/GLOSSARY.md)、実行モデルは [`workflow-engine-evolution-plan.md`](./workflow-engine-evolution-plan.md) を正とする。完成形の唯一の例は [`../specs/unified-node-model/examples/full-cycle-development.yml`](../specs/unified-node-model/examples/full-cycle-development.yml) である。

## 境界

| 境界 | 所有するもの |
| --- | --- |
| definition grammar | root、Contract、Node、children、rule の受理形 |
| load-time validation | Diagnostic、名前解決、型検査、control-flow 検査 |
| runtime | Node の実行、Artifact、辺の進行、stop / resume / abort |
| execution trigger | UI / CLI / API からの WorkflowExecution 起動 |

定義は起動時刻、周期、外部イベント購読を持たない。未知 field、旧形式、互換 alias は受理せず、Error Diagnostic が一つでもある定義は実行できない。

## YAML

### トップレベルと `main`

```yaml
name: full-cycle-development
description: 入力収集から実装とレビューまでを実行する
schemas: {}
nodes:
  main:
    sequence:
      children:
        - collect_request
        - implement
  collect_request:
    session:
      provider: claude
```

- `name`: 必須。一意な非空 workflow 名。先頭は ASCII 英数字、2文字目以降は ASCII 英数字・`-`・`_` だけを使う。
- `description`: 必須の文字列。
- `builtin`: 任意。保存された user 定義の builtin 判定はコード側が所有する。
- `schemas`: 任意。名前付き Contract の map。
- `nodes`: 必須かつ非空。Node 名をキーにした map。配列ではない。
- root は `nodes.main` という規約で決まる。`main` が無ければ Diagnostic になる。
- トップレベルの `entry` field は存在しない。`entry` は Sequence 固有 field である。
- 一つの定義は最大 256 Node、一つの Fanout は最大 64 children エントリを持つ。

`nodes` は単一名前空間であり、Node は `command` / `session` / `fanout` / `sequence` の kind block をちょうど一つ持つ。Node は遷移を持たず、配線と辺は、その Node を子として扱う合成子の `children` に置く。

### Node の4種

| 種別 | 役割 | 形 |
| --- | --- | --- |
| Session | provider CLI と継続対話する葉 Node | `session:` |
| Command | shell command を一度実行する葉 Node | `command:` |
| Fanout | children を並列に束ねる合成子 | `fanout:` |
| Sequence | children を時系列に束ねる合成子 | `sequence:` |

合成子の child には4種すべてを置ける。Sequence の子の Sequence、Fanout の子の Sequence や Fanout も通常の再帰構造として扱う。

### Node の Interface と children の配線

Node 共通 field は kind block と同じ階層に書く。

| field | 意味 |
| --- | --- |
| `input` | Node が受け取るパラメータのリスト。文字列は型なし、`- name: Contract` は型あり |
| `artifact` | Node が産出する Artifact の Contract 名。Fanout には宣言しない |
| `completion` | Node 自身の完了定義。`auto` または `approval`。省略時は `auto` |
| `worktree` | 将来の隔離実行用の予約 field。現行 loader では `WFU002` Error |

`input` と `artifact` は Node の Interface であり、`inputs` は children エントリに置く配線である。本文はパラメータ名を参照し、供給元 Node 名は配線にだけ現れる。

```yaml
nodes:
  judge:
    command: "echo '{{ reviews }}' | jq '{all_lgtm: all(.[].lgtm)}'"
    input:
      - reviews: review_verdicts
    artifact: judge_result

  main:
    sequence:
      children:
        - review_all
        - judge:
            inputs:
              reviews: review_all
```

children の `inputs` は `<パラメータ名>: <供給元>` の map である。

- Sequence の子: 兄弟エントリ、`<兄弟>.<field>`、Sequence 自身の input、`request`。
- Fanout の子: Fanout 自身の input、field path、`request`、展開中の要素を表す `items`。
- `request` は起動時の String input で、どの合成子の配線からも参照できる。
- Fanout の子は並走するため、兄弟の Artifact を直接参照しない。外側の値は親から input を一段ずつ渡す。
- `items` は本文の特殊名ではない。child の input パラメータへ配線し、そのパラメータ名を本文で参照する。
- 配線先は child が宣言した input パラメータでなければならない。供給元は `<name>` または `<name>.<field>` の1段だけで、参照先の Node / Contract field が存在し、Node 供給元は Artifact を産出する必要がある。
- 同じ名前が Sequence の兄弟 Node と Sequence 自身の input パラメータの両方に一致する配線は曖昧なので拒否される。`request` と `items` は予約供給元名であり、Node の input パラメータ名には使えない。`request` / `items` に field は無く、`items` は `items` を宣言した Fanout 内だけで使える。

### children の4形式

Sequence と Fanout の `children` は同じリスト形式を使う。

```yaml
children:
  - review_opus
  - fix_tests:
      inputs:
        test_result: run_tests
      rules:
        - next: run_tests
  - quick_check:
      command: "cargo check"
  - session:
      provider: codex
      model: o3
      permission: read-only
    artifact: review_verdict
```

1. 文字列: カタログ Node の参照。
2. kind block を持たない単一名 map: カタログ参照と、その children エントリでの `inputs` / `rules` / `on_failure`。
3. kind block を持つ単一名 map: 名前付きインライン Node。load 時に同じカタログへ正規化される。
4. kind または Node 共通 field から始まる map: 無名インライン Node。`<合成子名>#<index>` の内部名へ正規化される。

合成子の `children` は非空でなければならず、カタログ参照は存在する Node を指す必要がある。同じ Node を同じ合成子から複数回参照すると `WFC007`、複数の合成子の child として共有すると `WFC006` になる。root の `main` は別の合成子の child にできない。

名前は配線や辺から参照するときだけ必要である。インライン宣言もカタログと同じ名前空間を使い、名前衝突は Diagnostic になる。

### Sequence

```yaml
main:
  sequence:
    entry: run_tests
    output: publish
    children:
      - run_tests
      - fix_tests:
          inputs:
            test_result: run_tests
          on_failure:
            retry: 2
          rules:
            - loop_guard:
                max_iterations: 3
                on_exhausted: give_up
            - next: run_tests
      - publish
  artifact: release_result
```

- `entry`: 開始する children エントリ名。省略時は先頭。
- `output`: Sequence が `artifact` を宣言するときに、その Artifact を返す children エントリ名。`artifact` があれば必須で、無ければ書かない。
- `children`: 実行対象と、各 child の配線・辺・失敗時の扱い。
- `rules` を省略したエントリには、リストの次のエントリへ進む隣接辺がある。末尾では終端になる。
- `rules: []` は明示的な終端である。

`on_failure` は children エントリが所有する。省略時は中断し、resume または手動 Retry を待つ。`ignore` は失敗を除外して続行し、`retry: n` は新しい attempt を最大 n 回自動実行した後、省略時と同じ中断へ移る。

- `on_failure: retry` は attempt 機構の対象である Session / Command child だけに宣言できる。Sequence / Fanout child への宣言は `WFC010` になる。
- `on_failure: ignore` の child は、失敗時に Artifact を産出しない可能性がある。そのため、同じ Sequence の兄弟 `inputs`、その child 自身の `when` / `switch`、Sequence の `output`、または兄弟 Fanout の `items` がその Artifact に依存する定義は `WFC009` になる。

### Fanout

```yaml
fix_each:
  fanout:
    items: list_threads.threads
    children:
      - fix_one:
          inputs:
            thread: items
            plan: plan
  input:
    - plan
```

- `children`: Sequence と同じ4形式。
- Fanout の children エントリに `rules` は書けない。Fanout children は並列展開であり辺を持たないため、宣言すると `WFC007` になる。
- `items`: literal 配列、または Artifact の `<node>.<field>` 配列。
- item ごとに children を展開する。各要素は children の `inputs` で供給元 `items` から渡す。
- child の input が一つだけで `items` がある場合に限り、その `inputs` を省略できる。
- 型付き child input が `items` を受ける場合、Artifact 配列の要素 Contract または各 literal item がその Contract と一致する必要がある。
- children の Artifact は実行順の配列として Fanout の Artifact になる。
- `on_failure: ignore` の child は失敗時に結果配列から除かれる。

### Command

```yaml
run_tests:
  command: "cargo test"
  artifact: test_result
```

`command` は worktree を cwd として shell で一度実行する非空文字列である。結果は `ok`、`exit_code`、`stdout`、`stderr`、`duration` を持つ。`artifact` があれば stdout 全体を JSON として parse・Contract 検証し、予約 field と同じ Object Artifact に合成する。process 起動不能は Node failure、非ゼロ exit codeまたは stdout 検証失敗は `ok: false` の確定結果になる。

テンプレートの `{{ parameter }}` / `{{ parameter.field }}` は Node が宣言した input パラメータを参照する。field を付ける場合はそのパラメータの Contract に存在する1段の field でなければならず、未宣言パラメータ、未知 field、2段以上の path は拒否される。shell quoting は自動で行われないため、信頼できない値を shell syntax へ直接連結しない。

### Session

```yaml
review:
  session:
    provider: claude
    model: claude-opus-4-1
    permission: read-only
    facets:
      policy: reviewing
      knowledge:
        - releash-review
      instruction: review-diff
  artifact: review_verdict
  completion: approval
```

- `provider`: 必須。`claude` または `codex`。
- `model` / `permission`: 任意。値を変換せず provider CLI の起動設定として渡す。
- `facets`: `policy` / `knowledge` / `instruction` の参照。Session は少なくとも一つの facet 参照を必要とする。
- `artifact`: Submit に添付できる Artifact の Contract。宣言時は検証済み Artifact を含む Submit だけが有効。

Session の `completion: auto` は、同一 Node attempt の Submit と provider Stop の二信号が揃ったときに完了する。順序は問わず、一方だけでは完了しない。`completion: approval` は二信号が揃った後に WaitingApproval となり、人間の Approve で完了する。

### completion

`completion` は全4種の Node で宣言できる。`approval` は本来の完了条件を満たした後、人間が承認するまで完了を保留する。

| Node | `auto` | `approval` |
| --- | --- | --- |
| Session | Submit と provider Stop の二信号 | 二信号の後に Approve |
| Command | process 終了 | 終了後に Approve |
| Fanout | 全 child が決着 | 全 child 決着後に Approve |
| Sequence | 終端へ到達 | 終端到達後に Approve |

### rules と辺

辺は Sequence の children エントリが `rules` として所有する。

```yaml
rules:
  - when:
      on: passed
      then: done
    next: fix
```

```yaml
rules:
  - switch:
      on: verdict
      cases:
        SHIP: done
        HOLD: fix
    next: escalate
```

```yaml
rules:
  - loop_guard:
      max_iterations: 3
      on_exhausted: give_up
  - next: run_tests
```

- `when.on` は自 Node Artifact の required boolean field。
- `switch.on` は required string enum field。非網羅なら同じ要素の sibling `next` が必須。
- 単独の `next` は無条件辺。
- 一つの `rules` リストに置ける判別規則（`when` または `switch`）、`loop_guard`、単独 `next` はそれぞれ最大一つである。判別規則と単独 `next` は併記せず、判別規則自身の sibling `next` を catch-all に使う。
- 辺の target は存在する Node でなければならない。同じ Sequence の child またはどの合成子にも属さない Node へ遷移できるが、別の合成子が所有する child へ外から遷移できない。
- `switch` の case は enum 値だけを使う。case が非網羅なら sibling `next` が必須で、網羅していれば `next` は書かない。ただし Artifact を持つ Command の独自 field で分岐するときは、command failure の catch-all として `next` が必要である。
- Fanout child に `when` / `switch` は置けない。Command の予約結果 `ok` を除き、Artifact を宣言しない child の field では分岐できない。
- 後方辺の cycle には、その cycle 上の少なくとも一つのエントリに `loop_guard` が必要である。無い場合は `WFC005` になる。`max_iterations` は1以上で、上限では `on_exhausted` へ進む。合成子の静的な包含 cycle も load 時に拒否する。
- 全 Node は `main` から children または rule target を辿って到達可能でなければならない。Sequence 内でも、実効 `entry` から隣接辺または明示 rules で到達できない child は拒否される。
- 比較・計算・配列集約の式言語はない。Command または Session が routing 用 boolean / enum を Artifact にする。

### Contract / schemas

`schemas` は `type`、`properties`、`required`、`items`、`enum` だけを持つ JSON Schema subset である。型は `object` / `array` / `string` / `boolean` / `integer` / `number`。Contract 名と `artifact` / `input` の Contract 参照名は、先頭が ASCII 英数字、2文字目以降が ASCII 英数字・`-`・`_` の安全な identifier でなければならない。

```yaml
schemas:
  review_verdict:
    type: object
    properties:
      approved:
        type: boolean
      verdict:
        type: string
        enum:
          - SHIP
          - HOLD
    required:
      - approved
      - verdict
```

`required` の各 field は同じ Object の `properties` に存在しなければならない。配列の `items` は同じ `schemas` 内に存在する名前付き Contract を参照し、string の `enum` は宣言するなら非空でなければならない。Node の `artifact` / 型付き `input` が参照する Contract も同じ `schemas` 内に存在する必要がある。

`artifact` は Fanout 以外の Node で Object Contract を参照する。Fanout は child Artifact の配列を結果として持つため `artifact` を宣言しない。routing field は `properties` と `required` の両方に必要である。Command の `ok` は宣言なしで boolean routing field として使える。Command の Artifact Contract には標準結果 field の `ok` / `exit_code` / `stdout` / `stderr` / `duration` を再宣言しない。

### 予約語と未解禁 field

次は Node 名に使えない。

```text
command session fanout sequence input artifact completion worktree
inputs rules on_failure items entry output children
```

`request` と `items` は input 配線の予約供給元名であり、input パラメータ名には使えない。`request` は `schemas` の Contract 名としても使えない。

`worktree` は Node 共通 field として予約されているが、`shared` / `isolated` の実行は未解禁である。現行 loader は宣言を `WFU002` Error として拒否する。成功する定義には `worktree` を書かない。

## Lua

`.lua` は load 時に一度だけ評価され、YAML と同じ `WorkflowDefinition` を構築する。実行中に Lua は評価されない。chunk は `r.workflow{...}` を返す必要がある。

```lua
local r = require("releash")
local f = require("facets")

local implement = r.session{
  provider = r.provider.claude,
  facets = { instruction = f.instruction.implement_fix_plan },
  artifact = r.schema.object{
    name = "review_verdict",
    properties = { approved = r.schema.boolean() },
    required = { "approved" },
  },
  completion = r.completion.approval,
}

local main = r.sequence{
  children = {
    r.child{ node = implement },
  },
}

return r.workflow{
  name = "fix",
  description = "Implement a fix plan",
  main = main,
}
```

### Lua API

| API | 戻り値 |
| --- | --- |
| `r.command{ name?, command, artifact?, input?, completion? }` | Node |
| `r.session{ name?, provider, model?, permission?, facets?, artifact?, input?, completion? }` | Node |
| `r.fanout{ name?, children, items?, input?, completion? }` | Node |
| `r.sequence{ name?, entry?, output?, children, artifact?, input?, completion? }` | Node |
| `r.child{ node, inputs?, rules?, on_failure? }` | Child |
| `r.next(node)` | Rule |
| `r.when{ on, on_true, next }` | Rule |
| `r.switch{ on, cases, next? }` | Rule |
| `r.loop_guard{ max_iterations, on_exhausted }` | Rule |
| `r.retry(n)` / `r.ignore` | OnFailure |
| `r.input(name, contract?)` | Input |
| `r.request` / `r.items` | Source |
| `r.completion.approval` | Completion |
| `r.provider.claude` / `r.provider.codex` | Provider |
| `r.schema.object{ name?, properties, required? }` | Schema |
| `r.schema.array{ name?, items }` | Schema |
| `r.schema.string{ enum? }` / `boolean()` / `integer()` / `number()` | Schema |
| `r.workflow{ name, description, main }` | Workflow |

Node、Input、`r.request`、`r.items` は値参照として配線する。`node.field` は Artifact field の Source になる。children の要素はすべて `r.child{}` で書き、同じ Node 値を複数の child に置くことはできない。部品は Sequence を返す関数として作り、再利用時は関数を再度呼んで独立した Node 群を得る。

`require` は workflows ディレクトリ配下だけを探索し、合成後は単一の定義になる。評価環境は外部 I/O を持たず、命令数とメモリ量に上限がある。`.releash/releash.lua`、`.releash/facets.lua`、`.luarc.json` は LuaLS の補完用生成物にすぎず、load と実行の正本ではない。

## Diagnostic

Diagnostic は定義の検証結果であり lifecycle state ではない。

| 段階 | `Diagnostic.stage` | 責務 |
| --- | --- | --- |
| parse / shape | `parse_shape` | YAML/Lua の構文、root、field、kind、未知 field |
| resolve | `resolve` | Node、Contract、Artifact path、input source、facet の名前解決 |
| typecheck | `typecheck` | Contract、routing field、items と input の型 |
| control-flow | `control_flow` | 排他、網羅、到達性、cycle、children の制約 |

Rust backend が `code` / `stage` / `span` / `message` を返し、UI は表示だけを行う。YAML と Lua の同じ定義上の誤りには同じ domain Diagnostic が使われる。
