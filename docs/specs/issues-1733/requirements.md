# Context

- Primary source は GitHub Issue #1733「[Session delegate と Node worktree 隔離] Node worktree 隔離」（https://github.com/siro33950/releash/issues/1733 、state: OPEN、label: enhancement、milestone: #85、comment なし）である。
- Issue が「設計（正本）」として指定する `docs/specs/milestone-85/design.md` §3「Node worktree 隔離」と §4.3「isolated な Node」も Primary source である。関連して同 §4.1「Sequence」、§4.2「Fanout」、§7「現行からの変更一覧」の「`worktree` field」「Artifact の有無」行を参照する。GitHub Milestone #85「01. Session delegate と Node worktree 隔離」の説明文と記述が食い違う場合は design.md に従う（milestone #85 説明文の指示）。
- 追加資料は `docs/glossary/WORKFLOW.md`（「Node の Interface と children の配線」「Sequence」「Fanout」「Command」「Session」「Contract / schemas」「予約語と未解禁 field」「Lua API」「Diagnostic」の各節）、`docs/glossary/DOMAIN.md`（用語表の「Worktree」「隔離 worktree」行、「Workspace と Worktree」「隔離 worktree」節）、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）、GitHub Issue #1467 / #1729 / #1730 / #1734、`docs/specs/issues-1729/`、`docs/specs/issues-1730/`、`AGENTS.md`、`docs/architecture/`、および現行の Rust 実装（`src-tauri/src/domain/workflow/value_objects/node_fact.rs`、同 `worktree_origin.rs`、`src-tauri/src/domain/workflow/services/worktree_reconciliation.rs`、同 `validation.rs`、`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs`、同 `worktree_ledger_repository.rs`、同 `workflow_host.rs`）と既存テストである。
- design.md §3.1 は次を定める。`worktree: shared | isolated` は Node 共通 field であり、省略時は `shared`（親から継承）である。「宣言した Node がその実行で隔離される」という一つの規則で全種別を扱い、Fanout に宣言すると Fanout の実行が1つの隔離 worktree を持ち children は全員そこで動く、Fanout の children に宣言すると children ごとに隔離 worktree ができる、Sequence に宣言すると Sequence の実行が隔離 worktree を持ち children は全員そこで動く、Session / Command に宣言するとその Node の実行が隔離 worktree を持つ。並列に動く children が同じ隔離 worktree で書き込めば衝突するが、読み取りだけなら衝突しないため engine は禁止せず、定義の書き方の問題として扱う。
- design.md §3.2 は次を定める。`isolated` の NodeExecution は attempt ごとに、親 worktree の HEAD から branch と worktree を生成し、そこを cwd として実行する。隔離 worktree の台帳と reconciliation は milestone #86 W7（#1467）で実装済みであり、事実は `IsolatedWorktreeCreated` / `IsolatedWorktreeReleased` / `IsolatedWorktreeLost`、台帳は事実ログから導出する `IsolatedWorktreeLedgerSnapshot`、起動時の突合は `worktree_reconciliation`（実体喪失 / 所有者終了済み / 台帳外）、命名は `isolated_worktree_branch` / `isolated_worktree_path` である。本 milestone で実装するのは `worktree` field の解禁、実行時の branch + worktree 生成と cwd 適用、および観測経路への露出である。このうち「台帳と reconciliation は実装済みであり本 milestone は生成側をつなぐ」という前提は、本 Issue の確定判断（#1467 の項に続く項）で置き換わる。
- design.md §3.3 は次を定める。engine は統合（merge）を一切行わない。隔離 worktree の成果は branch に残り、親 worktree には現れない。統合は判断主体（親 session の agent、または human）が diff を確認したうえで通常の Git 操作として行う。逐次 Node で `isolated` を使った場合、その diff は branch に残り後続 Node からは見えない。これは仕様である。
- design.md §4.3 は次を定める。`isolated` を宣言した Node の Artifact に engine が `worktree` キー（`{ "branch": ..., "path": ... }`）を足す。全 Node 種別で同じ形になる。`worktree` は予約キーであり、同名の field や child 名があれば load 時 Diagnostic になる。`artifact` を宣言しない Node でも `isolated` なら `worktree` キーだけを持つ Artifact が生まれ、Artifact の有無は「`artifact` 宣言があるか、`isolated` であるか」で決まる。これは Command の Artifact に engine が `ok` / `exit_code` / `stdout` / `stderr` / `duration` を合成し Contract に再宣言させないのと同じ仕組みである。
- #1467（PR #1656、commit 6e5c47022 で main に取り込み済み）の確定事項のうち、本 Issue でも維持するものは次のとおりである。worktree の出自は、人間が作る作業の場（root Node を植える先、長寿命）と、`isolated` 宣言で生まれる隔離実行環境（ephemeral、その実行が所有し Worktree 管理 UI の一覧に混ぜない）の2種に分かれる。worktree は Node が親から継承する実行コンテキストであり木の構造ではなく、isolated な子が別 worktree で実行されても木の所属は root の Worktree に固定される。engine は worktree 実体・branch への削除系操作を一切持たず、成果未統合の worktree を機械的に削除しない。隔離 worktree の branch は `releash/isolated/<node_execution_id>-a<attempt>`、path は repository root の兄弟ディレクトリ `<repository root の親>/<repository 名>-worktrees/.releash-isolated/<node_execution_id>-a<attempt>` であり、名前に所有 NodeExecution と attempt が埋め込まれている。
- 本 Issue の確定判断として、#1467 が実装した隔離 worktree の台帳とその実装・記載をすべて削除する。削除対象は、事実 `IsolatedWorktreeCreated` / `IsolatedWorktreeReleased` / `IsolatedWorktreeLost`、事実ログから導出する台帳 `IsolatedWorktreeLedgerSnapshot`、起動時の worktree reconciliation（実体喪失 / 所有者終了済み / 台帳外の突合）、Worktree 管理の `isolated_owned` / `cleanup_candidate` / `untracked_cleanup_candidate` の区分と「掃除候補」の提示、実体喪失の recovery reason と resume / Retry の事前拒否、およびこれらに関する `docs/glossary/DOMAIN.md` と `docs/specs/milestone-85/design.md` §3.2 の記載である。隔離 worktree の識別は、名前に埋め込まれた NodeExecution と attempt（命名規則）と実行木の状態だけを根拠にする。この判断は本 Issue の決定の範囲内である。
- Issue の「含むもの」の「`NodeFact::IsolatedWorktreeCreated` / `Released` / `Lost` の生成側をつなぐ」は、台帳を削除する前項の判断により対象外になる。
- Issue は「正本サンプルへの `isolated` 適用（`implement_and_verify` / `fix_and_verify`）」と「正本サンプルと builtin 8本が Diagnostic ゼロで load できる」を要求する。builtin 8本のどれにも `worktree` 宣言はなく、Issue は builtin への `isolated` 適用を挙げていない。
- 正本サンプルが `instruction` として参照する `merge_implementations` / `merge_fixes` の facet は、`workflows/facets/instructions/` にも `workflows/examples/` にも存在しない。facet 本文の更新は本 Issue の対象にならない。
- 依存する #1729（Sequence の Artifact を統合 map にする）は commit 2467b1d0f、#1730（Fanout の Artifact を map にする）は commit 52f5b862d で main に取り込み済みである。同 Wave の #1731（述語の Predicate 共通化）は commit 94963548d、#1732（completion の map 化）は commit a31937204 で main に取り込み済みである。Fanout の map 化の理由の一つは「`isolated` の Node にメタデータを足す場所が必要なこと」であり（#1730）、`worktree` キーの付与は #1730 の Non-goal として本 Issue に送られている。
- 後続の #1734（Session delegate）は本 Issue と独立に実装され、delegate の child に `isolated` を宣言した場合は本 Issue と同じ規則で効く（Issue #1733「含まないもの」）。
- workflow 定義は YAML と Lua の2つの表面を持ち、どちらも同じ `WorkflowDefinition` を構築し、同じ定義上の誤りには同じ `code`・`stage`・`message` の domain Diagnostic を使う。定義は未知 field、旧形式、互換 alias を受理せず、Error Diagnostic が一つでもある定義は実行できない（`docs/glossary/WORKFLOW.md`）。Issue と design.md は YAML の表記だけを示し、Lua 表面での `worktree` の受理形は確認した正本のどこにも定義されていない。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。frontend に許すのは表示とレイアウト制御、入力の受付、`invoke` の呼び出し、表示用フォーマットだけである。
- milestone #85 の全 ISSUE 共通の境界は次のとおりである。各 ISSUE は 1 PR で完結し、コード・Diagnostic・テストに加えて `docs/glossary/WORKFLOW.md` / `DOMAIN.md` の該当節、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）の更新を含む。各 ISSUE 完了時点で全定義が Diagnostic ゼロで load でき、テストが通る状態を保つ。
- 現在状態の確認は、Issue、milestone、design.md、用語集、現行コード、既存テスト、現行の workflow 定義の読解によって行った。build / test / lint などの検証コマンドは実行していない。
- `docs/specs/issues-1733` は本文書の作成時点まで未作成であり、本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash の workflow 定義を書く開発者、並列に走る実装 Node の成果を確認して統合する判断主体（親 Session の agent、または human）、および後続 #1734 で delegate の child を隔離する担当者である。

現在、実行木のすべての NodeExecution は、実行木が属する root worktree を cwd として実行される。Node ごとに別の worktree で実行する手段は定義構文になく、`worktree` を書いた定義は load 時に Error Diagnostic（`WFU002`）になって実行できない。このため、Fanout で並走する実装 Node が同じファイルを編集すると同じ worktree 上で衝突し、正本サンプルのコメントが述べる「タスクごとに隔離 worktree で並走させる」構造を実際には書けない。隔離 worktree の台帳・reconciliation・命名規則（#1467）は実装済みだが、隔離 worktree を生成する経路が無いため台帳に記録が入ることはない。

変更後、Node 共通 field `worktree: shared | isolated` を全4種の Node に宣言できる。`isolated` を宣言した Node の NodeExecution は attempt ごとに親 worktree の HEAD から branch した隔離 worktree で実行され、隔離 worktree は名前に埋め込まれた所有 NodeExecution と attempt だけで識別され、台帳は持たない。隔離 worktree の branch と path は Artifact の `worktree` キーとして engine が付与し、`artifact` を宣言しない Node でも `isolated` なら参照できる。UI・CLI・API から branch と path を観測でき、判断主体は各 branch の diff を確認して通常の Git 操作で統合する。engine は統合を行わない。

# Current Behavior

現行の `worktree` と実行環境に関する挙動は次のとおりである。

| 対象 | 現行の挙動 |
| --- | --- |
| YAML の `worktree` field | 全4種の Node で任意の文字列値を受理して保持し、load 時に `WFU002`（stage: `resolve`）の Error Diagnostic になる。値が `shared` / `isolated` / それ以外のいずれでも同じ。message は ``node '<node>' declares `worktree`, which is not supported yet (#85)`` |
| Lua の `worktree` field | `r.session{}` / `r.command{}` / `r.fanout{}` / `r.sequence{}` の table に `worktree` を書くと、未知 field として `WFS002` の Error Diagnostic（message は `unknown field 'worktree'`）になる |
| Node 名 `worktree` | 予約語として拒否される（`docs/glossary/WORKFLOW.md`「予約語と未解禁 field」） |
| NodeExecution の cwd | Session、Command とも、実行木が属する worktree（WorkflowExecution の `worktree_path`）を cwd として起動する。Node ごとに別の worktree を使う経路はない |
| Command の環境変数 | engine が `RELEASH_WORKFLOW_EXECUTION_ID`、`RELEASH_NODE_EXECUTION_ID`、`RELEASH_WORKTREE_PATH`（実行木の `worktree_path`）、`RELEASH_SESSION_ID` を渡す |
| Session の起動 | 実行木の `worktree_path` を起動 directory として provider CLI を起動し、`RELEASH_BASE_BRANCH` をその worktree の設定から解決して渡す |
| 隔離 worktree の事実 | `IsolatedWorktreeCreated` / `IsolatedWorktreeReleased` / `IsolatedWorktreeLost` が定義済み。`IsolatedWorktreeLost` は起動時 reconciliation が実体喪失を検出したときに追記する。`IsolatedWorktreeCreated` と `IsolatedWorktreeReleased` を追記する production 経路はなく、テストだけが追記する |
| 台帳と reconciliation | 事実ログから導出した台帳と `git worktree list` の inventory を突合し、`working_area` / `isolated_owned` / `cleanup_candidate` / `untracked_cleanup_candidate` に分類する。Worktree 管理 UI の通常一覧には `working_area` だけを表示し、掃除候補は別節に表示する。実体喪失の Node は `recovery_reason` を持ち、resume と retry を拒否される |
| 命名規則 | branch は `releash/isolated/<node_execution_id>-a<attempt>`、path は `<repository root の親>/<repository 名>-worktrees/.releash-isolated/<node_execution_id>-a<attempt>`。repository root の内側ではない |
| Artifact の有無 | Session は `artifact` 宣言がある場合だけ Artifact を持つ。Command は常に持つ。Sequence の統合 map には Artifact を産出しなかった child が現れず、Fanout の map では `artifact` を宣言しない child の slot が `null` になる |
| 参照解決 | `artifact` を宣言しない Session は配線 `inputs` / `when.on` / `switch.on` / `fanout.items` の供給元や段にできない |
| Artifact の `worktree` キー | 存在しない。Contract の `worktree` field を予約する検査もない |
| NodeExecution の read model | UI の Node 詳細、local API の execution 取得、CLI `releash workflow status` のいずれにも branch / path の項目はない。`recovery_reason`（実体喪失時）だけが隔離 worktree に由来する |

最小の再現手順と結果は次のとおりである（実装の読解による。コマンドは実行していない）。

1. `workflows/` に、Session Node `implement` へ `worktree: isolated` を書いた定義を置く。
2. `releash workflow diagnostics --dir <dir>` を実行する。
3. `WFU002` の Error Diagnostic ``node 'implement' declares `worktree`, which is not supported yet (#85)`` が出て、終了コードは 3 になる。定義は load されず実行できない。
4. 同じ Node を `worktree: shared` にしても結果は同じである。

現行の workflow 定義の実態は次のとおりである。

- `workflows/examples/full-cycle-development.yml` に `worktree` 宣言はない。`implement_all`（`items: create_detailed_design.tasks`）は Sequence `implement_and_verify`（children: `implement_task` → `verify_task`）を、`fix_all`（`items: create_fix_plan.tasks`）は Sequence `fix_and_verify`（children: `fix_task` → `verify_fix`）を item ごとに展開する。両 Fanout の直前のコメントは「タスクごとに『実装 → 検証』の一連の流れを隔離 worktree で並走させる。各子の diff は branch に残り、統合は merge_implementations が明示的に行う」「タスクごとに隔離 worktree で修正 + 検証 → 統合」と述べるが、実際は全 slot が同じ worktree で並走する。`implement_task` / `fix_task` は `artifact` を宣言しない Session、`verify_task` / `verify_fix` は `artifact` を宣言する Session である。`merge_implementations` は `results: implement_all`、`merge_fixes` は `results: fix_all` を型なし input で受ける。
- builtin 8本に `worktree` 宣言はない。`workflows/02_implement-existing-spec.yml` の `implement_fanout` は `items: create_detailed_design.tasks` で Session `implement_task` を展開し、全 slot が同じ worktree で並走する。
- `docs/glossary/WORKFLOW.md` は、Node 共通 field の表で `worktree` を「将来の隔離実行用の予約 field。現行 loader では `WFU002` Error」とし、「予約語と未解禁 field」節で「`shared` / `isolated` の実行は未解禁である。成功する定義には `worktree` を書かない」とする。「Lua API」の表に `worktree` はない。
- `docs/glossary/DOMAIN.md` の「隔離 worktree」節は、Node attempt が所有する状態（root Worktree と owner の identity、attempt ごとの branch / path identity、lifecycle fact、recovery fence）と、統合を engine が無条件に実行しないことを記述し、末尾で「定義上の `worktree` field は現時点では未解禁である」とする。
- 既存テストが、builtin 8本と正本サンプルが Diagnostic なしで load できることを確認している。

# Scope / Non-goals

## Scope

- Node 共通 field `worktree` の解禁。値域 `shared` / `isolated` の受理、省略時の扱い、値域外の値の Error Diagnostic。YAML 表面と Lua 表面の両方。
- `isolated` の NodeExecution が、attempt ごとに親 worktree の HEAD から branch と worktree を生成し、そこを cwd として実行すること。生成に失敗した attempt を Node failure として `on_failure` の対象にすること。Node の process から見える worktree の値（Command の `RELEASH_WORKTREE_PATH`、Session の起動 worktree と `RELEASH_BASE_BRANCH` の解決元）も隔離 worktree であること。4種の Node それぞれでの隔離の範囲。
- `isolated` な Node から `releash review` で Thread を扱うとき、その隔離 worktree を所有する実行木が属する Workspace の Thread が対象になること。
- 隔離 worktree の台帳とその実装・記載（事実3種、台帳、起動時 reconciliation、Worktree 管理の区分と掃除候補の提示、実体喪失の recovery reason と resume / Retry の事前拒否、関連文書の記載）の削除。
- 隔離 worktree が命名規則と実行木の状態だけで識別され、Worktree 管理の一覧に現れないこと。実体を失った隔離 worktree での attempt の再開が process の起動失敗として Node failure になること。
- 隔離 worktree の配置と命名が #1467 の規則に従い、repository root の内側に作られないこと。
- `isolated` の Node の Artifact に engine が `worktree` キー（`branch` / `path`）を足すこと。`artifact` を宣言しない Node でも `isolated` なら Artifact が生まれること。それに伴う Sequence の統合 map、Fanout の map、参照解決の変化。
- `worktree` を予約キーとして、`artifact` に参照した Contract の同名 field を、Node の `worktree` 宣言の有無に関わらず load 時 Diagnostic にすること。
- 隔離 worktree の branch / path を、attempt 開始時点から NodeExecution の観測経路（UI、local API の execution 取得、CLI `releash workflow status --json`）で観測できること、および Artifact 経由でも観測できること。
- engine が統合を行わないこと、および逐次 Node で `isolated` を使った場合の成果が後続 Node から見えないことの文書化。
- 正本サンプル `workflows/examples/full-cycle-development.yml` の `implement_and_verify` / `fix_and_verify` への `isolated` 適用と、正本サンプル・builtin 8本の Diagnostic ゼロでの load。
- `docs/glossary/WORKFLOW.md` の予約 field 節・Node 共通 field 表・Lua API 表・Artifact の有無と参照に関する記述、`docs/glossary/DOMAIN.md` の隔離 worktree 節、`docs/specs/milestone-85/design.md` §3.2 の更新。
- isolated Node の worktree 生成・cwd・観測経路、Fanout children 並走時の相互隔離、attempt 再実行での新規 worktree、`artifact` 宣言なしの isolated Node の Artifact、`worktree` 予約キーの衝突 Diagnostic、隔離 worktree の一覧非表示、実体喪失時の起動失敗を対象とするテスト。

## Non-goals

- engine による統合（merge）。統合は親 Session の agent または human が通常の Git 操作として行う。
- delegate の child の隔離（#1734）。#1734 で delegate が入れば本 Issue と同じ規則で効く。
- 削除する台帳・掃除候補の代わりとなる、隔離 worktree の残骸を Releash から一覧・削除する操作の新設。残骸は Node の観測経路（R-010）と Git から辿る。
- 隔離 worktree の branch / path の命名規則の変更（#1467 で確定済み）。
- 隔離 worktree と branch の削除、掃除、実体への変更操作。engine は削除系操作を持たない。
- 隔離 worktree を Workspace として開くこと、Worktree 管理 UI の通常一覧へ出すこと（#1467 で「一覧に混ぜない」と確定済み）。
- 実行木の所属の変更。isolated な Node が別 worktree で実行されても、木の所属は root の Worktree に固定される（#1467 明確化済み）。
- 合成子（Sequence / Fanout）への手動 Retry の追加、および隔離 worktree の生成失敗のための新しい中断理由や resume 経路の追加。
- 同一 worktree 上での書き込み競合の禁止。並列 children が同じ隔離 worktree で書き込む定義を engine は拒否しない。
- 正本サンプルが参照する instruction facet の本文更新。該当 facet はリポジトリに存在しない。
- builtin 8本への `isolated` 適用。builtin `workflows/02_implement-existing-spec.yml` の `implement_fanout` には統合 Node が無く、適用には統合 Session と instruction facet の新設を伴うため、本 Issue では適用しない。builtin は Diagnostic ゼロで load できることだけを確認する。
- `worktree` 以外の Node 共通 field、および Node の kind block の構文。
- frontend へのロジック追加。表示だけを変える。

# Requirements

- R-001: Node 共通 field `worktree` を全4種の Node に宣言でき、値は `shared` または `isolated` である。YAML では `worktree: shared` / `worktree: isolated` の文字列で、Lua では `worktree = r.worktree.shared` / `worktree = r.worktree.isolated` の handle で宣言し、どちらの表面でも Error Diagnostic なく load できる。省略時は `shared` として扱う。YAML で `shared` / `isolated` 以外の値を書いた定義、および Lua で `r.worktree.*` の handle 以外の値（文字列を含む）を書いた定義は load 時に Error Diagnostic になり、実行できない。
- R-002: `isolated` を宣言した Node の NodeExecution は、attempt ごとに、親 worktree の HEAD から新しい branch と worktree を生成し、その worktree を cwd として実行される。親 worktree は、その Node を子として扱う実行の worktree（外側に `isolated` の実行がなければ実行木が属する root worktree、あれば直近の外側の隔離 worktree）である。`shared`（省略を含む）の Node は親の worktree をそのまま引き継ぐ。`isolated` な Node の process から見える worktree の値はすべて隔離 worktree であり、Command の `RELEASH_WORKTREE_PATH` は隔離 worktree の path、Session は隔離 worktree を起動 worktree として起動され、`RELEASH_BASE_BRANCH` もそこから解決される。root worktree の path を process に渡す環境変数は増やさない。branch / worktree の生成に失敗した attempt は、Command の process 起動不能と同じくその Node の failure になり、children エントリの `on_failure` が適用される（`retry` は新しい attempt を新しい branch / worktree で始め、`ignore` は失敗を除外して続行し、宣言なしは中断して手動 Retry を待つ）。Sequence / Fanout に宣言した `isolated` の生成失敗も同じくその合成子の failure になる。これらの failure は Releash を再起動した後の復元でも failure のままであり、`worktree` を宣言しない Command の process 起動不能も同じく再起動後も failure として復元される。合成子には手動 Retry がないため、`ignore` 以外の復帰手段がないことは現行の制約として維持する。
- R-003: 隔離の範囲は「宣言した Node がその実行で隔離される」という一つの規則で決まる。Session / Command に宣言するとその Node の実行が隔離 worktree を持つ。Sequence / Fanout に宣言するとその合成子の実行が1つの隔離 worktree を持ち、children は全員そこで動く。Fanout の children エントリに宣言すると、展開された slot ごとに独立した隔離 worktree ができ、各 slot は互いの worktree 内の変更を見ない。
- R-004: 同じ Node の再実行で attempt が進むたびに新しい branch と worktree が生成され、前 attempt の worktree 内の中途状態を引き継がない。
- R-005: 隔離 worktree は、名前に埋め込まれた所有 NodeExecution と attempt（R-006 の命名規則）と実行木の状態だけで識別され、生成・解放・喪失の事実、台帳、起動時の突合は存在しない。命名規則に一致する worktree は、所有 Node の実行中・終了後を問わず Worktree 管理の一覧（作業の場の一覧、および掃除候補などの別節）に現れず、隔離環境喪失の recovery reason も存在しない。実体を失った隔離 worktree で attempt を再開しようとした場合は、process の起動失敗として R-002 の failure 規則に従い、手動 Retry は R-004 に従って新しい branch / worktree で新しい attempt を始める。
- R-006: 隔離 worktree の branch と path は #1467 で定義済みの命名規則（branch `releash/isolated/<node_execution_id>-a<attempt>`、path `<repository root の親>/<repository 名>-worktrees/.releash-isolated/<node_execution_id>-a<attempt>`）に従い、worktree は repository root の内側には作られない。この命名は隔離 worktree の識別と所有者の判定の唯一の根拠である。
- R-007: `isolated` を宣言した Node の Artifact には、engine が `worktree` キーを足す。値は `branch`（隔離 branch 名）と `path`（隔離 worktree の path）を持つ object であり、Session / Command / Sequence / Fanout の全種別で同じ形である。`artifact` を宣言しない Node でも `isolated` なら `worktree` キーだけを持つ Artifact が生まれる。したがって Node が Artifact を持つかどうかは「`artifact` 宣言があるか、`isolated` であるか」で決まり、`isolated` な child は Sequence の統合 map に現れ、Fanout の map で `null` にならない。
- R-008: `worktree` は Artifact の予約キーである。Node が `artifact` に参照した Contract の直下に `worktree` field がある定義は、その Node の `worktree` 宣言の有無（`shared` / `isolated` / 省略）に関わらず load 時に Error Diagnostic になり、実行できない。これは Command の予約 field `ok` / `exit_code` / `stdout` / `stderr` / `duration` の検査と同じ無条件の検査である。Node 名 `worktree` は引き続き予約語として拒否される。
- R-009: 配線 `inputs`、Command の `env`、テンプレート `{{ }}`、`fanout.items` の field path から、`isolated` な Node の Artifact の `worktree.branch` / `worktree.path` を宣言なしに解決でき、実行時に string 値が渡る。合成子の map を経由する参照（`<sequence>.<child>.worktree.branch`、`<fanout>.<キー>.worktree.path` など）も同じ規則で解決できる。`artifact` を宣言しない `isolated` な Session も、`worktree` キーを持つ Artifact を産出する Node として供給元と段になれる。
- R-010: `isolated` な NodeExecution の隔離 branch と path を、attempt 開始時点から、UI、local API の execution 取得、CLI の `releash workflow status --json` のそれぞれで参照できる。値は attempt ごとの隔離 worktree の branch 名と path であり、Artifact 産出前の実行中、Artifact を産出して完了した後、Artifact を産出せずに失敗または abort で終わった後のいずれでも参照できる。これは Session / Command / Sequence / Fanout の全種別に当てはまる。UI では Session / Command の Node 詳細で参照でき、合成子（Sequence / Fanout）の attempt についても、children の有無に関わらず、その合成子自身の branch / path を参照できる。加えて、Node が Artifact を産出した後は、その Artifact を取得できる各経路（local API の execution 取得と Artifact 取得、CLI の `releash workflow status --json` と `releash workflow output get`）で Artifact の `worktree.branch` / `worktree.path` を読める。UI は Artifact の値を表示する経路を持たず、UI での Artifact の `worktree` の参照は、同じ値を示す UI 上の Node の branch / path の表示が担う。Artifact の値を UI に表示する経路は新設しない。
- R-011: engine は隔離 worktree の成果を親 worktree へ統合しない。隔離 worktree で行った変更は隔離 branch に残り、親 worktree と、親 worktree で実行される後続 Node からは見えない。統合は判断主体が通常の Git 操作として行う。
- R-012: `workflows/examples/full-cycle-development.yml` の `implement_and_verify` と `fix_and_verify` は `worktree: isolated` を宣言し、`implement_all` / `fix_all` の各 slot が独立した隔離 worktree で並走する。正本サンプルと `workflows/*.yml`（builtin 8本）は Diagnostic ゼロで load できる。
- R-013: `docs/glossary/WORKFLOW.md` は、`worktree` を解禁済みの Node 共通 field として値域・省略時の意味・種別ごとの隔離の範囲・Artifact の `worktree` キー・予約キーの検査・Artifact の有無の規則を記述し、「Lua API」の表に `r.worktree.shared` / `r.worktree.isolated` の handle と各 Node builder の `worktree?` field を記述し、「未解禁」「`WFU002`」「成功する定義には `worktree` を書かない」の記述を残さない。`docs/glossary/DOMAIN.md` の隔離 worktree 節は、生成の規則（attempt ごと、親 worktree の HEAD から）、命名が識別の根拠であること、統合を engine が行わないこと、逐次 Node での成果が後続 Node から見えないことを記述し、「定義上の `worktree` field は現時点では未解禁である」の記述と、台帳・lifecycle fact・recovery fence・掃除候補の記述を残さない。`docs/specs/milestone-85/design.md` §3.2 は、台帳・reconciliation・事実3種を実装済みの前提として挙げる記述を残さず、命名規則と本 Issue の生成・観測の規則だけを記述する。
- R-014: `isolated` な Node から `releash review` で Thread を扱うとき（Command からの `releash review list`、Session からの `releash review list` / `get` の `--session-id` 経由とも）、その隔離 worktree を所有する実行木が属する Workspace の Thread が対象になる。隔離 worktree の path を Workspace として新しい Thread 集合を作らない。隔離 worktree がどの実行木のどの NodeExecution に属するかは命名規則（R-006）に埋め込まれた NodeExecution と attempt から判断でき、Workspace への結び付けは読み側がそこから導く。

# Assumptions / Open Questions

Assumption はない。

Open Question はない。
