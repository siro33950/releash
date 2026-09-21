# YAML リファレンス

この文書は、workflow を YAML で書く方法を説明します。各要素の意味は [Workflow の概念](./concepts.md)、置き場所・名前の規則・Diagnostic のコードは [共通リファレンス](./common.md) を参照してください。

例はすべて、`releash workflow diagnostics --dir` で Diagnostic が 0 件になる定義から抜き出しています。

## 最小の定義

`request` を Session で要約し、その Artifact を Command で出力する定義です。

例: `summarize-request.yml`

```yaml
name: summarize-request
description: request を要約し、要約文を出力する
schemas:
  summary:
    type: object
    properties:
      text:
        type: string
    required:
      - text
nodes:
  main:
    sequence:
      children:
        - summarize:
            inputs:
              task: request
        - print_summary:
            inputs:
              result: summarize
  summarize:
    session:
      provider: claude
      facets:
        instruction: summarize
    input:
      - task
    artifact: summary
  print_summary:
    command: 'printf "%s\n" "$TEXT"'
    env:
      TEXT: result.text
    input:
      - result: summary
```

この定義は facet `instructions/summarize.md` を参照します。

```markdown
# request を要約する

次の依頼を3行以内で要約する。

{{ task }}
```

## トップレベル

```yaml
name: plan-and-count
description: 計画を立て、手順の数を数える
schemas:
  step-text: string
```

| field | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `name` | string | 必須 | workflow 名。ファイル名（拡張子を除く）と一致させる |
| `description` | string | 必須 | 一覧に表示する説明 |
| `schemas` | map | 任意 | Contract 名 → Contract。[schemas](#schemas) を参照 |
| `nodes` | map | 必須 | Node 名 → Node。`main` が必要 |

- 上の表にない field を書くと `WFS002` になります。
- `builtin` も受け付けますが、書かないでください。builtin かどうかは Releash が名前で判断します。

## nodes

`nodes` は、Node 名をキーにした map です。実行は `nodes.main` から始まります。

```yaml
nodes:
  main:
    sequence:
      children:
        - make_plan:
            inputs:
              task: request
        - count_steps:
            inputs:
              plan: make_plan
```

各 Node は、種類を決める field（`session` / `command` / `sequence` / `fanout`）をちょうど一つ持ち、同じ階層に次の共通 field を書けます。

| field | 型 | 書ける Node | 説明 |
| --- | --- | --- | --- |
| `input` | list | すべて | 受け取るパラメータ。[input](#input) を参照 |
| `artifact` | string | Session / Command | Artifact の Contract 名 |
| `completion` | map | すべて | 承認と delegate。[completion](#completion) を参照 |
| `worktree` | string | すべて | `shared`（既定）/ `isolated` |
| `env` | map | Command | 環境変数。[command](#command) を参照 |

- 種類を決める field が無い、または2つ以上あると `WFS003` になります。
- Node の直下に `inputs` / `rules` を書くと `WFS007` になります。これらは Sequence / Fanout の children のエントリに書きます。
- Sequence / Fanout に `artifact` を書くと、Sequence では `WFS008`、Fanout では `WFT004` になります。

## session

```yaml
make_plan:
  session:
    provider: claude
    model: sonnet
    permission: auto
    facets:
      policy: planner
      knowledge:
        - project-rules
      instruction: make-plan
  input:
    - task
  artifact: plan
```

| field | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `provider` | string | 必須 | `claude` / `codex` |
| `model` | string | 任意 | provider CLI にそのまま渡すモデル名 |
| `permission` | string | 任意 | `manual` / `auto` / `bypass` / `read-only` |
| `facets.policy` | string | 下記 | policy facet の key（1つ） |
| `facets.knowledge` | list | 下記 | knowledge facet の key（複数可、この順で連結） |
| `facets.instruction` | string | 下記 | instruction facet の key（1つ） |

- `facets` の3つのうち、少なくとも一つを書きます。一つも無いと `WFR900` になります。
- `permission` は、provider CLI の起動オプションに次のように対応します。

| `permission` | 意味 | claude | codex |
| --- | --- | --- | --- |
| `manual` | ツールを使うたびに確認を求める | `--permission-mode default` | `--sandbox workspace-write --ask-for-approval on-request` |
| `auto` | 自動レビューを挟んで自動承認する | `--permission-mode auto` | `--approve-for-me` |
| `bypass` | 確認しない | `--permission-mode bypassPermissions` | `--dangerously-bypass-approvals-and-sandbox` |
| `read-only` | 書き込みを許さない | `--permission-mode plan` | `--sandbox read-only --ask-for-approval never` |

注: claude の `read-only` は完了時の提出コマンドを拒否するため、Session が自動では完了しません。

## command

```yaml
count_steps:
  command: 'printf "%s" "$STEPS" | jq "{count: length}"'
  env:
    STEPS: plan.steps
  input:
    - plan: plan
  artifact: step-count
```

| field | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `command` | string | 必須 | worktree を作業ディレクトリにして shell で実行する文字列。空にできない |
| `env` | map | 任意 | `<環境変数名>: <パラメータ>` または `<環境変数名>: <パラメータ>.<field>...` |

- `env` の値には、同じ Node の `input` で宣言したパラメータだけを使えます。値が string ならそのまま、それ以外は JSON 文字列として渡ります。
- 環境変数名は `[A-Za-z_][A-Za-z0-9_]*` です。`RELEASH_` で始まる名前は使えません（`WFR004`）。
- `command` の中の `{{ <パラメータ> }}` は値に置き換わりますが、shell の quoting は行いません。依頼文や Artifact のように中身を信頼できない値は、`env` で渡して `"$STEPS"` のように引用付きで参照してください。

## sequence

```yaml
main:
  sequence:
    entry: run_tests
    children:
      - run_tests:
          rules:
            - when:
                on: passed
                then: report
              next: fix
      - fix:
          inputs:
            result: run_tests
          rules:
            - loop_guard:
                max_iterations: 3
                on_exhausted: report
            - next: run_tests
      - report
```

| field | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `children` | list | 必須 | 子のエントリ。空にできない。[children](#children) を参照 |
| `entry` | string | 任意 | 開始するエントリの名前。省略すると先頭 |

Sequence の Artifact は `<Sequence>.<エントリ名>.<field>` で参照します。

## fanout

```yaml
review_files:
  fanout:
    items: list_files.files
    children:
      - review_one
review_one:
  session:
    provider: claude
    facets:
      instruction: review-file
  input:
    - path: file-path
  artifact: review
```

```yaml
checks:
  fanout:
    children:
      - lint:
          command: 'pnpm lint'
      - test:
          command: 'pnpm test'
```

| field | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `children` | list | 必須 | 並列に動かすエントリ。空にできない。最大 64 |
| `items` | string / list | 任意 | 展開する配列。`<Node>.<field>...` で Artifact の配列 field を参照するか、配列を直接書く |

- `items` を書いた場合、子の input へは供給元 `items` で要素を渡します。子の input が一つだけなら `inputs` を省略できます。
- 型付きの input で要素を受ける場合、配列の要素の Contract と一致させます。一致しないと `WFT003` になります。
- Fanout のエントリに `rules` は書けません（`WFC007`）。
- Artifact の参照は、`items` なしなら `checks.lint.ok`、`items` ありなら `review_files.0.approved` です。

## children

Sequence と Fanout の `children` のエントリには、4つの書き方があります。

```yaml
- classify:
    rules:
      - switch:
          on: verdict
          cases:
            SHIP: publish
            HOLD: rework
- publish:
    command: 'git push'
    rules: []
- rework
```

```yaml
rework:
  sequence:
    children:
      - session:
          provider: codex
          facets:
            instruction: second-opinion
      - apply_rework:
          session:
            provider: claude
            facets:
              instruction: rework
```

| 書き方 | 例 | 意味 |
| --- | --- | --- |
| 名前 | `- rework` | `nodes` の Node を参照する |
| 名前とエントリの field | `- classify:` の下に `rules` | `nodes` の Node を参照し、エントリの field を書く |
| 名前と Node | `- publish:` の下に `command` | その場で Node を宣言する。名前は `nodes` と共通 |
| Node だけ | `- session:` | 名前の無い Node を宣言する。`<親の名前>#<番号>` の名前が付く |

エントリには、Node の field と並べて次の field を書けます。

| field | 型 | 書けるエントリ | 説明 |
| --- | --- | --- | --- |
| `inputs` | map | すべて | `<パラメータ名>: <供給元>`。[input](#input) を参照 |
| `rules` | list | Sequence の子 | 次に進む先。[rules](#rules) を参照 |

- 同じ Node を2つの Sequence / Fanout の子にすると `WFC006`、同じ Sequence / Fanout に2回置くと `WFC007` になります。
- `main` は子にできません。
- 名前を付けていない Node は、辺や配線から参照できません。

## input

Node の `input` で受け取るパラメータを宣言し、エントリの `inputs` で値を渡します。

```yaml
input:
  - task
  - result: summary
```

| 要素 | 意味 |
| --- | --- |
| `- <名前>` | 型なしのパラメータ |
| `- <名前>: <Contract 名>` | 型付きのパラメータ |

`inputs` の供給元には次を書けます。

| 供給元 | 書ける場所 | 値 |
| --- | --- | --- |
| `request` | すべて | 起動時の依頼文 |
| `<Node>` | Sequence の子 | 兄弟の Node の Artifact |
| `<Node>.<field>...` | Sequence の子 | その field |
| `<パラメータ>` / `<パラメータ>.<field>...` | すべて | 親の Sequence / Fanout 自身の input |
| `items` | `items` を持つ Fanout の子 | 展開中の要素 |

- field は `.` で区切ります。各段は ASCII 英数字で始まり、ASCII 英数字・`-`・`_` だけを使います。
- 配線先は、子が `input` で宣言したパラメータでなければなりません（`WFR007`）。
- 兄弟の Node と親のパラメータの両方に同じ名前があると、曖昧なので `WFR008` になります。
- `request` と `items` はパラメータ名に使えません（`WFR008`）。

## rules

`rules` は、Sequence のエントリが次にどこへ進むかを指定します。条件は、そのエントリの Node の Artifact を参照します。

| 要素 | 形 | 振る舞い |
| --- | --- | --- |
| 無条件 | `next: <Node>` | 常にその Node へ進む |
| 真偽 | `when` の `on` と `then`、同じ要素の `next` | `on` が true なら `then`、false なら `next` へ進む |
| 列挙 | `switch` の `on` と `cases`、同じ要素の `next`（任意） | `on` の値に対応する `cases` の Node へ進む |
| 回数制限 | `loop_guard` の `max_iterations` と `on_exhausted` | このエントリを上限回数だけ実行済みなら、代わりに `on_exhausted` へ進む |
| 終了 | `rules: []` | どこにも進まず Sequence を終える |

`rules` を省略したエントリは、次のエントリへ進みます。

例: 真偽で分岐する（[sequence](#sequence) の `run_tests`）。

例: 複数の条件を組み合わせる。

```yaml
- checks:
    rules:
      - when:
          on:
            and:
              - lint.ok
              - test.ok
          then: done
        next: fix_checks
```

- `on` には、field の参照、`and: [...]`、`or: [...]` を書けます。`and` / `or` の要素にも同じ3つを書けるので、入れ子にできます。空の `and` / `or` は `WFS002` です。
- `when` の field は、Contract の `required` にある boolean でなければなりません（`WFT001`）。Command の `ok` は宣言なしで使えます。
- `switch` の field は、Contract の `required` にある string の `enum` でなければなりません（`WFT002`）。すべての値を `cases` に書いたら `next` は書かず、一部だけなら `next` が必要です（`WFC003`）。
- 一つの `rules` に、`when` / `switch`、`loop_guard`、単独の `next` はそれぞれ一つまでです（`WFC002`）。`when` / `switch` と単独の `next` は併記しません。
- 前のエントリへ戻るループには、ループのどこかに `loop_guard` が必要です（`WFC005`）。
- `next` / `then` / `cases` / `on_exhausted` の Node は、同じ Sequence の子か、どの Sequence / Fanout の子でもない Node です。別の Sequence の子へは進めません（`WFC006`）。
- すべての Node は `main` から到達できなければなりません（`WFC001`）。

## completion

```yaml
report:
  session:
    provider: claude
    facets:
      instruction: write-report
  completion:
    require: approval
```

| field | 型 | 説明 |
| --- | --- | --- |
| `require` | string | `approval` だけ。完了条件を満たした後に承認待ちにする |
| `delegate` | map | Session だけ。下記 |

- `completion` を書く場合は、`require` か `delegate` の少なくとも一方を書きます。空の map は `WFS002` です。

### delegate

```yaml
implement:
  session:
    provider: codex
    facets:
      instruction: implement-task
  input:
    - task
  artifact: implementation
  worktree: isolated
  completion:
    require: approval
    delegate:
      child: verify
      inputs:
        result: implement
      when: child.complete
      max_iterations: 3
verify:
  session:
    provider: claude
    facets:
      instruction: check-result
  input:
    - result
  artifact: check
```

| field | 型 | 必須 | 説明 |
| --- | --- | --- | --- |
| `child` | string | 必須 | 実行する Node の名前。Artifact を持つ Node に限る |
| `inputs` | map | 任意 | child への配線。供給元は親の input、親の Artifact（`implement`、`implement.summary`、`implement.child.<field>`）、`request` |
| `when` | string / map | 必須 | 完了の条件。親の Artifact を起点に書く（`child.complete` など）。`and` / `or` も使える |
| `max_iterations` | integer | 必須 | child を実行する回数の上限（1以上） |

- `delegate` は `artifact` を宣言した Session にだけ書けます。Session 以外では `WFC011`、`artifact` が無いと `WFT006` です。
- 親の Artifact の Contract に `child` を宣言できません（`WFT005`）。
- child に Sequence を指定した場合は `child.<子>.<field>`、`items` なしの Fanout は `child.<キー>.<field>`、`items` ありの Fanout は `child.0.<field>` のように参照します。

## worktree

| 値 | 振る舞い |
| --- | --- |
| 省略 / `shared` | 親の worktree で動く |
| `isolated` | 親の worktree の HEAD から新しい branch と worktree を作って動く |

隔離した Node の Artifact は `implement.worktree.branch` / `implement.worktree.path` のように参照できます。例は [delegate](#delegate) の `implement` を参照してください。

## schemas

```yaml
schemas:
  step-text: string
  plan:
    type: object
    properties:
      title: string
      steps:
        type: array
        items: step-text
      risk:
        type: string
        enum:
          - low
          - high
    required:
      - title
      - steps
      - risk
  step-count:
    type: object
    properties:
      count:
        type: integer
    required:
      - count
```

| 型 | 書き方 |
| --- | --- |
| object | `type: object`、`properties`（field 名 → 型）、`required`（field 名のリスト） |
| array | `type: array`、`items`（同じ `schemas` の Contract 名） |
| string | `type: string`、`enum`（任意、空にできない）。`string` とだけ書いてもよい |
| boolean / integer / number | `type: boolean` / `type: integer` / `type: number` |

- `properties` の中の型は、同じ書き方で入れ子にできます。
- `required` の field は `properties` に無ければなりません。
- array の `items` と、Node の `artifact` / `input` が参照する Contract は、同じ `schemas` に無ければなりません（`WFR002`）。

## 誤りやすい点

### エントリの field の階層

名前と Node を書くエントリでは、`rules` などのエントリの field を Node の field と同じ階層に書きます。

誤り:

```text
- apply_rework:
    session:
      provider: claude
      facets:
        instruction: rework
  rules: []
```

```text
error WFS008 42:11 [workflow=triage, node=rework, field=children]: children entry 'apply_rework' must be the only key in its mapping
```

`rules: []` は `apply_rework` の下に置き、`session` と同じ階層に揃えます。

### delegate の when の起点

`delegate.when` は、親の Artifact を起点に書きます。親の Node 名から始めません。

誤り:

```text
when: implement.child.complete
```

```text
error WFT001 39:9 [workflow=implement-with-check, node=implement, field=completion.delegate.when]: node 'implement' completion.delegate.when: routing field 'implement.child.complete' has undeclared segment 1 ('implement')
```

正しくは `when: child.complete` です。
