# Context

- Primary source は GitHub Issue #1734「[Session delegate と Node worktree 隔離] Session delegate」（https://github.com/siro33950/releash/issues/1734 、state: OPEN、label: enhancement、milestone: #85、comment なし）である。
- Issue が「設計（正本）」として指定する `docs/specs/milestone-85/design.md` §2「Session delegate」（§2.1 定義、§2.2 Session が所有する理由、§2.3 構文、§2.4 発火、§2.5 述語と評価、§2.6 上限、§2.7 親 Artifact の構造、§2.8 実行木での表現、§2.9 resume）も Primary source である。関連して同 §3.1 の表の「delegate の child」行、§6「述語の共通化」、§7「現行からの変更一覧」の「Session」「`ExecutionParentRef`」行を参照する。GitHub Milestone #85「01. Session delegate と Node worktree 隔離」の説明文と記述が食い違う場合は design.md に従う（milestone #85 説明文の指示）。
- 追加資料は `docs/glossary/WORKFLOW.md`（「Node の Interface と children の配線」「Session」「completion」「rules と辺」「Contract / schemas」「予約語」「Lua」「Lua API」「Diagnostic」の各節）、`docs/glossary/DOMAIN.md`（正規語の表の「completion」「述語（Predicate）」「NodeExecution」「AgentSession」行、「状態所有」節）、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）、GitHub Issue #1731 / #1732 / #1733、`docs/specs/issues-1731/`、`docs/specs/issues-1732/`、`docs/specs/issues-1733/`、`AGENTS.md`、`docs/architecture/`、および現行の Rust 実装（`src-tauri/src/domain/workflow/value_objects/definition.rs`、同 `node_execution.rs`、同 `predicate.rs`、`src-tauri/src/domain/workflow/services/validation.rs`、同 `reference.rs`、`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs`、`src-tauri/src/adaptor/gateway/workflow/completion_wire.rs`、同 `lua/mod.rs`、同 `lua/stubs.rs`、同 `workflow_host/lifecycle_commands.rs`、同 `workflow_host/prompt_rendering.rs`、`src-tauri/src/domain/workflow/services/prompt_composition.rs`、`src-tauri/src/adaptor/gateway/provider_lifecycle/launch_spec.rs`、`src-tauri/src/domain/workspace_tree/projection.rs`）と既存テストである。
- design.md §2.1 / §2.2 は次を定める。delegate は同一 session 継続機構であり、親 session が文脈を保ったまま child の結果を受けて自分で続行する。現行モデルの「実装 → 検証 → 通るまで直す」は辺のループで書けるが、後方辺で戻ると新しい NodeExecution と attempt が作られるため戻り先は別 session になる。delegate はこの往復と構造が同じで、戻り先が同じ session である点だけが違う。並列は child に置いた Fanout、直列は child に置いた Sequence が担い、delegate 自身は並列も直列も持たない。delegate は completion の一種であり `approval` と同型（本来の完了条件に条件を足して完了を保留する）で、Session だけが所有する。
- design.md §2.3 は構文を次のように定める。`completion` は map であり `require` と `delegate` を並べる。両方あるときの意味は and（述語が成立し、かつ human が承認して完了）で、or は持たない。`delegate` は `child`（必須。任意の Node を名前で参照する）、`inputs`（child が input を宣言しているとき）、`when`（必須。述語）、`max_iterations`（必須）を持つ。`inputs` は合成子の children エントリと同形で `<パラメータ名>: <供給元>` を書き、「配線は、その Node を子として扱う側が書く」という原則に従い、child を子として扱う親 session が配線を持つ。供給元は親 session のスコープで参照できるものすべて（親の `input` パラメータ、親の Artifact、`request`）であり、親の Artifact には前ラウンドの結果である `child` キーも含まれる。

    ```yaml
    implement:
      session:
        provider: codex
        facets:
          instruction: implement
      artifact: implement_result
      completion:
        require: approval
        delegate:
          child: verify
          inputs:
            task: implement_result
            spec: spec
          when: child.judge.clean
          max_iterations: 3
    ```

- design.md §2.4 は、発火は Artifact 提出であり、提出のたびに child が起動すると定める。発火を専用の typed command にしない理由は、agent が呼ばない選択をできてしまい engine が制御フローの唯一の権威であるという不変条件が崩れるためである。「Node の完了」を発火点にしない理由は、完了してから session を維持して作業を続けるのは完了の意味を壊すためである。
- design.md §2.5 は、`when` は真偽値を返す述語で `and` / `or` で合成でき、原子は Artifact の required boolean field への参照（親の Artifact の field、または `child.` から始まる child の Artifact の field）と定める。評価は「何かが完了したとき」（親が提出したときと child が完了したとき）に行い、参照先が何であるかによる場合分けはしない。参照先が未確定なら false になる。親が提出したとき、真なら完了（child は起動しない）、偽なら child を起動する。child が完了したとき、真なら完了、偽なら child の結果を注入して親が続行する。
- design.md §2.6 は、`max_iterations` は必須で、上限に達したら `when` の評価をスキップして完了させる能力と定める。child は上限回数だけ起動し、すべて注入されて親が判定を受け、上限に達した後の提出で完了する。上限に達して完了した場合、述語は false のままなので、その後の分岐は辺の `when` でそのまま区別できる。delegate に遷移先（`on_exhausted` 相当）は持たせない。必須にする理由は、辺の cycle に `loop_guard` を必須としている（`WFC005`）のと同じである。
- design.md §2.7 は、親が提出した Artifact に engine が `child` キーを足すと定める。提出直後は `child: null`、child 完了後は `child: <child Node の Artifact>` であり、child 名は挟まない。したがって child の kind によって参照の形が変わる（Session / Command: `child.passed`、Sequence: `child.<子Node名>.passed`、Fanout: `child.<添字またはchild名>.passed`）。`child` は予約キーであり、親の Artifact Contract に `child` field があれば load 時 Diagnostic になる。型検査は、親 Contract の `properties` に `child` → child Node の Artifact schema を足した合成 schema で静的に解決する。複数ラウンド回った場合、`child` は最後の結果で上書きされる。
- design.md §2.8 は、child の NodeExecution は親 session Node を親に持つ部分木として実行木に載り、発火ごとに attempt が増えると定める。現行の実行木は retry とループ再訪を区別しておらず、delegate の発火も同じ扱いで既存の attempt 機構に乗る。内部の親参照型 `ExecutionParentRef` は sequence child と fanout child の2種しか持たず、delegate child を3種目として追加する。
- design.md §2.9 は、child の再開は Fanout の子と同じ扱い（完了済みの child は Artifact を再利用し、未確定のものだけ再実行する）と定める。delegate 固有の扱いは、child の結果の親 session への注入を事実として記録することであり、child が完了した後・注入が済む前に中断した場合、resume 時に未注入であれば注入する。親 session の provider session が復元できない場合は resume が成立せず、既存の失敗経路（`on_failure` / 手動 Retry）に委ねる。
- design.md §3.1 の表は、delegate の child に `worktree: isolated` を宣言すると child の実行が隔離 worktree を持つと定める。#1733 は「delegate の child の隔離」を Non-goal とし、#1734 で delegate が入れば #1733 と同じ規則で効くと記す。
- Issue の「含むもの」には、`ExecutionParentRef` に delegate child を3種目として追加すること、型検査を合成 schema で静的に解決すること、child の結果の注入を事実として記録することという実装上の指定が含まれる。本文書はこれらを要求として扱わず、外部から観測できる結果（実行木での親子関係、load 時の型検査、注入前中断からの resume）だけを Requirements に書き、実装上の指定は Design で扱う。
- Issue は「正本サンプルへの delegate 適用」を求めるが、`workflows/examples/full-cycle-development.yml` のどの Node に delegate を宣言するかは Issue にも design.md §2 にも指定がない。design.md §2.3 の `implement` / `verify` は構文例であり、正本サンプルの Node 名ではない。適用先は Assumptions の自動判断で確定した。
- Issue と design.md §2.3 は `completion.delegate` の YAML 表記だけを示す。Lua 表面での `delegate` の受理形は確認した正本のどこにも定義されていない。Lua 表面の扱いは Assumptions の自動判断で未決とし、Non-goals に置いた。現行の Lua DSL は `completion = { require = r.completion.approval }` の table 形で completion を宣言し（#1732 で確定）、#1732 の Design は「#1734 は同じ table に `delegate` を足すだけで済む」ことを Lua 表面を table 形にした理由の一つとして記す。
- 依存する #1731（述語の Predicate 共通化）は commit 94963548、#1732（completion の map 化）は commit a3193720、同 Wave 3 の #1733（Node worktree 隔離）は commit c4a9f36c で main に取り込み済みである。#1731 は「`completion.delegate.when` は #1734」を、#1732 は「`completion.delegate`（#1734）。本 Issue では `delegate` は `completion` map の未知キーとして扱う」を Non-goal とする。
- workflow 定義は YAML と Lua の2つの表面を持ち、どちらも同じ `WorkflowDefinition` を構築し、同じ定義上の誤りには同じ `code`・`stage`・`message` の domain Diagnostic を使う。定義は未知 field、旧形式、互換 alias を受理せず、Error Diagnostic が一つでもある定義は実行できない（`docs/glossary/WORKFLOW.md`）。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。frontend に許すのは表示とレイアウト制御、入力の受付、`invoke` の呼び出し、表示用フォーマットだけである。
- milestone #85 の全 ISSUE 共通の境界は次のとおりである。各 ISSUE は 1 PR で完結し、コード・Diagnostic・テストに加えて `docs/glossary/WORKFLOW.md` / `DOMAIN.md` の該当節、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）の更新を含む。各 ISSUE 完了時点で全定義が Diagnostic ゼロで load でき、テストが通る状態を保つ。
- 現在状態の確認は、Issue、milestone、design.md、用語集、現行コード、既存テスト、現行の workflow 定義の読解によって行った。build / test / lint などの検証コマンドは実行していない。
- `docs/specs/issues-1734` は本文書の作成時点まで未作成であり、本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash の workflow 定義を YAML で書く開発者、「実装 → 検証 → 通るまで直す」のような往復を一つの agent session の文脈を保ったまま回したい開発者、および実行木を UI / CLI / API で観測して介入する人である。

現在、Session Node が自分の成果を別の Node に検証させ、その結果を受けて同じ session で作業を続ける手段はない。往復は辺のループとして書けるが、後方辺で戻るたびに新しい NodeExecution と attempt が作られ、戻り先は文脈を持たない別の session になる。`completion` に `require` 以外のキーを書いた定義は load 時に Error Diagnostic になり実行できない。

変更後、Session は `completion.delegate` で child Node を名前で宣言でき、Artifact を提出するたびに child が起動する。述語 `when` が親の提出時または child の完了時に成立すると Session は完了し、成立しなければ child の結果が親の Artifact の `child` キーとして注入され、同じ provider session が続行して再提出できる。`max_iterations` に達した後の提出では child を起こさず完了する。child は実行木上で親 Session の部分木として発火ごとの attempt として並び、child 完了後・注入前に中断しても resume で child を再実行せず注入から再開する。

# Current Behavior

現行の `completion` と Session の完了に関する挙動は次のとおりである。

| 対象 | 現行の挙動 |
| --- | --- |
| `completion` の受理形 | map。受理するキーは `require` だけで、値域は `approval`。全4種の Node で宣言できる。要素を持たない map、`require` 以外のキー、文字列形式は Error Diagnostic |
| `completion.delegate` を書く | YAML で `completion: { require: approval, delegate: {...} }` または `completion: { delegate: {...} }` を書くと、parse/shape 段の Error Diagnostic `WFS002`（field: `completion`、message: `completion map only accepts the key 'require'`）になり、その定義は load されない。Lua で `completion = { delegate = {...} }` を書いた場合も同じ `code`・`stage`・`message` になる。Session / Command / Fanout / Sequence のどの Node でも同じ |
| Session の完了 | 同一 attempt の Submit と provider Stop の二信号が揃ったときに、`completion` を省略していれば完了し、`require: approval` があれば WaitingApproval になる。二信号が揃った後に完了を保留する条件は `require: approval` だけである |
| Submit | agent が `releash workflow output submit --node-execution <node_execution_id>`（`artifact` を宣言した Session では `--type <Contract> --json <JSON>` を伴う）で提出する。Session の起動プロンプト末尾の「完了時の必須アクション」が、提出が成功したら追加の調査や tool 実行を行わずその turn を終了するよう指示する。同じ Node のより新しい attempt が既に開始されていれば、その提出は現行でない attempt として拒否される |
| Session の Artifact | `artifact` を宣言した Session は検証済み Artifact を含む Submit だけが有効で、その Contract の値が Artifact になる。`artifact` を宣言しない Session は `worktree: isolated` の場合を除き Artifact を持たない。engine が Session の Artifact に足す予約キーは `worktree`（`isolated` のとき）だけであり、`child` キーは存在しない |
| Artifact の予約キー | 全 Node で `worktree`、Command で `ok` / `exit_code` / `stdout` / `stderr` / `duration` を Contract の直下に再宣言すると load 時 Error Diagnostic になる。`child` を予約する検査はない |
| 参照の型検査 | 配線 `inputs`、辺の `when.on` / `switch.on`、`env`、テンプレート、`fanout.items` は Node の Artifact Contract（`isolated` なら `worktree` を合成した schema）を起点に各段を静的に解決する。`child` 段を解決する規則はない |
| 述語 | 辺の `when.on` に参照文字列、`and` / `or` の map、そのネストを置ける。各参照は自 Node の Artifact を起点にし、末端は required boolean field。実行時に値が存在しないか boolean でない参照は false として合成する（#1731） |
| 後方辺のループ | Sequence の辺で完了済み Node へ戻ると、その Node の新しい NodeExecution（新しい node_execution_id と attempt）が開始され、Session なら新しい provider session が起動する。前の attempt の provider session は再利用されない |
| 実行木の親参照 | root 以外の NodeExecution は親参照を持ち、親は合成子（Sequence / Fanout）の実行インスタンスである。Fanout の子だけが展開座標（items 行 / children 列）を持つ。Session / Command の NodeExecution を親に持つ NodeExecution は存在しない |
| 実行木の attempt 表示 | workspace tree は node 名と親参照が一致する NodeExecution を同じ再試行対象とみなし、最新 attempt 以外を retry history として扱う。UI・local API・CLI（`releash workflow status --json`）は実行木を親参照に沿って表示する |
| resume | resume の対象は Paused の NodeExecution と、provider process の終了で失敗した Session の NodeExecution だけである。Session は provider CLI を `--resume <provider session id>`（codex は `resume <id>`）で再起動し、初期指示を投入する。完了済み（Succeeded）の NodeExecution は再実行されない |
| 事実（fact） | NodeExecution の事実は Started / SessionAttached / CommandSpawned / ProcessExited / RuntimeFailureObserved / AgentActivityObserved / SessionNodeRenamed / ProviderSessionTitleObserved / SubmitReceived / SubmitRejected / StopReceived / ArtifactProduced / ApprovalGranted / RetryRequested / ResumeRequested / AbortRequested / ArchiveRequested / RestoreRequested である。child の結果の注入を表す事実はない |
| Lua 表面 | `completion = { require = r.completion.approval }` の table を受理する。table のキーは `require` だけで、`r.completion.approval` 以外の値は `WFS002`。生成 stub（`.releash/releash.lua`）の `ReleashCompletion` は `require` だけを注釈する |
| 正本ドキュメント | `docs/glossary/WORKFLOW.md` の「completion」節は「`require` 以外のキー（`delegate` を含む）は Error Diagnostic になる」と定める。「Session」節、「Lua API」の表、「予約語」節に delegate と `child` の記述はない。`docs/glossary/DOMAIN.md` の正規語の表は `completion` を「Node の完了に対する要求の集合。`require: approval` で承認を要求し、要求を書かないことが自動完了を意味する」、述語（Predicate）を「辺と completion の判断に使う真偽値の論理式」と定め、delegate の記述はない |

最小の再現手順と結果は次のとおりである（実装の読解による。コマンドは実行していない）。

1. `workflows/` に、Session Node `implement` の `completion` に `delegate` を書いた定義を置く。

    ```yaml
    nodes:
      main:
        sequence:
          children:
            - implement
      implement:
        session:
          provider: codex
          facets:
            instruction: implement
        artifact: implement_result
        completion:
          delegate:
            child: verify
            when: child.passed
            max_iterations: 3
      verify:
        session:
          provider: claude
          facets:
            instruction: verify
        artifact: verify_result
    ```

2. `releash workflow diagnostics --dir <dir>` を実行する。
3. parse/shape 段の Error Diagnostic `WFS002`（message: `completion map only accepts the key 'require'`）が出て、定義は load されず実行できない。`delegate` の内側に何を書いても、また `require: approval` と並べても結果は同じである。

現行の workflow 定義の実態は次のとおりである。

- `workflows/examples/full-cycle-development.yml` に `delegate` はない。`completion` は `spec_confirmation` / `implementation_confirmation` の `require: approval` の2箇所である。「実装 → 検証」の往復は、`implement_and_verify`（`implement_task` → `verify_task`）と `fix_and_verify`（`fix_task` → `verify_fix`）の `worktree: isolated` な Sequence を Fanout が item ごとに展開する形で書かれており、検証結果を受けて同じ session で直す経路はない。「書く → 不備なら前段を直して書き直す」（`authoring_behavior` / `authoring_design`）と「統合確認 → 修正」（`check_integration` / `fix_integration`）、「レビュー → 修正」（`review_scan` / `fix_round`）は、辺の後方遷移と `loop_guard` で書かれている。
- `workflows/*.yml`（builtin 8本）に `delegate` はない。
- 既存テストが、builtin 8本と正本サンプルが Diagnostic ゼロで load できること、YAML と Lua の `completion` map の誤りが同じ Diagnostic になることを確認している。

# Scope / Non-goals

## Scope

- Session の `completion` に `delegate`（`child` / `inputs` / `when` / `max_iterations`）を宣言できること。`require: approval` と並べて宣言できること。宣言できる Session の条件（`artifact` を宣言していること）、child の条件（Artifact を持つ Node であること、他の合成子・他の delegate の child と共有しないこと、親 Session 自身と親 Session を部分木に含む Node を指さないこと）、`max_iterations` の値域（1以上）。必須 field の欠落、Session 以外での宣言、存在しない child、`delegate` map の未知キー、および上記条件の違反の load 時 Diagnostic。delegate の child としてだけ参照される Node の到達可能性。
- 親 Session の Artifact 提出を発火点とし、専用の typed command を設けないこと。
- 述語 `when` の受理形（#1731 の述語と同じ構造。原子は親の Artifact の field と `child.` から始まる child の Artifact の field）、load 時の型検査、評価の時点（親の提出時と child の完了時）、参照先が未確定なら false になること。
- 評価結果に応じた進行。提出時に真なら child を起動せず完了、偽なら child を起動。child 完了時に真なら完了、偽なら child の結果を注入して親が同じ provider session で続行し再提出できること。
- `max_iterations` による上限。上限回数だけ child を起動し、上限に達した後の提出では `when` を評価せず child を起こさず完了すること。述語は false のままで、辺の `when` で区別できること。遷移先を持たないこと。
- 親 Artifact への `child` キーの合成（提出直後は `null`、child 完了後は child Node の Artifact そのもの、最後の結果で上書き）。`child` の予約キー検査。合成 schema による静的な型検査と、delegate の `when`、下流の配線、親 Session を自 Node とする辺からの `child.` 経由の参照。
- `inputs` の配線。配線先が child の宣言した input パラメータであること、供給元が親の `input` パラメータ、親 Session の Node 名で参照する親の Artifact、`request` であること、既存の配線の検査規則が同じく適用されること。
- 実行木での表現。child の NodeExecution が親 Session の NodeExecution を親に持つ部分木として載り、発火ごとに attempt が増え、UI・local API・CLI の実行木の観測経路で親 Session の下に並ぶこと。親 Session の NodeExecution と AgentSession が child の実行中も同一のまま保たれること。
- resume。child 完了後・注入前の中断から resume したとき child を再実行せず注入から再開すること。注入済みなら二重注入しないこと。child 自身の resume が Fanout の子と同じ扱いであること。親 session の provider session を復元できない場合は既存の失敗経路に委ねること。
- `require: approval` と `delegate` の併記が and であること。
- delegate の child の worktree が #1733 の規則に従うこと。`isolated` なら child の実行が隔離され、`shared`（省略を含む）なら親 Session の実行 worktree を引き継ぐこと。
- 正本サンプル `workflows/examples/full-cycle-development.yml` の `implement_task` への delegate 適用（child は `verify_task`）と、正本サンプル・builtin 8本の Diagnostic ゼロでの load。
- `docs/glossary/WORKFLOW.md` の「Session」「completion」「予約語」「Contract / schemas」の各節の更新と、「Lua」「Lua API」の各節が Lua 表面で `delegate` を受理しないことを記述すること。`docs/glossary/DOMAIN.md` の `completion` / 述語（Predicate）の定義と状態所有の記述の更新。
- delegate の一連（提出 → 起動 → 評価 → 注入 → 続行 → 完了）、親 Artifact を見る述語で child を起こさず完了する経路、上限到達での完了とその後の辺の分岐、child が Session / Command / Sequence / Fanout それぞれの場合の `child` キーの形と参照、注入前中断からの resume、必須 field 欠落と Session 以外での宣言の Diagnostic を対象とするテスト。

## Non-goals

- delegate 自身での並列・直列の表現。並列は child に置いた Fanout、直列は child に置いた Sequence が担う。
- 発火のための専用 typed command。発火は Artifact 提出だけである。
- 上限到達時の遷移先（`on_exhausted` 相当）。辺は Sequence が所有するため Session は遷移先を持たない。
- Session 以外の Node への `delegate` の解禁。Session 以外での宣言は Diagnostic である。
- 述語の構造・評価規則・`and` / `or` 以外の演算子（#1731 で確定した規則を変えない）。`switch` を delegate の条件に使うこと。
- `completion` map と `require: approval` の受理形と承認の実行時経路（#1732 で確定した規則を変えない）。
- Node worktree 隔離の規則（#1733 で確定）。delegate の child の隔離は #1733 の規則がそのまま効く。
- Fanout の子と異なる、delegate child 固有の resume・Retry 経路。child 固有なのは注入の扱いだけである。
- 親 Session の provider session を復元できない場合の新しい復旧経路。既存の失敗経路（`on_failure` / 手動 Retry）に委ねる。
- builtin 8本への delegate 適用。Issue は「正本サンプルへの delegate 適用」だけを挙げ、builtin は Diagnostic ゼロで load できることだけを確認する。
- 正本サンプルの `implement_task` 以外への delegate 適用。`fix_and_verify`（`fix_task` → `verify_fix`）、`check_integration` ↔ `fix_integration`、`authoring_behavior` / `authoring_design` の「書く → 直す」ループ、`review_scan` ↔ `fix_round` は現行の Sequence と後方辺のまま変更しない。
- Lua 表面での `completion.delegate` の受理。Lua の `completion` table は引き続き `require` だけを受理し、`delegate` を含む table は現行と同じ Error Diagnostic になる。生成 stub の型注釈も変えない。受理形は正本に定義がなく、親 Session 自身の Artifact の field を Lua の値参照で表す既存の手段がないため、本 Issue では決めない。
- `artifact` を宣言しない Session への `delegate` の解禁。Artifact の有無を「`artifact` 宣言があるか `isolated` であるか」で決める #1733 の規則を変えない。
- Artifact を持たない Node を delegate の child にすること。
- delegate の `inputs` に固有の型互換検査（供給元の Contract と配線先の型付き input の Contract の構造的互換の判定）。children エントリの配線に存在しない検査規則を delegate だけに足さず、children エントリの配線へ型互換検査を足すこともしない。
- delegate の child を合成子の child や別の delegate の child と共有すること。Node が一つの親だけを持つ現行の規則を変えない。
- `max_iterations: 0` の受理と、その場合の意味の定義。
- delegate を宣言しない Node の Artifact Contract に対する `child` field の拒否。`child` の予約は engine が `child` キーを合成する delegate 親の Contract に限る。
- 正本サンプルが参照する instruction facet の本文の新設・更新。正本サンプルが参照する facet の多くはリポジトリに存在せず、facet 本文は本 Issue の対象にならない。
- 実行木の UI への新しい表示要素の追加。child は既存の attempt 機構と親子表示に乗る。
- frontend へのロジック追加。
- 変更前に保存された実行の定義 snapshot の読み出し互換。旧形式の解釈経路と自動移行は追加しない。
- `docs/specs/milestone-85/design.md` の変更。design.md は既に変更後の構文と規則を記載している。

# Requirements

- R-001: `artifact` を宣言した Session の `completion` に `delegate` を宣言できる。`delegate` は `child`（必須。同じ定義の Node を名前で参照する）、`inputs`（任意。child が `input` を宣言しているときの配線）、`when`（必須。述語）、`max_iterations`（必須。1以上の整数）を持つ map である。`child` に指定できるのは Artifact を持つ Node（Command / Fanout / Sequence、および `artifact` を宣言したか `worktree: isolated` を宣言した Session）であり、その Node は合成子の child にも別の delegate の child にもなっていない Node でなければならない。delegate の child としてだけ参照される Node は `main` から到達可能とみなされ、到達不能の Diagnostic にならない。`delegate` だけを持つ `completion`、および `require: approval` と `delegate` を並べた `completion` のどちらも Error Diagnostic なく load できる。
- R-002: 次の定義は load 時に Error Diagnostic になり、実行できない。`child` を宣言しない `delegate`、`when` を書かない `delegate`、`max_iterations` を書かない `delegate`、`max_iterations` が 0 以下または整数でない `delegate`、存在しない Node 名を `child` に書いた `delegate`、Artifact を持たない Node を `child` に書いた `delegate`、親 Session 自身または親 Session を部分木に含む Node（`main` を含む）を `child` に書いた `delegate`、合成子の child または別の delegate の child である Node を `child` に書いた `delegate`、`child` / `inputs` / `when` / `max_iterations` 以外のキーを持つ `delegate`、`artifact` を宣言しない Session での `delegate` の宣言、および Session 以外の Node（Command / Fanout / Sequence）での `delegate` の宣言。
- R-003: delegate の発火は親 Session の Artifact 提出である。親 Session が Artifact を提出するたびに `when` が評価され、成立しなければ child が起動する。発火のための専用 typed command は存在せず、提出は既存の `releash workflow output submit` で行う。
- R-004: 親 Session が Artifact を提出した時点で `when` が真なら、child を起動せず、delegate による保留なしに Session は完了する（`require: approval` があれば R-015 に従う）。偽なら child を起動し、Session は完了せず child の完了を待つ。
- R-005: child が完了した時点で `when` が真なら Session は完了する。偽なら、child の Artifact を親 Session の Artifact の `child` キーとして注入し、親 Session は同じ provider session（同じ AgentSession、同じ NodeExecution、同じ attempt）で続行して再び Artifact を提出できる。再提出は R-003 に従って再び評価される。
- R-006: `when` は #1731 で確定した述語と同じ構造（参照文字列、`and` / `or` を唯一のキーとする map、そのネスト）を受理する。各参照は親 Session の Artifact を起点にし、親の Artifact の field、または `child.` から始まる child の Artifact の field を指す。各参照は load 時に R-010 の合成 schema で検査され、末端が required boolean でない参照、解決できない段を含む参照、要素が空の `and` / `or` は Error Diagnostic になる。実行時に参照先の値が存在しない、または boolean でない参照は false として評価され、親の提出時の `child.` 参照（`child` が `null` の時点）はこの規則により false になる。
- R-007: `max_iterations` は child を起動する回数の上限である。child は上限回数まで起動され、各回の結果は注入されて親が判定を受ける。上限回数の child がすべて完了した後の提出では、`when` を評価せず child を起こさず Session は完了する。この完了で述語は成立していないため、親 Session を自 Node とする辺の `when` で、述語が成立して完了した場合と区別できる。delegate は上限到達時の遷移先を持たない。
- R-008: delegate を宣言した Session の Artifact には、engine が `child` キーを足す。提出直後の値は `null`、child 完了後の値は child Node の Artifact そのものであり、child 名は挟まない。child が Session / Command なら `child.<field>`、Sequence なら `child.<子Node名>.<field>`、Fanout なら `child.<添字またはchild名>.<field>` で参照する。複数ラウンド回った場合、`child` は最後の child の結果で上書きされる。この Artifact は、Session が完了した後、下流の配線・辺・local API・CLI の既存の Artifact 取得経路から `child` キーを含む形で読める。
- R-009: `child` は delegate を宣言した Session の Artifact の予約キーである。delegate を宣言した Session が `artifact` に参照する Contract の直下に `child` field がある定義は load 時に Error Diagnostic になり、実行できない。この検査は delegate を宣言した Session の Contract にだけ適用され、delegate を宣言しない Node が `artifact` に参照する Contract の直下の `child` field は引き続き受理される。
- R-010: delegate を宣言した Session の Artifact の型は、親の Artifact Contract の `properties` に `child` → child Node の Artifact schema を足した合成 schema として load 時に静的に解決される。delegate の `when`、下流の配線 `inputs`（`<親Session>.child.<...>`）、親 Session を自 Node とする辺の `when.on` / `switch.on`（`child.<...>`）から、この合成 schema に沿って child の field を参照でき、既存の各段の検査規則（存在しない field、Object でない値から引く段、要求型を満たさない末端の Error Diagnostic）が同じく適用される。child の kind が Sequence / Fanout の場合は、それぞれの map の規則で段を辿る。
- R-011: `inputs` は `<パラメータ名>: <供給元>` の map であり、配線先は child が宣言した input パラメータでなければならない。供給元は親 Session の `input` パラメータ（field path 付きを含む）、親 Session 自身の Node 名で参照する親 Session の Artifact（field path 付きを含む。前ラウンドの `child` キーは `<親Session名>.child.<field>...` で参照する）、`request` である。親 Session の Node 名と親 Session の input パラメータ名が同じ名前に一致する配線は、children エントリの配線で兄弟 Node と input パラメータが衝突する場合と同じく曖昧として拒否される。Node 名を持たない無名インライン Session は自身の Artifact を供給元にできない。未宣言のパラメータへの配線、受理形でない供給元、Artifact を持たない供給元、解決できない field path は、children エントリの配線と同じ規則で load 時 Error Diagnostic になる。供給元の Contract と配線先の型付き input の Contract の互換は、children エントリの配線と同じく load 時に検査せず、両者が異なる Contract である配線も Error Diagnostic にならない。実行時、child の各起動では供給元をその時点の値（親の直近の提出と直近の `child`）で解決して渡す。
- R-012: child の NodeExecution は、親 Session の NodeExecution を親に持つ部分木として実行木に載る。発火ごとに同じ child Node の新しい NodeExecution が開始され attempt が増え、UI、local API の execution 取得、CLI の `releash workflow status --json` のそれぞれで、親 Session の下に発火ごとの行として観測できる。親 Session の NodeExecution は child の実行中も完了しておらず、child の完了後も同じ NodeExecution・同じ attempt・同じ AgentSession のまま続行する。
- R-013: child の再開は Fanout の子と同じ扱いであり、完了済みの child は Artifact を再利用し、未確定の child だけが再実行される。child が完了した後・注入が済む前に中断した WorkflowExecution を resume すると、child は再実行されず、child の結果が注入されて親 Session が続行する。注入が済んだ後の中断から resume した場合は二重に注入されない。二重注入の防止は親 Session が child ごとに結果の送付を一度だけ受理することで実現し、受理の後・provider に届く前に中断した場合は初回指示と同じく再送しない。
- R-014: 注入後の続行、および resume 後の続行は親 Session の provider session を復元して行う。provider session が復元できない場合は resume が成立せず、既存の失敗経路（children エントリの `on_failure`、手動 Retry）に委ねる。delegate 固有の復旧経路は存在しない。
- R-015: `require: approval` と `delegate` を並べた Session は、述語が成立して完了する条件（R-004 / R-005）または上限到達で完了する条件（R-007）を満たした後に WaitingApproval となり、人間の Approve で完了する。承認だけでは delegate の完了条件を代替できず、delegate の完了だけでは承認を代替できない。
- R-016: delegate の child の worktree は #1733 の規則に従う。child が `worktree: isolated` を宣言していれば、child の各起動（attempt）が親 Session の実行 worktree（親 Session が `isolated` ならその隔離 worktree）の HEAD から生成された隔離 worktree で実行され、child の Artifact に `worktree` キーが合成される。child が `shared`（省略を含む）なら、child は親 Session の実行 worktree をそのまま引き継ぎ、親 Session が `isolated` なら child も同じ隔離 worktree で実行される。
- R-017: `workflows/examples/full-cycle-development.yml` では、`implement_task` が `worktree: isolated` と Object Contract の `artifact` を宣言し、`completion.delegate` に `child: verify_task`、`inputs` の `task: task` と `spec: spec`、`when: child.complete`、`max_iterations` を宣言する。`implement_all` の children は `implement_task` を item ごとに直接展開し、Sequence `implement_and_verify` は残らない。`fix_and_verify`（`fix_task` → `verify_fix`）と正本サンプルの他のループは変更しない。正本サンプルと `workflows/*.yml`（builtin 8本）は本変更後も Diagnostic ゼロで load できる。
- R-019: `docs/glossary/WORKFLOW.md` の「Session」節と「completion」節は、`delegate` の受理形（`child` / `inputs` / `when` / `max_iterations`）、`artifact` を宣言した Session だけが宣言できること、child の条件（Artifact を持つ Node であること、他の合成子・他の delegate の child と共有しないこと、親 Session 自身と親 Session を部分木に含む Node を指さないこと）、`max_iterations` が1以上であること、`inputs` の供給元（親の `input` パラメータ、親 Session の Node 名で参照する親の Artifact、`request`）、発火が Artifact 提出であること、評価の時点と規則、上限の意味、`require: approval` との併記が and であること、親 Artifact の `child` キーの形と child の kind ごとの参照形、child の worktree が #1733 の規則に従うことを記述し、「`require` 以外のキー（`delegate` を含む）は Error Diagnostic になる」の記述を残さない。「予約語」節と「Contract / schemas」節は `child` を delegate を宣言した Session の Artifact の予約キーとして記述する。「Lua」節と「Lua API」の表は、Lua の `completion` table が `require` だけを受理し `delegate` を受理しないことを記述する。`docs/glossary/DOMAIN.md` は、`completion` の要求に `delegate` が含まれること、delegate が Session の所有する同一 session 継続機構であること、child の NodeExecution が親 Session の部分木であることを記述する。

# Assumptions / Open Questions

次はいずれも自動判断（人間の確認を経ず、規則で決めた仮定）である。

- 自動判断: 正本サンプルの delegate 適用先は `implement_task`（child `verify_task`）の1箇所とする。Issue は適用を求めるが適用先を指定せず、design.md §2.1 の動機「実装 → 検証 → 通るまで直す」と §2.3 の構文例（`implement` が `verify` に `task` / `spec` を配線し、検証結果の boolean を `when` に置く）に最も近い Node の組が `implement_task` / `verify_task` である。`verify_task` を delegate の child に移した Sequence `implement_and_verify` は残さず、`implement_task` に `worktree: isolated` を移して `implement_all` の child にする。`implement_task` が新たに宣言する `artifact` Contract の field 構成は正本に指定がなく、要求で固定しない。他の候補（`fix_and_verify`、`check_integration` ↔ `fix_integration`、authoring の各ループ、`review_scan` ↔ `fix_round`）への適用は最小の解釈として含めない。
- 自動判断: 未決。Lua 表面での `completion.delegate` の受理形。Issue、design.md、`docs/glossary/WORKFLOW.md` のいずれにも定義がなく、Lua の値参照（`<Node 値>.<field>`、`<Input>.<field>`、`r.request`、`r.items`）には親 Session 自身の Artifact の field を指す手段がないため、design.md §2.5 が求める「親の Artifact を見る述語」と「child の Artifact を見る述語」の両方を Lua で表す形はどの選択も正本に根拠を持たない。Lua 表面での受理は対象範囲から外し、Non-goals に置いた。
- 自動判断: `inputs` の load 時の検査規則は children エントリの配線と同じ集合（未宣言パラメータ、受理形でない供給元、曖昧な供給元、Artifact を持たない供給元、解決できない field path）に限り、供給元の Contract と配線先の型付き input の Contract の互換は検査しない。design.md §2.3 は `inputs` を合成子の children エントリと同形と定めるだけで型互換の規則を定めず、現行の children エントリの配線にも型互換の検査はない（要素 Contract の照合は Fanout の `items` 固有である）。children エントリの配線へ型互換検査を足すことは対象範囲の拡大であり、delegate だけに足すことは children と異なる規則を増やすため、どちらも採らない。
- 自動判断: `inputs` で親 Session 自身の Artifact を参照する名前は親 Session の Node 名とする。既存の配線が Artifact を Node 名で参照する規則をそのまま使い、予約供給元名や Contract 名という新しい参照の種類を増やさない。
- 自動判断: `delegate` は `artifact` を宣言した Session だけが宣言できる。design.md §2.4 / §2.7 は「親が提出した Artifact」を発火と `child` キー合成の前提とし、Artifact の有無を「`artifact` 宣言があるか `isolated` であるか」で決める #1733 の規則に第三の条件を足さない。
- 自動判断: `child` 予約キーの検査は delegate を宣言した Session の Contract にだけ適用する。design.md §2.7 は「親の Artifact Contract」に対する Diagnostic と定めており、engine が `child` キーを足さない Node の Contract を新たに拒否しない。
- 自動判断: `max_iterations` は1以上とし、0 以下は load 時 Error Diagnostic にする。design.md §2.6 は必須とする理由を `loop_guard` と同じとし、`loop_guard` の `max_iterations` は1以上である。0 のときの意味は正本に定義がなく、新しい挙動を増やさない。
- 自動判断: `child` は Artifact を持つ Node に限る。design.md §2.7 は child 完了後の `child` キーの値を child Node の Artifact そのものとし、既存の配線は Artifact を産出しない Node を供給元にできない。Artifact を持たない child を受理して `child` を `null` のままにする挙動は増やさない。
- 自動判断: delegate の child は他の合成子の child、別の delegate の child と共有できず、親 Session 自身と親 Session を部分木に含む Node も指せない。design.md §2.8 は child を親 Session の部分木とし、既存の規則は Node が一つの親だけを持つこと（`WFC006` / `WFC007`）と静的な包含 cycle の拒否を定めている。
- 自動判断: delegate の child としてだけ参照される Node は到達可能とみなす。design.md §2.3 の構文例と §2.8 の実行木は `verify` を合成子の children に置かず delegate だけで参照している。

Open Question はない。
