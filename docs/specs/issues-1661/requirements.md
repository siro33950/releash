# Context

- 入力: [Issue #1661 `[workflow] permission を provider 非依存の4値にし、builtin を全て auto にする`](https://github.com/siro33950/releash/issues/1661)
- 対象 spec: `docs/specs/issues-1661/`

確定済みの背景:

- `session.permission` は workflow 定義上 `Option<String>` であり、Releash は値を検証も写像もしない。`domain/workflow/value_objects/definition.rs` の `SessionSpec.permission`、`domain/agent_session/provider_launch.rs` の `ProviderLaunchOptions.permission` が同じ形で値を素通しし、`adaptor/gateway/provider_lifecycle/launch_spec.rs` が provider ごとに1個のフラグへ連結する。
- provider CLI の権限指定は軸が異なる。claude は `--permission-mode` の1フラグに「承認の求め方」と「到達範囲」を畳む。codex は `--sandbox`（到達範囲）と `--ask-for-approval` / `--approve-for-me`（承認の求め方）が直交する。
- Session node の完了には、node 自身が `releash workflow output submit` を shell 実行する必要がある。この完了アクションは `domain/workflow/services/prompt_composition.rs` の `artifact_completion_action` / `artifactless_completion_action` が全 Session node のプロンプトへ必ず付加する。したがって permission の写像先が書き込み系コマンドを拒否する場合、node は完了を提出できない。
- workflow 定義の正本は `docs/glossary/WORKFLOW.md`。現行記述は `model` / `permission` を「値を変換せず provider CLI の起動設定として渡す」と1行で束ねている。
- `workflows/examples/full-cycle-development.yml` は builtin 登録対象ではないが、`docs/glossary/WORKFLOW.md` が「完成形の唯一の例」として参照し、`adaptor/gateway/workflow/diagnostics.rs` と `domain/workflow/entities/workflow_execution/mod.rs` のテストが実ファイルを読んで検証する。permission の受理形を変えると、この例も同じ受理形に従う必要がある。

計測で確認した provider CLI の受理形（本 spec 作成時点の実機）:

| CLI | version | 受理値 |
| --- | --- | --- |
| claude | 2.1.240 | `--permission-mode` = `acceptEdits` / `auto` / `bypassPermissions` / `manual` / `dontAsk` / `plan`。`default` は choices に列挙されないが受理される |
| codex | codex-cli 0.149.0 | `--sandbox` = `read-only` / `workspace-write` / `danger-full-access`、`--ask-for-approval` = `on-request` / `never` ほか、`--approve-for-me`、`--dangerously-bypass-approvals-and-sandbox` |

# Outcome

対象は、workflow 定義を書き、読み、実行する Releash の利用者である。

現在、`session.permission` には provider CLI の値をそのまま書く。そのため次の問題がある。

- 定義を読んでも node に何を許しているのかが分からない。`acceptEdits` と `danger-full-access` が同じ意図なのか違う意図なのかを、定義だけからは判定できない。
- `provider` を差し替えると `permission` の意味が変わる。値は検証されないため、差し替えた側で無効な値になっても load 時には分からない。
- claude の `auto`（自動レビューを挟んだ自動承認）に相当する指定を codex で書けない。codex 側の対応物は `--sandbox` の値ではなく `--approve-for-me` という別フラグであり、現行実装は permission を `--sandbox` にしか渡さない。

変更後は、`permission` が provider 非依存の意味を持つ固定値になり、provider ごとの起動フラグへの写像は Releash が持つ。定義を読めば node に何を許しているのかが分かり、`provider` を差し替えても意味が保たれ、provider 固有値は load 時に拒否される。

# Current Behavior

## permission の受理と伝播

`session.permission` は任意の文字列を受理する。値の検証は行われない。

```yaml
nodes:
  main:
    session:
      provider: claude
      permission: acceptEdits
      facets:
        instruction: implement_fix_plan
```

`permission: totally-bogus` と書いても load は成功し、Diagnostic は出ない。誤りは provider CLI の起動時にしか現れない。

値は `SessionSpec.permission` → `WorkflowSessionLaunchConfig.permission`（`adaptor/gateway/workflow/node_session_boundary.rs`）→ `WorkflowAgentSessionLaunchRequest.permission`（`usecase/agent_session/agent_session_launch.rs`）→ `ProviderLaunchOptions.permission` と、いずれも `Option<String>` のまま運ばれる。

## 起動フラグへの連結

`adaptor/gateway/provider_lifecycle/launch_spec.rs:180-189` が、provider ごとに固定の1フラグへ値を連結する。

- claude: `--permission-mode <値>`
- codex: `--sandbox <値>`

フラグ名は provider ごとに1つに固定されており、複数フラグを組み立てる経路はない。`permission` が未指定なら、フラグ自体を付けない。

## builtin の現状値

builtin 8本の Session node は 58 個ある。permission は全 58 個に指定されている。

| 定義 | Session 数 |
| --- | --- |
| `workflows/01_author-spec.yml` | 8 |
| `workflows/02_implement-existing-spec.yml` | 5 |
| `workflows/03_full-review.yml` | 16 |
| `workflows/04_review-fix-policy.yml` | 3 |
| `workflows/04_review-fix-policy-manual.yml` | 3 |
| `workflows/05_review-fix.yml` | 5 |
| `workflows/06_handle-pr-review.yml` | 9 |
| `workflows/06_handle-pr-review-manual.yml` | 9 |

値は provider ごとに1種類しかない。`provider: claude` の 25 個はすべて `acceptEdits`、`provider: codex` の 33 個はすべて `danger-full-access` である。

`workflows/examples/full-cycle-development.yml` は上記に含まれないが、`acceptEdits` / `danger-full-access` に加えて `bypassPermissions` と `workspace-write` を使っている。

## 完了アクションの実行可否

Session node は完了時に `releash workflow output submit` を shell 実行する。現在の builtin 値でこれが通ることは、実行実績から確認できる。

Issue が写像先候補として挙げる値については、node のプロンプトと同形の入力（完了時の必須アクションとして submit コマンドを提示）を非対話形の provider CLI へ与え、コマンドが実行されるかを確認した。存在しない node execution id を渡しているため、実行された場合は local API が id を拒否して exit code 4 で終わる。

| provider | 与えた起動フラグ | 完了アクション |
| --- | --- | --- |
| claude 2.1.240 | `--permission-mode auto` | 実行される（exit code 4） |
| claude 2.1.240 | `--permission-mode plan` | 拒否される |
| codex 0.149.0 | `--approve-for-me` | 実行される（exit code 4） |
| codex 0.149.0 | `--sandbox read-only` | 実行される（exit code 4） |

拒否されるのは claude の `plan` だけである。plan mode が非 read-only ツールの実行を禁じるため、コマンドは実行されず node は完了を提出できない。

```text
$ claude --permission-mode plan --print '<node プロンプトと同形の入力>'
plan mode が有効なため、指示された提出コマンドを実行できませんでした。
… 副作用のある実行（Artifact 提出）であり、plan mode では非 read-only ツールを実行できません。
```

codex は `--approve-for-me`（承認を自動レビューへ回す）でも、`--sandbox read-only` と承認を求めない非対話実行（対話形の `--ask-for-approval never` に相当）でも、コマンドを実行する。完了アクションは local API への提出だけでファイル書き込みを伴わないため、read-only sandbox の禁止対象に当たらない。

# Scope / Non-goals

## 変更する

- `session.permission` の受理値を provider 非依存の固定値集合に変える。
- 受理値集合を外れる値を load 時に Error Diagnostic として拒否する。
- 抽象値から provider ごとの起動フラグ列への写像を Releash が持つ。写像先は1フラグ固定ではなく、provider ごとに複数フラグになりうる。
- builtin 8本 58 Session の `permission` 値を差し替える。
- `workflows/examples/full-cycle-development.yml` の `permission` 値を新しい受理値へ差し替える。この例は `docs/glossary/WORKFLOW.md` が参照し、テストが実ファイルを読んで検証するため、受理形の変更に追随しなければ不整合になる。
- Lua API の `permission` の受理形と、LuaLS 補完用 stub（`.releash/releash.lua`、生成元は `adaptor/gateway/workflow/lua/stubs.rs`）を新しい受理値に合わせる。
- `docs/glossary/WORKFLOW.md` の `model` / `permission` の記述を改訂する。

## 変更しない

- `session.model`。値は provider CLI へ無変換で渡すままとする。
- `provider` の受理値（`claude` / `codex`）と provider の追加。
- Session 以外の Node 種別、completion、facets、artifact、rules の受理形。
- 実行時に permission を変更する経路の新設。permission は定義に書いた値だけが効く。
- provider CLI 側の権限判定そのもの。Releash は起動フラグを選ぶだけで、承認判定は provider CLI が行う。
- 永続化済みの実行履歴との後方互換。event store に残る provider 固有 permission 値を読めるようにする措置と、その移行手段は設けない。

# Requirements

- R-001: `session.permission` は provider 非依存の4値 `manual` / `auto` / `bypass` / `read-only` だけを受理する。各値の意味は provider によらず次のとおりである。`manual` はツールを使うたびに人間へ確認を求める。`auto` は自動レビューを挟んで自動承認する。`bypass` は確認しない。`read-only` は書き込みを許さない。
- R-002: `permission` は任意である。省略した場合、Releash は permission に対応する起動フラグを付けず、provider CLI の既定に委ねる。
- R-003: R-001 の4値以外の値を書いた定義は、load 時に Error Diagnostic を持ち、実行できない。provider 固有値（`acceptEdits`、`danger-full-access`、`workspace-write`、`bypassPermissions`、`plan` 等）も受理しない。同じ意味に複数の書き方を作らないため、これらを新しい値への alias として受理することもしない。
- R-004: Diagnostic は、値が不正である旨と、当該 `permission` field の位置を示す。YAML と Lua の同じ誤りには同じ Diagnostic が使われる。
- R-005: 抽象値は provider ごとの起動フラグ列へ写像される。写像は次のとおりである。

  | 抽象値 | claude | codex |
  | --- | --- | --- |
  | `manual` | `--permission-mode default` | `--sandbox workspace-write --ask-for-approval on-request` |
  | `auto` | `--permission-mode auto` | `--approve-for-me` |
  | `bypass` | `--permission-mode bypassPermissions` | `--dangerously-bypass-approvals-and-sandbox` |
  | `read-only` | `--permission-mode plan` | `--sandbox read-only --ask-for-approval never` |

- R-006: `manual` の意味は両 provider で完全には一致しない。claude の `default` は「ツールの初回使用ごとに必ず聞く」、codex の `on-request` は「モデルが必要と判断したときに聞く」であり、聞く頻度の決定権が Claude Code 側とモデル側に分かれる。この差は仕様として受け入れ、`docs/glossary/WORKFLOW.md` に明記する。
- R-007: `permission: auto` を指定した Session node は、完了時の `releash workflow output submit` を実行できる。builtin が依存する経路であり、この値では完了アクションが provider CLI に拒否されてはならない。
- R-008: `permission: read-only` を指定した Session node は、claude では完了時の `releash workflow output submit` が拒否されるため、自動では完了しない。codex では拒否されない。この provider 差は、`read-only` の意味（書き込みを許さない）に対する provider CLI の判定方式の違いから生じる帰結として受け入れる。claude で完了させる必要が生じた場合は、人間が provider 側で権限を変更する。
- R-009: builtin 8本の 58 Session はすべて `permission: auto` になる。`provider` の別によらず同じ値になる。
- R-010: `workflows/examples/full-cycle-development.yml` は R-001 の受理値だけを使い、Error Diagnostic を持たない。
- R-011: Lua で書いた定義も R-001 の4値だけを受理し、R-003 と同じ拒否になる。LuaLS 補完用 stub は `permission` の受理値を `string` ではなく4値として示す。
- R-012: `docs/glossary/WORKFLOW.md` は、`permission` が Releash の所有する4値であり provider ごとの起動フラグへ写像されること、`model` は写像せず provider CLI へ無変換で渡すことを、区別して記述する。あわせて R-006 の `manual` の provider 間差異と、R-008 の `read-only` が claude では自動完了しないことを記述する。
- R-013: provider 固有の permission 値を含む fact が永続化されている実行木は、変更後は履歴を読み出せない。当該実行木を含む起動時 reconciliation はエラーを返す。旧値を読めるようにする移行手段は提供しない。

# Assumptions / Open Questions

## Assumptions

- A-001: `read-only` を指定した Session node が claude では自動完了しないことを、ユーザーが明示的に受け入れた。完了が必要な場面では人間が provider 側の権限を変更するという前提である。

## Open Questions

なし。
