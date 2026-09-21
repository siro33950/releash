# Lua リファレンス

この文書は、workflow を Lua で書く方法を説明します。各要素の意味は [Workflow の概念](./concepts.md)、置き場所・名前の規則・Diagnostic のコードは [共通リファレンス](./common.md) を参照してください。

Lua の定義は、読み込むときに YAML と同じ定義へ変換されます。変換後の形を確認できたものは「翻訳後の YAML」として載せています。YAML の書き方は [YAML リファレンス](./yaml.md) を参照してください。

例はすべて、`releash workflow diagnostics --dir` で Diagnostic が 0 件になる定義から抜き出しています。

## 最小の定義

`request` を Session で要約し、その Artifact を Command で出力する定義です。

例: `summarize-request.lua`

```lua
local r = require("releash")
local f = require("facets")

local summary = r.schema.object{
  name = "summary",
  properties = { text = r.schema.string{} },
  required = { "text" },
}

local task = r.input("task")
local summarize = r.session{
  name = "summarize",
  provider = r.provider.claude,
  facets = { instruction = f.instruction.summarize },
  input = { task },
  artifact = summary,
}

local result = r.input("result", summary)
local print_summary = r.command{
  name = "print_summary",
  command = [[printf "%s\n" "$TEXT"]],
  env = { TEXT = result.text },
  input = { result },
}

return r.workflow{
  name = "summarize-request",
  description = "request を要約し、要約文を出力する",
  main = r.sequence{
    children = {
      r.child{ node = summarize, inputs = { task = r.request } },
      r.child{ node = print_summary, inputs = { result = summarize } },
    },
  },
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

この定義は facet `instructions/summarize.md` を参照します。

```markdown
# request を要約する

次の依頼を3行以内で要約する。

{{ task }}
```

## 評価のしかた

- `.lua` は、定義を読み込むときに一度だけ評価されます。workflow の実行中に Lua は評価されません。
- ファイルは `r.workflow{}` の戻り値を返さなければなりません。返さないと `WFS010` になります。
- 使える標準ライブラリは `table`、`string`、`math` だけです。ファイルの読み書きなど外部との入出力はできません。
- 評価には上限があります。上限を超えると `WFS010` になります。

| 上限 | 値 |
| --- | --- |
| メモリ | 64MB |
| 命令数 | 50,000,000 |
| builder で作る値の数 | 100,000 |

- 構文の誤りは `WFS009` です。評価は最初の誤りで止まるため、Diagnostic は1件ずつ出ます。

## require

```lua
local r = require("releash")
local f = require("facets")
```

| モジュール | 内容 |
| --- | --- |
| `releash` | builder。この文書では `r` と呼ぶ |
| `facets` | facet の一覧。この文書では `f` と呼ぶ |
| それ以外 | workflows ディレクトリ配下の Lua ファイル。`require("dev.test_and_fix")` は `dev/test_and_fix.lua` を読む |

- 自分で書いたモジュールは、workflows ディレクトリの中だけから読めます。見つからないと `WFS011` です。
- workflows ディレクトリ直下の `.lua` は workflow 定義として読み込まれます。モジュールはサブディレクトリに置きます。
- モジュールが互いに `require` し合うと、エラーになります。

## 値と名前

Lua では、builder が返す値を変数に入れ、その値で Node や Contract を参照します。

- Node の `name` は任意です。付けた名前は Diagnostic と UI に表示され、YAML の Node 名と同じ規則に従います。
- `name` を付けない Node には `<親の名前>#<番号>` の名前が付きます。
- `name` を付けない Contract には `schema-<番号>` の名前が付きます。Diagnostic のメッセージにはこの名前が出ます。
- `r.workflow{}` の `main` に渡す Node には `name` を付けません。付けると `WFS006` です。
- 同じ Node の値は、一つの `r.child{}` にしか置けません（`WFC007`）。同じ部品を複数使う場合は、[関数で部品を作る](#関数で部品を作る) を参照してください。

## API 一覧

| API | 戻り値 |
| --- | --- |
| `r.workflow{ name, description, main }` | workflow |
| `r.session{ name?, provider, model?, permission?, facets?, artifact?, input?, completion?, worktree? }` | Session |
| `<Session>.delegate{ child, inputs?, when, max_iterations }` | なし |
| `r.command{ name?, command, env?, artifact?, input?, completion?, worktree? }` | Node |
| `r.sequence{ name?, entry?, children, input?, completion?, worktree? }` | Node |
| `r.fanout{ name?, children, items?, input?, completion?, worktree? }` | Node |
| `r.child{ node, inputs?, rules? }` | エントリ |
| `r.input(name, contract?)` | input |
| `r.request` / `r.items` | 供給元 |
| `r.next(node)` | rule |
| `r.when{ on, on_true, next }` | rule |
| `r.all{ ... }` / `r.any{ ... }` | 条件 |
| `r.switch{ on, cases, next? }` | rule |
| `r.loop_guard{ max_iterations, on_exhausted }` | rule |
| `r.completion.approval` | 承認の要求 |
| `r.worktree.shared` / `r.worktree.isolated` | worktree |
| `r.provider.claude` / `r.provider.codex` | provider |
| `r.schema.object{ name?, properties, required? }` | Contract |
| `r.schema.array{ name?, items }` | Contract |
| `r.schema.string{ enum? }` | Contract |
| `r.schema.boolean()` / `r.schema.integer()` / `r.schema.number()` | Contract |
| `f.policy.<key>` / `f.knowledge.<key>` / `f.instruction.<key>` | facet |

- builder は table を1つ受け取ります。`r.schema.string()` のように引数なしで呼ぶと `WFS002` です。
- 表にない引数を渡すと `WFS002` です。

## r.workflow

```lua
return r.workflow{
  name = "plan-and-count",
  description = "計画を立て、手順の数を数える",
  main = r.sequence{
    children = {
      r.child{ node = make_plan, inputs = { task = r.request } },
      r.child{ node = count_steps, inputs = { plan = make_plan } },
    },
  },
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

| 引数 | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `name` | string | 必須 | workflow 名。ファイル名（拡張子を除く）と一致させる。違うと `WFS006` |
| `description` | string | 必須 | 一覧に表示する説明 |
| `main` | Node | 必須 | 実行を始める Node。`name` を付けない |

## r.session

```lua
local task = r.input("task")
local make_plan = r.session{
  name = "make_plan",
  provider = r.provider.claude,
  model = "sonnet",
  permission = "auto",
  facets = {
    policy = f.policy.planner,
    knowledge = { f.knowledge["project-rules"] },
    instruction = f.instruction["make-plan"],
  },
  input = { task },
  artifact = plan,
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

| 引数 | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `name` | string | 任意 | Node 名 |
| `provider` | provider | 必須 | `r.provider.claude` / `r.provider.codex` |
| `model` | string | 任意 | provider CLI にそのまま渡すモデル名 |
| `permission` | string | 任意 | `"manual"` / `"auto"` / `"bypass"` / `"read-only"` |
| `facets` | table | 下記 | `policy`（facet 1つ）、`knowledge`（facet のリスト）、`instruction`（facet 1つ） |
| `input` | input のリスト | 任意 | 受け取るパラメータ |
| `artifact` | Contract | 任意 | Artifact の Contract（object） |
| `completion` | table | 任意 | [承認](#承認) を参照 |
| `worktree` | worktree | 任意 | [worktree](#worktree) を参照 |

- `facets` には少なくとも一つの facet が必要です。`-` を含む key は `f.instruction["make-plan"]` のように書きます。存在しない facet を参照すると `WFR900` です。
- `permission` の各値の意味と provider CLI の起動オプションは、[YAML リファレンスの session](./yaml.md#session) と同じです。

## r.command

```lua
local plan_input = r.input("plan", plan)
local count_steps = r.command{
  name = "count_steps",
  command = [[printf "%s" "$STEPS" | jq "{count: length}"]],
  env = { STEPS = plan_input.steps },
  input = { plan_input },
  artifact = step_count,
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

| 引数 | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `name` | string | 任意 | Node 名 |
| `command` | string | 必須 | worktree を作業ディレクトリにして shell で実行する文字列 |
| `env` | table | 任意 | `<環境変数名> = <input>` または `<環境変数名> = <input>.<field>...` |
| `input` | input のリスト | 任意 | 受け取るパラメータ |
| `artifact` | Contract | 任意 | stdout を読む Contract（object） |
| `completion` | table | 任意 | [承認](#承認) を参照 |
| `worktree` | worktree | 任意 | [worktree](#worktree) を参照 |

- `env` の値には、同じ Node の `input` に入れた input だけを使えます。`r.input` の値以外を渡すと `WFS002` です。
- field を辿る input には、Contract を付けてください（`r.input("plan", plan)`）。
- 環境変数名の規則と、`command` の中の `{{ }}` の注意は、[YAML リファレンスの command](./yaml.md#command) と同じです。

## r.sequence

```lua
main = r.sequence{
  entry = run_tests,
  children = {
    r.child{
      node = run_tests,
      rules = {
        r.when{ on = run_tests.passed, on_true = report, next = fix },
      },
    },
    r.child{
      node = fix,
      inputs = { result = run_tests },
      rules = {
        r.loop_guard{ max_iterations = 3, on_exhausted = report },
        r.next(run_tests),
      },
    },
    r.child{ node = report },
  },
},
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

| 引数 | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `name` | string | 任意 | Node 名 |
| `entry` | Node | 任意 | 開始する子の Node。省略すると先頭 |
| `children` | エントリのリスト | 必須 | `r.child{}` のリスト。空にできない |
| `input` | input のリスト | 任意 | 受け取るパラメータ |
| `completion` | table | 任意 | [承認](#承認) を参照 |
| `worktree` | worktree | 任意 | [worktree](#worktree) を参照 |

Sequence の Artifact は `<Sequence の値>.<子の名前>.<field>` で参照します。

## r.fanout

```lua
local path = r.input("path", file_path)
local review_one = r.session{
  name = "review_one",
  provider = r.provider.claude,
  facets = { instruction = f.instruction["review-file"] },
  input = { path },
  artifact = review,
}

local review_files = r.fanout{
  name = "review_files",
  items = list_files.files,
  children = {
    r.child{ node = review_one, inputs = { path = r.items } },
  },
}
```

```lua
local checks = r.fanout{
  name = "checks",
  children = {
    r.child{ node = r.command{ name = "lint", command = "pnpm lint" } },
    r.child{ node = r.command{ name = "test", command = "pnpm test" } },
  },
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

| 引数 | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `name` | string | 任意 | Node 名 |
| `children` | エントリのリスト | 必須 | 並列に動かす `r.child{}` のリスト。空にできない。最大 64 |
| `items` | 供給元 / table | 任意 | 展開する配列。`<Node>.<field>` か、配列の table |
| `input` | input のリスト | 任意 | 受け取るパラメータ |
| `completion` | table | 任意 | [承認](#承認) を参照 |
| `worktree` | worktree | 任意 | [worktree](#worktree) を参照 |

- `items` には input を直接渡せません。Artifact の配列 field を渡します。
- 要素は、子の `inputs` に `r.items` で渡します。
- 要素を型付きの input で受ける場合は、配列の要素と input に同じ Contract の値を使います（`file_path`）。`r.schema.string{}` を別々に呼ぶと別の Contract になり、`WFT003` になります。
- Artifact の参照は、`items` なしなら `checks.lint.ok`、`items` ありなら `review_files["0"].approved` です。

## r.child

`children` の要素は、すべて `r.child{}` で書きます。

```lua
local rework = r.sequence{
  name = "rework",
  children = {
    r.child{
      node = r.session{
        provider = r.provider.codex,
        facets = { instruction = f.instruction["second-opinion"] },
      },
    },
    r.child{
      node = r.session{
        name = "apply_rework",
        provider = r.provider.claude,
        facets = { instruction = f.instruction.rework },
      },
    },
  },
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

| 引数 | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `node` | Node | 必須 | 子にする Node。builder をその場で呼んでもよい |
| `inputs` | table | 任意 | `<パラメータ名> = <供給元>` |
| `rules` | rule のリスト | 任意 | Sequence の子だけ。次に進む先 |

`inputs` の供給元には次を渡せます。

| 供給元 | 値 |
| --- | --- |
| `r.request` | 起動時の依頼文 |
| Node の値 | 兄弟の Node の Artifact |
| `<Node>.<field>...` | その field |
| input の値 / `<input>.<field>...` | 親の Sequence / Fanout 自身の input |
| `r.items` | `items` を持つ Fanout の展開中の要素 |

- Contract を付けていない input の field を渡すと `WFR003` です。
- 配線先は、子が `input` で宣言したパラメータでなければなりません（`WFR007`）。

## rules

`r.child{}` の `rules` に、次に進む先を書きます。条件には、その子の Node の値から辿った field を使います。

| API | 振る舞い |
| --- | --- |
| `r.next(node)` | 常に `node` へ進む |
| `r.when{ on, on_true, next }` | `on` が true なら `on_true`、false なら `next` へ進む |
| `r.switch{ on, cases, next? }` | `on` の値に対応する `cases` の Node へ進む |
| `r.loop_guard{ max_iterations, on_exhausted }` | この子を上限回数だけ実行済みなら、代わりに `on_exhausted` へ進む |
| `rules = {}` | どこにも進まず Sequence を終える |

例: 真偽で分岐する（[r.sequence](#rsequence) の `run_tests`）。

例: 列挙で分岐する。

```lua
r.child{
  node = classify,
  rules = {
    r.switch{
      on = classify.verdict,
      cases = { SHIP = publish, HOLD = rework },
    },
  },
},
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

例: 複数の条件を組み合わせる。

```lua
r.child{
  node = checks,
  rules = {
    r.when{
      on = r.all{ checks.lint.ok, checks.test.ok },
      on_true = done,
      next = fix_checks,
    },
  },
},
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

- `r.all{}` は全部が true、`r.any{}` はどれかが true のときに true です。要素には field と `r.all{}` / `r.any{}` を混ぜて入れ子にできます。空にすると `WFS002` です。
- 遷移先（`r.next` の引数、`on_true`、`next`、`cases` の値、`on_exhausted`）は、同じ Sequence の子の Node でなければなりません（`WFR001`）。
- field の型の条件、`next` を書く・書かないの規則、一つの `rules` に置ける数、ループの `loop_guard` は、[YAML リファレンスの rules](./yaml.md#rules) と同じです。

## 承認

```lua
local report = r.session{
  name = "report",
  provider = r.provider.claude,
  facets = { instruction = f.instruction["write-report"] },
  completion = { require = r.completion.approval },
}
```

- `completion` は `{ require = r.completion.approval }` の table で書きます。`r.completion.approval` を直接渡したり、文字列を渡したりすると `WFS002` です。
- 4種類どの builder にも書けます。

翻訳後の YAML（参考）: 子を持つ Sequence に承認を要求する場合

```lua
local r = require('releash')
local f = require('facets')
return r.workflow{
  name = 'completion',
  description = 'test',
  main = r.sequence{
    children = { r.child{ node = r.command{ name = 'leaf', command = 'true' } } },
    completion = { require = r.completion.approval },
  },
}
```

```yaml
name: completion
description: test
nodes:
  main:
    sequence:
      children:
        - leaf:
            command: 'true'
    completion:
      require: approval
```

## delegate

Session の値の `delegate` メソッドで指定します。

```lua
local implement = r.session{
  name = "implement",
  provider = r.provider.codex,
  facets = { instruction = f.instruction["implement-task"] },
  input = { task },
  artifact = implementation,
  worktree = r.worktree.isolated,
  completion = { require = r.completion.approval },
}
implement.delegate{
  child = verify,
  inputs = { result = implement },
  when = implement.child.complete,
  max_iterations = 3,
}
```

| 引数 | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `child` | Node | 必須 | 実行する Node。Artifact を持つ Node に限る |
| `inputs` | table | 任意 | child への配線。親の input、親の Session の値と field（`implement`、`implement.child.<field>`）、`r.request` |
| `when` | 供給元 / 条件 | 必須 | 完了の条件。`implement.<field>` と `implement.child.<field>` を使う。`r.all{}` / `r.any{}` も使える |
| `max_iterations` | integer | 必須 | child を実行する回数の上限（1以上） |

- `delegate` は、`artifact` を持つ Session の値で一度だけ呼べます。2回呼ぶと `WFS002` です。
- `completion = { delegate = ... }` の形は受け付けません（`WFS002`）。
- 親の Contract に `delegate` という field があっても、`implement.delegate` はメソッドになり、その field の参照には使えません。

翻訳後の YAML（参考）

```lua
local r = require('releash')
local f = require('facets')
local result = r.schema.object{
  name = 'result',
  properties = { done = r.schema.boolean(), task = r.schema.string{} },
  required = { 'done', 'task' },
}
local verdict = r.schema.object{
  name = 'verdict',
  properties = {
    complete = r.schema.boolean(),
    detail = r.schema.object{
      properties = { clean = r.schema.boolean() },
      required = { 'clean' },
    },
  },
  required = { 'complete', 'detail' },
}
local task = r.input('task')
local check = r.session{
  name = 'check',
  provider = r.provider.codex,
  facets = { instruction = f.instruction.implement },
  artifact = verdict,
  input = { r.input('result') },
}
local work = r.session{
  name = 'work',
  provider = r.provider.codex,
  facets = { instruction = f.instruction.implement },
  artifact = result,
  input = { task },
  completion = { require = r.completion.approval },
}
local main = r.sequence{ children = { r.child{ node = work, inputs = { task = r.request } } } }
work.delegate{ child = check, inputs = { result = work }, when = work.child.complete, max_iterations = 3 }
return r.workflow{ name = 'delegate', description = 'test', main = main }
```

```yaml
name: delegate
description: test
schemas:
  result:
    type: object
    properties:
      done:
        type: boolean
      task:
        type: string
    required:
      - done
      - task
  verdict:
    type: object
    properties:
      complete:
        type: boolean
      detail:
        type: object
        properties:
          clean:
            type: boolean
        required:
          - clean
    required:
      - complete
      - detail
nodes:
  main:
    sequence:
      children:
        - work:
            inputs:
              task: request
  work:
    session:
      provider: codex
      facets:
        instruction: implement
    artifact: result
    input:
      - task
    completion:
      require: approval
      delegate:
        child: check
        inputs:
          result: work
        when: child.complete
        max_iterations: 3
  check:
    session:
      provider: codex
      facets:
        instruction: implement
    artifact: verdict
    input:
      - result
```

- Lua の `when = work.child.complete` は、YAML では `when: child.complete` になります。YAML は親の Artifact を起点に書くためです。

## worktree

| 値 | 振る舞い |
| --- | --- |
| 省略 / `r.worktree.shared` | 親の worktree で動く |
| `r.worktree.isolated` | 親の worktree の HEAD から新しい branch と worktree を作って動く |

- 文字列（`"isolated"`）は受け付けません。
- 隔離した Node の Artifact は `implement.worktree.branch` / `implement.worktree.path` のように参照できます。例は [delegate](#delegate) の `implement` を参照してください。

## r.schema

```lua
local plan = r.schema.object{
  name = "plan",
  properties = {
    title = r.schema.string{},
    steps = r.schema.array{ items = r.schema.string{} },
    risk = r.schema.string{ enum = { "low", "high" } },
  },
  required = { "title", "steps", "risk" },
}

local step_count = r.schema.object{
  name = "step-count",
  properties = { count = r.schema.integer() },
  required = { "count" },
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

| API | 引数 |
| --- | --- |
| `r.schema.object{}` | `name`（任意）、`properties`（field 名 → Contract）、`required`（field 名のリスト、任意） |
| `r.schema.array{}` | `name`（任意）、`items`（要素の Contract） |
| `r.schema.string{}` | `enum`（任意、空にできない） |
| `r.schema.boolean()` / `integer()` / `number()` | なし |

- 同じ Contract を複数の場所で使う場合は、同じ値を使います。
- `name` を付けない Contract には `schema-<番号>` の名前が付きます。

## 関数で部品を作る

同じ Node の値は一つの `r.child{}` にしか置けないため、繰り返し使う部品は Node を作って返す関数にします。

例: `dev/test_and_fix.lua`

```lua
local r = require("releash")
local f = require("facets")

local M = {}

function M.build(prefix, command)
  local run = r.command{ name = prefix .. "_run", command = command }
  local fix = r.session{
    name = prefix .. "_fix",
    provider = r.provider.codex,
    facets = { instruction = f.instruction["fix-tests"] },
  }
  local done = r.command{ name = prefix .. "_done", command = "true" }
  return r.sequence{
    name = prefix,
    children = {
      r.child{
        node = run,
        rules = { r.when{ on = run.ok, on_true = done, next = fix } },
      },
      r.child{
        node = fix,
        rules = {
          r.loop_guard{ max_iterations = 3, on_exhausted = done },
          r.next(run),
        },
      },
      r.child{ node = done },
    },
  }
end

return M
```

例: `check-both.lua`

```lua
local r = require("releash")
local test_and_fix = require("dev.test_and_fix")

return r.workflow{
  name = "check-both",
  description = "Rust と frontend をそれぞれ検査して直す",
  main = r.sequence{
    children = {
      r.child{ node = test_and_fix.build("rust", "cargo test") },
      r.child{ node = test_and_fix.build("web", "pnpm test") },
    },
  },
}
```

<!-- 翻訳後の YAML: 未記載。releash workflow export（#1753）の出力で確認してから記載する -->

- 関数を呼ぶたびに別の Node ができるので、名前が重ならないように引数から名前を作ります。名前が重なると `WFS006` です。

## 誤りやすい点

### 引数なしの builder

`r.schema.string()` は誤りです。`r.schema.string{}` と書きます。

```text
error WFS002 summarize-request.lua:6:1 [workflow=summarize-request]: builder expects exactly one table argument
```

### 別々に作った Contract

同じ型のつもりでも、builder を別々に呼ぶと別の Contract になります。

誤り:

```text
local file_list = r.schema.object{
  name = "file-list",
  properties = {
    files = r.schema.array{ items = r.schema.string{} },
  },
  required = { "files" },
}

local path = r.input("path", r.schema.string{})
```

```text
error WFT003 review-each.lua:33:1 [workflow=review-each, node=review_files, field=fanout.items]: fanout node 'review_files' items do not match child 'review_one' input: items element Contract 'schema-0' does not match parameter 'path' Contract 'schema-5'
```

正しい書き方は [r.fanout](#rfanout) の `file_path` です。
