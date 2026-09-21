# Workflow 定義構文

この文書は Releash が受理する workflow 定義の正本である。YAML と Lua は load 後に同じ `WorkflowDefinition` になり、実行・事実ログ・read model・Session の Resume に定義形式の違いは残らない。語彙と実行モデルは [`DOMAIN.md`](./DOMAIN.md) を正とする。完成形の唯一の例は [`../../workflows/examples/full-cycle-development.yml`](../../workflows/examples/full-cycle-development.yml) である。

## 境界

| 境界 | 所有するもの |
| --- | --- |
| definition grammar | root、Contract、Node、children、rule の受理形 |
| load-time validation | Diagnostic、名前解決、型検査、control-flow 検査 |
| runtime | Node の実行、Artifact、辺の進行、実行木の Abort、Session の Resume、Command の Retry |
| execution trigger | UI / CLI / API からの WorkflowExecution 起動 |

定義は起動時刻、周期、外部イベント購読を持たない。未知 field、旧形式、互換 alias は受理せず、Error Diagnostic が一つでもある定義は実行できない。

保存済み実行の参照では、定義の解釈可否と記録の取得可否を分ける。現行の型に変換できないNode定義があっても、Nodeの識別・親子関係、Sessionの接続情報、Commandの保存済み出力を保持する。未使用・過去の定義が現在の処理に不要なら継続できる。必要な完了条件・遷移規則・入力・成果を確定できない処理だけを制限し、理由を示す。未対応定義を未起動や通常の失敗として扱わず、旧記法の実行規則や自動移行は追加しない。

合成Nodeの成果とSessionのWorkflow上の完了は定義と事実から再導出するため、定義を解釈できない実行の完了・成果を推定で補わない。Session自体の表示・接続・provider sessionの再開は、そのSessionの接続情報と個別の事実から行う。

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
| Session | provider CLI と継続対話する Node。delegate の子を持てる | `session:` |
| Command | shell command を一度実行する葉 Node | `command:` |
| Fanout | children を並列に束ねる合成子 | `fanout:` |
| Sequence | children を時系列に束ねる合成子 | `sequence:` |

合成子の child には4種すべてを置ける。Sequence の子の Sequence、Fanout の子の Sequence や Fanout も通常の再帰構造として扱う。

### Node の Interface と children の配線

Node 共通 field は kind block と同じ階層に書く。

| field | 意味 |
| --- | --- |
| `input` | Node が受け取るパラメータのリスト。文字列は型なし、`- name: Contract` は型あり |
| `artifact` | Node が産出する Artifact の Contract 名。Sequence / Fanout には宣言しない |
| `completion` | Node の完了に対する要求の map。`require: approval` で承認を要求する。`delegate` は Session の同一会話内の続行条件を指定する。要求しない場合は `completion` を省略する |
| `worktree` | `shared` / `isolated`。省略時は `shared` で、親の実行worktreeを継承する |

`input` と `artifact` は Node の Interface であり、`inputs` は children エントリまたは Session の `completion.delegate` に置く配線である。本文はパラメータ名を参照し、供給元 Node 名は配線にだけ現れる。

```yaml
nodes:
  judge:
    command: "printf '%s' \"$REVIEWS\" | jq '{all_lgtm: all(.[].lgtm)}'"
    env:
      REVIEWS: reviews
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

- Sequence の子: 兄弟エントリ、`<兄弟>.<field>...`、Sequence 自身の input、`request`。
- Fanout の子: Fanout 自身の input、field path、`request`、展開中の要素を表す `items`。
- `request` は起動時の String input で、どの合成子の配線からも参照できる。
- Fanout の子は並走するため、兄弟の Artifact を直接参照しない。外側の値は親から input を一段ずつ渡す。
- `items` は本文の特殊名ではない。child の input パラメータへ配線し、そのパラメータ名を本文で参照する。
- 配線先は child が宣言した input パラメータでなければならない。供給元は `<name>` または `<name>.<field>...` で、参照先の Node と各 field が存在し、Node 供給元は Artifact を産出する必要がある。Sequence は宣言なしに統合 map を産出し、`<sequence>.<child>.<field>...` で child の値を参照できる。
- Fanout は宣言なしに slot ごとの Artifact の map を産出し、`<fanout>.<キー>.<field>...` で slot の値を参照できる。合成子の内側にある場合も、`<合成子>.<fanout>.<キー>.<field>...` で参照できる。例えば Sequence `outer_seq` の child に `items` なしの Fanout `parallel_checks` があり、その child `check_a` が `passed` field を持つ Artifact を産出する場合、`outer_seq.parallel_checks.check_a.passed` で参照できる。合成子経由で辿る各段の slot は Artifact を産出する Node に限る。Command / Fanout / Sequence は常に Artifact を持つが、Session は `artifact` 宣言があるか `worktree: isolated` の場合に Artifact を持ち、参照の供給元と段にできる。配線 `inputs`、`when.on` / `switch.on`、`fanout.items` の3経路で同じ map を辿る。Fanout 全体を渡す場合は field path なしで配線する（例: `results: full_review_fanout`）。
- 同じ名前が Sequence の兄弟 Node と Sequence 自身の input パラメータの両方に一致する配線は曖昧なので拒否される。`request` と `items` は予約供給元名であり、Node の input パラメータ名には使えない。`request` / `items` に field は無く、`items` は `items` を宣言した Fanout 内だけで使える。

配線 `inputs`、Command の `env`、テンプレート `{{ }}`、`fanout.items` の field path は `.` で区切り、各段は先頭が ASCII 英数字、以降が ASCII 英数字・`-`・`_` である。参照文字列の前後に空白を含めることはできない。テンプレート `{{ }}` の内側にある空白だけは区切りとして扱い、参照文字列には含めない。段数に上限はない。起点に Contract がある参照は、各段を Object の `properties` に沿って load 時に解決する。存在しない field、Object でない値から field を引く段、または末端が参照箇所の要求型を満たさない参照は Error Diagnostic になる。中間 Object の field は `required` でなくてもよく、array の要素 Contract を経由して次の段を解決しない。共有 domain validation は、型なし input パラメータを起点とする field path の Contract 検査を行わず、実行時の Object 値を各段に沿って引く。`when.on` / `switch.on` の field path は後述の rules 固有の規則に従う。

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
2. kind block を持たない単一名 map: カタログ参照と、その children エントリでの `inputs` / `rules`。
3. kind block を持つ単一名 map: 名前付きインライン Node。load 時に同じカタログへ正規化される。
4. kind または Node 共通 field から始まる map: 無名インライン Node。`<合成子名>#<index>` の内部名へ正規化される。

合成子の `children` は非空でなければならず、カタログ参照は存在する Node を指す必要がある。同じ Node を同じ合成子から複数回参照すると `WFC007`、複数の合成子の child として共有すると `WFC006` になる。root の `main` は別の合成子の child にできない。

名前は配線や辺から参照するときだけ必要である。インライン宣言もカタログと同じ名前空間を使い、名前衝突は Diagnostic になる。

### Sequence

```yaml
main:
  sequence:
    entry: run_tests
    children:
      - run_tests
      - fix_tests:
          inputs:
            test_result: run_tests
          rules:
            - loop_guard:
                max_iterations: 3
                on_exhausted: give_up
            - next: run_tests
      - publish
```

- `entry`: 開始する children エントリ名。省略時は先頭。
- `children`: 実行対象と、各 child の配線・辺。
- `rules` を省略したエントリには、リストの次のエントリへ進む隣接辺がある。末尾では終端になる。
- `rules: []` は明示的な終端である。

Sequence の Artifact は、宣言なしに常に engine が産出する JSON object である。その実行で通り Artifact を産出した children の成果を、children エントリ名をキーにした統合 map として返す。各キーの値は child の Artifact そのものである。通らなかった child と Artifact を産出しなかった child はキーに現れず、該当する child がなければ空の object になる。ループで同じ child を複数回通った場合は、最後の結果が残る。Sequence 自身には `artifact` や Contract を宣言しない。

```json
{ "run_tests": { "ok": true }, "publish": { "version": "1.0.0" } }
```

Session / Command の起動が失敗した場合は、新しい attempt を作りながら最大 4 回自動で起動し直す。待機は 1、2、4、8 秒と長くなる。使い切っても Node は Running のままとし、プロセス在否を別に示す。プロセスが居ない Session は Resume、Command は Retry で利用者の介入を受け付ける。Session の Resume は会話と作業場所が残っていれば同じ attempt の会話を復旧し、いずれかが無ければ新しい attempt を起動する。既存会話の復旧時に会話の続きを促す指示は送らない。

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
- `items`: literal 配列、または Artifact の `<node>.<field>...` で末端が配列になる参照。
- item ごとに children を展開する。各要素は children の `inputs` で供給元 `items` から渡す。
- child の input が一つだけで `items` がある場合に限り、その `inputs` を省略できる。
- 型付き child input が `items` を受ける場合、Artifact 配列の要素 Contract または各 literal item がその Contract と一致する必要がある。
- children の Artifact は slot をキーで引ける map として Fanout の Artifact になる。各キーの値は、その slot の Artifact そのものである。
- `items` なしのキーは children エントリ名、`items` ありのキーは0から始まる展開順の添字の文字列である。
- `items` と複数 children を同時に宣言した場合は、item を外側・children エントリを内側とするフラットな並びに展開する。キーは `item_index * children.len() + child_index` で決まり、item ごとや child ごとの階層は作らない。
- 完了した slot が Artifact を産出しなかった場合（`artifact` を宣言せず `shared` で動く Session child など）はキーとして残り、値が `null` になる。
- `items` が空配列で slot が展開されない場合は空の object `{}` になる。

```json
{ "run_lint": { "ok": true }, "run_test": { "ok": true } }
```

上は `items` なしの例である。`items` が2件で children が `run_lint`、`run_test` の順なら、キーは次のようになる。`"0"` と `"1"` が最初の item、`"2"` と `"3"` が次の item の成果である。

```json
{ "0": { "ok": true }, "1": { "ok": true }, "2": { "ok": true }, "3": { "ok": true } }
```

添字キーは正準な10進表記だけを解決し、`007`、`+1`、`1.0`、前後に空白を含むキーは解決しない。load 時は対応する children エントリの Contract を検査し、実行時に決まる item 数による添字の上限は検査しない。無名インラインエントリの合成内部名（`<fanout名>#<index>`）も map のキーになるが、配線 `inputs` と `fanout.items` の field path では `#` を段に使えない。Lua で添字を参照する場合は `fan["0"].passed` と書く。field path の区切り・文字種・段数の規則は変わらない。

### Command

```yaml
materialize_requirements:
  command: "printf '%s' \"$DOC\" > \"$SPEC_DIR/requirements.md\""
  env:
    DOC: doc
    SPEC_DIR: context.spec_dir
  input:
    - doc
    - context
```

`artifact` を宣言する Command は、stdout 全体に対応する JSON を出力する。

```yaml
record_revision:
  command: 'jq -n --arg revision "$(git rev-parse HEAD)" ''{revision: $revision}'''
  artifact: revision_info
```

`command` は worktree を cwd として shell で一度実行する非空文字列である。結果は `ok`、`exit_code`、`stdout`、`stderr`、`duration` を持つ。`artifact` があれば stdout 全体を JSON として parse・Contract 検証し、予約 field と同じ Object Artifact に合成する。process 起動不能は起動失敗の事実として記録し、非ゼロ exit codeまたは stdout 検証失敗は `ok: false` の確定結果になる。

`env` は任意の map で、`<環境変数名>: <input パラメータ名>` または `<環境変数名>: <input パラメータ名>.<field>...` を宣言する。参照先は同じ Command が `input` で宣言したパラメータと、その Contract の Object を各段に沿って辿った field である。map 以外の値、受理形でない参照、未宣言パラメータ、型ありパラメータから解決できない field path は load 時に Error Diagnostic になる。`env` は Command だけに宣言でき、ほかの Node 種別での宣言は load 時に Error Diagnostic になる。

環境変数名は `[A-Za-z_][A-Za-z0-9_]*` に一致しなければならない。`RELEASH_` で始まる名前はすべて engine の予約名であり、宣言すると load 時に Error Diagnostic になる。参照した値が string なら文字列をそのまま、string 以外なら compact JSON テキストを子 process の環境変数へ渡す。値にテンプレート展開や shell 解釈は行わない。

テンプレートの `{{ parameter }}` / `{{ parameter.field... }}` は Node が宣言した input パラメータを参照する。field path を付ける場合、型ありパラメータではその Contract の Object を各段に沿って辿って解決できなければならない。未宣言パラメータと解決できない field path は拒否される。`{{ }}` は shell quoting を自動で行わないため、信頼できない値を shell syntax へ直接連結しない。信頼できない値は `env` で渡し、上の例の `"$DOC"` のように引用付きの shell 変数として参照する。

### Session

```yaml
review:
  session:
    provider: claude
    model: claude-opus-4-1
    permission: auto
    facets:
      policy: reviewing
      knowledge:
        - releash-review
      instruction: review-diff
  artifact: review_verdict
  completion:
    require: approval
```

- `provider`: 必須。`claude` または `codex`。
- `model`: 任意。指定した値を変換せず provider CLI の起動設定として渡す。`permission` の指定または省略によって意味は変わらない。
- `permission`: 任意。Releash が所有する provider 非依存の4値だけを受理し、provider ごとの起動フラグ列へ写像する。
- `facets`: `policy` / `knowledge` / `instruction` の参照。Session は少なくとも一つの facet 参照を必要とする。
- `artifact`: Submit に添付できる Artifact の Contract。宣言時は検証済み Artifact を含む Submit だけが有効。

| `permission` | 意味 | claude | codex |
| --- | --- | --- | --- |
| `manual` | ツールを使うたびに人間へ確認を求める | `--permission-mode default` | `--sandbox workspace-write --ask-for-approval on-request` |
| `auto` | 自動レビューを挟んで自動承認する | `--permission-mode auto` | `--approve-for-me` |
| `bypass` | 確認しない | `--permission-mode bypassPermissions` | `--dangerously-bypass-approvals-and-sandbox` |
| `read-only` | 書き込みを許さない | `--permission-mode plan` | `--sandbox read-only --ask-for-approval never` |

`manual` の確認頻度は provider 間で完全には一致しない。claude の `default` はツールの初回使用ごとに Claude Code が必ず確認を求め、codex の `on-request` は確認が必要かをモデルが判断する。この決定者の差は provider CLI の仕様として受け入れる。

`read-only` では provider CLI の判定方式が異なる。claude の `plan` は完了時の `releash workflow output submit` を拒否するため、Session Node は自動では完了しない。完了させる場合は人間が provider 側で権限を変更する。codex の `read-only` は同じ完了提出を拒否しない。

`completion` を省略した Session は、同一 Node attempt の Submit と provider Stop の二信号が揃ったときに完了する。順序は問わず、一方だけでは完了しない。`completion` に `require: approval` を宣言すると二信号が揃った後に WaitingApproval となり、人間の Approve で完了する。

Session に `completion.delegate` を指定すると、Artifact 提出を起点に child を実行して同じ会話で続行できる。受理条件、評価と上限は次の completion 節に従う。

### completion

`completion` は全4種の Node で宣言できる要求の map である。`require: approval` は本来の完了条件を満たした後に WaitingApproval となり、人間が承認するまで完了を保留する。要求しない場合は `completion` を省略し、本来の完了条件を満たした時点で完了する。

`completion` を書く場合は要求を一つ以上持つ。空 map、文字列形式、`approval` 以外の `require`、`require` / `delegate` 以外のキーは Error Diagnostic になる。`delegate` は `artifact` を宣言した Session だけが受理する。

| Node | `completion` 省略 | `require: approval` |
| --- | --- | --- |
| Session | Submit と provider Stop の二信号 | 二信号の後に Approve |
| Command | process 終了 | 終了後に Approve |
| Fanout | 全 child が決着 | 全 child 決着後に Approve |
| Sequence | 終端へ到達 | 終端到達後に Approve |

#### Session の delegate

`delegate` は親 Session が Artifact を提出したときに子 Node を実行し、その結果を同じ provider session へ返す仕組みである。Session は子の実行中も完了せず、結果注入後も同じ NodeExecution・attempt・AgentSession のまま続行する。提出後はいったん provider の turn を終了し、結果の追加入力を受けて作業と再提出を行う。

```yaml
implement:
  input: [task, spec]
  artifact: implementation_result
  session:
    provider: codex
    facets: {instruction: implement}
  completion:
    require: approval
    delegate:
      child: verify
      inputs: {task: task, spec: spec, result: implement}
      when:
        or: [done, child.complete]
      max_iterations: 3
```

`child` / `when` / `max_iterations` は必須、`inputs` は任意である。未知キー、欠落、不正な型、1未満の `max_iterations` は load 時 Error Diagnostic になる。child はカタログ上の Artifact を持つ Node を指す。Command / Sequence / Fanout、または `artifact` 宣言か `worktree: isolated` を持つ Session を指定できる。他の合成子や別の delegate の child と共有できず、親 Session 自身、root、親 Session を部分木に含む Node も指定できない。delegate だけから参照される child も到達可能である。

`inputs` の供給元は親の `input` パラメータ、親 Session の Node 名による親 Artifact（例: `implement.summary` / `implement.child.reason`）、`request` に限る。名前の曖昧さ、存在しない参照先・field、配線先の不一致を load 時に検査する。名前を持たない inline Session は自分の Artifact を Node 名で参照できない。各発火時に親の最新提出と直近の child の結果から解決して渡す。

`when` は required boolean field の参照文字列、または非空の `and` / `or` 配列を持つ map で、再帰的に合成できる。親の提出 field と `child` 以下の field を使い、既存の述語と同じ型検査を受ける。Artifact の提出直後と child 完了直後に評価し、実行時に未解決または非booleanの参照は false とする。提出直後に true なら child を起こさず、false なら child を起こす。child 完了後に true なら親の完了条件が成立し、false なら結果を親に注入して再提出を待つ。いずれの完了も Submit と provider Stop の二信号を必要とする。

`max_iterations` は child の発火回数の上限であり、親の attempt は増えない。N回目の child の結果が false なら結果を注入し、次の親の提出では述語を評価せず child も起こさず完了する。この最終提出にも直近の child の結果を保持し、後続の辺で参照できる。`require: approval` との併記は and であり、delegate の完了条件と二信号が揃った後に WaitingApproval へ移り、Approve により完了する。

engine は親の Artifact に予約キー `child` を合成する。未実行・実行中は `null`、完了後は child の Artifact そのものである。Session / Command の child は `implement.child.complete` のように直接参照する。Sequence は `implement.child.check.complete`、Fanout は items なしなら `implement.child.check.complete`、items ありなら `implement.child.0.complete` のように統合 map を辿る。配線・`when.on`・`switch.on` の参照と型検査はこの合成された形を使う。

child の NodeExecution は親 Session の部分木に発火ごとの行として載り、attempt が増える。プロセスが終了しても完了済み child の Artifact は再利用する。結果が未注入なら、親 Session Node の Resume 時にその結果を返す。親 Session は child ごとに結果の送付を一度だけ受理するため、注入済みの事実が残っていなくても再送しない。受理の後・provider に届く前の中断も再送せず、初回指示と同じく人間の介入に委ねる。再開できる会話または作業場所が無い場合は、Session Node の Resume で新しい attempt を起動する。

child の worktree も共通規則に従う。省略または `shared` なら親 Session の実行 worktree を引き継ぐ。`isolated` なら発火ごとの attempt が親 Session の worktree の HEAD から新しい隔離 worktree を作り、child Artifact に `worktree` を合成する。

### rules と辺

辺は Sequence の children エントリが `rules` として所有する。

```yaml
rules:
  - when:
      on: passed
      then: done
    next: fix
```

`when: { on, then }` と同じ要素の sibling `next` を維持し、`on` に述語を置く。述語は参照文字列、`and` を唯一のキーとする map、`or` を唯一のキーとする map のいずれかである。`and` / `or` の値は要素1つ以上の配列で、各要素にも同じ3形を置いてネストできる。空の合成はネスト内でも parse/shape 段の Error Diagnostic `WFS002` になり、load されない。

```yaml
# 全要素が true のとき done、それ以外は fix
rules:
  - when: { on: { and: [passed, clean] }, then: done }
    next: fix
```

```yaml
# いずれかが true のとき done、それ以外は fix
rules:
  - when: { on: { or: [passed, skipped] }, then: done }
    next: fix
```

```yaml
# passed AND (clean OR skipped)
rules:
  - when:
      on:
        and:
          - passed
          - or: [clean, skipped]
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

- `when.on` の各参照（Ref）は自 Node の Artifact を起点に Object を各段に沿って辿る。末端は直上 Object の `required` に含まれる boolean field でなければならず、中間 Object は required でなくてよい。全 Ref を load 時に検査し、非boolean・required でない末端・未宣言の段・非Objectから次の段を引く参照は Error Diagnostic `WFT001` になる。実行時に短絡され得る Ref も検査対象である。
- `and` は全要素が true、`or` はいずれかが true の場合に true になる。述語が true なら `then`、false なら sibling `next` へ進む。Artifact 全体・途中の値・末端の値が欠損しているか、末端の実行値が boolean でない場合、その Ref を false として合成する。欠損 Ref を含む `and` は false だが、`or` は他の Ref が true なら true になる。
- `switch.on` は自 Node Artifact の Object を各段に沿って辿り、末端の直上 Object で required になっている非空の string enum field。中間段は required でなくてよい。case が非網羅なら同じ要素の sibling `next` が必須。
- `when.on` の各 Ref と `switch.on` の field path は `.` で区切り、空の段および参照文字列全体の前後空白を拒否する。各段の文字種は制限せず、`legacy flag` のように空白を含む property 名は1段参照として引ける。`.` は段の区切りとして解釈されるため、`.` を含む property 名は `when.on` / `switch.on` から引けない。
- 単独の `next` は無条件辺。
- 一つの `rules` リストに置ける判別規則（`when` または `switch`）、`loop_guard`、単独 `next` はそれぞれ最大一つである。判別規則と単独 `next` は併記せず、判別規則自身の sibling `next` を catch-all に使う。
- 辺の target は存在する Node でなければならない。同じ Sequence の child またはどの合成子にも属さない Node へ遷移できるが、別の合成子が所有する child へ外から遷移できない。
- `switch` の case は enum 値だけを使う。case が非網羅なら sibling `next` が必須で、網羅していれば `next` は書かない。ただし Artifact を持つ Command の独自 field で分岐するときは、command failure の catch-all として `next` が必要である。
- Fanout の children エントリに `when` / `switch` を含む `rules` は置けない（`WFC007`）。Artifact を宣言しない Session child の field では分岐できない。Command の予約結果 `ok` は宣言なしで使える。Sequence を自 Node とする辺では、統合 map の `<child>.<field>...` を起点に分岐できる（例: `check_full_review_threads.has_open_threads`）。Fanout を自 Node とする辺にも `when` / `switch` を置け、`<キー>.<field>...`（例: `a.passed`、`0.verdict`）を起点に分岐できる。Sequence 経由の `<fanout>.<キー>.<field>...` も同じ規則で解決する。合成子の map のキー自体は required にせず、受理の可否は終端 field の型と、その直上 Object の `required` で決まる。これらの map を経由する参照も、`when.on` の各 Ref として合成できる。`switch.on` の指す値が存在しない場合は、どの case にも当たらない。非網羅 switch は sibling `next` へ進み、網羅 switch は `next` を書けないため進行エラーになる（前述の Command の command failure catch-all は例外）。
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

`artifact` は Session / Command で Object Contract を参照する。Sequence は child Artifact の統合 map、Fanout は child Artifact の map を engine が組み立てるため、どちらも `artifact` を宣言しない。routing field は `properties` と `required` の両方に必要である。Command の `ok` は宣言なしで boolean routing field として使える。全 Node の Artifact Contract の直下に `worktree` を再宣言しない。Command の Artifact Contract には標準結果 field の `ok` / `exit_code` / `stdout` / `stderr` / `duration` を再宣言しない。

delegate を宣言した Session の Artifact Contract の直下には `child` を宣言しない。engine が child の Artifact schema を合成して参照を検査する。delegate のない Node の `child` field は予約しない。

### 予約語

次は Node 名に使えない。

```text
command session fanout sequence input artifact completion env worktree
inputs rules items entry children
```

`child` は delegate を宣言した Session の Artifact の直下だけで予約する。Node 名や input パラメータ名全体を予約するものではない。

`request` と `items` は input 配線の予約供給元名であり、input パラメータ名には使えない。`request` は `schemas` の Contract 名としても使えない。

`worktree` は全4種の Node 共通 field である。YAML は `shared` / `isolated` の文字列だけを受理する。省略時と `shared` は、その Node を子として扱う実行の worktreeを継承する。`isolated` は宣言した NodeExecution の attempt ごとに、親 worktreeの HEAD から新しい branch と worktreeを作る。Sequence / Fanout に宣言すると children 全員がその1つの worktreeで動き、Fanout の child Node に宣言すると slot ごとに独立する。入れ子の `isolated` は直近の隔離 worktreeから分岐する。

engine は隔離 Node の Artifact に `worktree: { branch, path }` を合成する。`artifact` 宣言のない隔離 Session もこのキーだけを持つ Artifact を産出するため、Sequence の map に現れ、Fanout の slot は `null` にならない。合成子自身の `worktree` は children の map と同じ階層に入る。配線・env・テンプレート・fanout.items の参照は `worktree.branch` / `worktree.path` を string として解決できる。たとえば `seq.work.worktree.path` で隔離 child の pathを参照できる。

`worktree` は Artifact の予約キーでもあり、全 Node で Artifact Contract の直下への再宣言を拒否する。Node の `worktree` 宣言の有無によらない。branch/path は Artifact 産出前から Node の詳細、API、CLI に表示され、失敗後にも保持される。生成失敗は事実として記録し、Node の状態は Running のまま保つ。Session / Command の起動失敗には上記の自動再起動を適用する。engine は成果の統合や worktree・branch の削除を行わない。

## Lua

`.lua` は load 時に一度だけ評価され、YAML と同じ `WorkflowDefinition` を構築する。実行中に Lua は評価されない。chunk は `r.workflow{...}` を返す必要がある。Session の delegate は、構築後の handle に `work.delegate{ child, inputs?, when, max_iterations }` と宣言する。親自身の Artifact は `work` / `work.<field>...`、child の Artifact は `work.child.<field>...` で参照する。

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
  completion = { require = r.completion.approval },
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
| `r.command{ name?, command, env?, artifact?, input?, completion?: { require = r.completion.approval }, worktree? }` | Node |
| `r.session{ name?, provider, model?, permission?, facets?, artifact?, input?, completion?: { require = r.completion.approval }, worktree? }` | Node |
| `r.fanout{ name?, children, items?, input?, completion?: { require = r.completion.approval }, worktree? }` | Node |
| `r.sequence{ name?, entry?, children, input?, completion?: { require = r.completion.approval }, worktree? }` | Node |
| `work.delegate{ child, inputs?, when, max_iterations }`（Session handle のメソッド） | なし |
| `r.child{ node, inputs?, rules? }` | Child |
| `r.next(node)` | Rule |
| `r.when{ on, on_true, next }`（`on` は Source または Predicate） | Rule |
| `r.all{ ... }`（要素は Source または Predicate） | Predicate（and） |
| `r.any{ ... }`（要素は Source または Predicate） | Predicate（or） |
| `r.switch{ on, cases, next? }` | Rule |
| `r.loop_guard{ max_iterations, on_exhausted }` | Rule |
| `r.input(name, contract?)` | Input |
| `r.request` / `r.items` | Source |
| `r.worktree.shared` / `r.worktree.isolated` | Worktree（各 Node builder の `worktree` 値。文字列は受理しない） |
| `r.completion.approval` | CompletionRequirement（`completion` table の `require` に指定） |
| `r.provider.claude` / `r.provider.codex` | Provider |
| `r.schema.object{ name?, properties, required? }` | Schema |
| `r.schema.array{ name?, items }` | Schema |
| `r.schema.string{ enum? }` / `boolean()` / `integer()` / `number()` | Schema |
| `r.workflow{ name, description, main }` | Workflow |

`completion` は `completion = { require = r.completion.approval }` の table で宣言する。handle を table で包まず直接渡す旧形式は Error Diagnostic になる。`require` の値はこの handle だけを受理し、文字列は受理しない。

`delegate` は Artifact Contract を宣言した Session handle で一度だけ宣言できる。`child` は Node 値、`when` は Source または `r.all` / `r.any` の Predicate、`max_iterations` は1以上の整数で、いずれも必須である。任意の `inputs` は `<パラメータ名> = <Source>` の table で、親 Session の Input、親自身の Artifact 全体または field（`child` 以下を含む）、`r.request` を渡せる。

```lua
local check = r.session{
  name = "check",
  provider = r.provider.codex,
  facets = { instruction = f.instruction.implement_fix_plan },
  input = { r.input("result") },
  artifact = r.schema.object{
    properties = { complete = r.schema.boolean() },
    required = { "complete" },
  },
}
local work = r.session{
  name = "work",
  provider = r.provider.codex,
  facets = { instruction = f.instruction.implement_fix_plan },
  artifact = r.schema.object{
    properties = { done = r.schema.boolean() },
    required = { "done" },
  },
  completion = { require = r.completion.approval },
}
work.delegate{
  child = check,
  inputs = { result = work },
  when = work.child.complete,
  max_iterations = 3,
}
```

`when` は親自身の提出 field（`work.done` など）と `work.child` 以下の field を参照し、各 Ref は required boolean field でなければならない。delegate を宣言した Session の `child` は、child が Session / Command なら直接、Sequence / Fanout なら統合 map を辿る（`work.child.check.complete`、`work.child["0"].complete` など）。`inputs` / `when` は child の Artifact Contract に沿って load 時に型検査される。delegate だけから参照する child も定義に含まれる。

`completion = { require = r.completion.approval }` と delegate の併記は and になる。`completion = { delegate = {...} }` は引き続き `WFS002` で拒否する。同じ handle への二重宣言も `WFS002` / `parse_shape` になる。Session の Artifact Contract 直下に `delegate` field があっても、`work.delegate` はメソッドに解決され、その field を Lua の値参照には使えない。これによる Error Diagnostic の追加や YAML の参照規則の変更はない。

`r.all` / `r.any` は top-level の述語 builder である。要素は1つ以上で、空の builder はネスト内でも YAML と同じ parse/shape の Error Diagnostic になる。`r.when.on` の単一 Source は従来どおり使える。述語内の全 Source は、その辺の自 child の Artifact field を指し、多段 Object や Sequence / Fanout の map を辿れる。論理演算・型検査・欠損または非boolean値を Ref 単位で false とする評価は YAML と同じで、実行中に Lua で評価しない。

```lua
r.when{
  on = r.all{ judge.passed, r.any{ judge.clean, judge.skipped } },
  on_true = done,
  next = fix,
}
```

Lua の Command は `env = { <環境変数名> = <Input>, <環境変数名> = <Input>.<field>... }` で同じ対応を宣言する。

```lua
local doc = r.input("doc")
local context = r.input("context")

local materialize_requirements = r.command{
  command = [[printf '%s' "$DOC" > "$SPEC_DIR/requirements.md"]],
  env = {
    DOC = doc,
    SPEC_DIR = context.spec_dir,
  },
  input = { doc, context },
}
```

Node、Input、`r.request`、`r.items` は値参照として配線する。`node.field...` と Contract 付き Input の `input.field...` は Object を各段に沿って辿る Source になり、YAML と同じ多段 field path の解決規則を使う。Lua の child `inputs` では、Contract を持たない Input の `input.field...` を `WFR003`（`input does not declare a contract`）として拒否する。`r.request` と `r.items` に field はない。children の要素はすべて `r.child{}` で書き、同じ Node 値を複数の child に置くことはできない。部品は Sequence を返す関数として作り、再利用時は関数を再度呼んで独立した Node 群を得る。

`require` は workflows ディレクトリ配下だけを探索し、合成後は単一の定義になる。評価環境は外部 I/O を持たず、命令数とメモリ量に上限がある。`.releash/releash.lua`、`.releash/facets.lua`、`.luarc.json` は LuaLS の補完用生成物にすぎず、load と実行の正本ではない。

## Diagnostic

Diagnostic は定義の検証結果であり lifecycle state ではない。

| 段階 | `Diagnostic.stage` | 責務 |
| --- | --- | --- |
| parse / shape | `parse_shape` | YAML/Lua の構文、root、field、kind、未知 field |
| resolve | `resolve` | Node、Contract、Artifact path、input source、facet の名前解決 |
| typecheck | `typecheck` | Contract、routing field、items と input の型 |
| control-flow | `control_flow` | 排他、網羅、到達性、cycle、children の制約 |

Rust backend が `code` / `stage` / `span` / `message` を返し、UI は表示だけを行う。YAML と Lua の同じ定義上の誤りには、同じ `code`・`stage`・`message` の domain Diagnostic が使われる。`span` は各表面の位置付けに従う。
