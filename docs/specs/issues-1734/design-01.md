# Design 01

## 開始状態

初回。base は `main`、派生点は `c4a9f36c`（#1733 取り込み後）。branch `feat/issues/1734` は派生点と同一 commit で、未コミットの変更は `docs/specs/issues-1734/` の新規文書だけである。差分の基準は `docs/specs/issues-1734/requirements.md` の Current Behavior 節が記録する実装状態で、直前の Design はなく、open Thread もない。

Current Behavior が記録する挙動の現行の所在は次のとおりである。

- `completion` の受理は `src-tauri/src/adaptor/gateway/workflow/completion_wire.rs` が YAML / Lua 共通の shape 検査として持ち、`require` 以外のキーを `WFS002`（`completion map only accepts the key 'require'`）にする。Lua は `adaptor/gateway/workflow/lua/mod.rs` の `parse_completion` が同じ検査に委ね、`lua/stubs.rs` の `ReleashCompletion` は `require` だけを注釈する。domain の `NodeCompletion` は `domain/workflow/value_objects/definition.rs` にあり、`require_approval` だけを表す。
- Session の完了信号は `domain/workflow/entities/workflow_execution/mod.rs` の `record_completion_signal` が `Pending → SubmitReceived / StopReceived → Ready` の一方向で持つ。同一 attempt で二度目の Submit / Stop は `AlreadyApplied` として捨てられ、`Ready` の後は `completion` の `require: approval` の有無だけで完了か WaitingApproval かが決まる。
- Artifact の予約キー検査（`worktree`、Command の `ok` / `exit_code` / `stdout` / `stderr` / `duration`）は `domain/workflow/services/validation.rs` の `ReservedArtifactField` にあり、参照の段解決は同 `reference.rs` が Node の Artifact Contract（`isolated` なら `worktree` 合成後）を起点に行う。述語は `domain/workflow/value_objects/predicate.rs` が持ち、辺の `when.on` だけが使う。
- 実行木の親参照は `domain/workflow/value_objects/node_execution.rs` の `ExecutionParentRef`（`parent_id` と Fanout 限定の `fanout_slot`）であり、`sequence_child` / `fanout_child` の2種だけを構築できる。doc は「親（合成子インスタンス）」と記す。NodeExecution の開始は `workflow_execution/mod.rs` の `start_node_instance` / `start_fanout_child_instance` で、いずれも新しい node_execution_id と attempt を採番する。
- 到達可能性と child の共有・包含 cycle の検査は `services/validation.rs`（`UnreachableNode`、`ChildReferenceViolation` → `WFC006`、`WFC007`、`WFC005`）にあり、`adaptor/gateway/workflow/diagnostics.rs` が code に写す。
- Session の起動と初期指示の投入は `adaptor/gateway/workflow/workflow_host.rs` が `domain/workflow/services/prompt_composition.rs` の `provider_tui_initial_instruction` で組んだ文面を `usecase/agent_session/agent_session_initial_instruction.rs` の `AgentSessionInitialInstructionUsecase`（`ProviderAgentTerminalInputGateway` 経由）で送る。resume は `workflow_host/lifecycle_commands.rs` の `resume_workflow_execution` が provider CLI を `--resume` で再起動して初期指示を再投入する。稼働中の provider session へ engine が後から指示を送る経路は、起動時の初期指示以外に存在しない。
- NodeExecution の事実は `domain/workflow/value_objects/node_fact.rs` にあり、child の結果の注入を表す事実はない。実行木の attempt 表示は `domain/workspace_tree/projection.rs` が node 名と親参照の一致で再試行対象をまとめる。
- 正本サンプル `workflows/examples/full-cycle-development.yml` は `implement_all`（Fanout）が `worktree: isolated` の Sequence `implement_and_verify`（`implement_task` → `verify_task`）を item ごとに展開する。`implement_task` は `artifact` を持たず、`verify_task` は `implement-task-check-result`（required boolean `complete` を持つ）を `artifact` にする。`docs/glossary/WORKFLOW.md` の「completion」節は「`require` 以外のキー（`delegate` を含む）は Error Diagnostic になる」と記し、「Lua API」の表は `completion?: { require = r.completion.approval }` だけを示す。

## 変える部分

- `delegate` の受理: Session の `completion` map に `delegate`（`child` 必須、`inputs` 任意、`when` 必須、`max_iterations` 必須で1以上の整数）を受理し、`require: approval` との併記も受理する。`delegate` map の未知キー、必須 field の欠落、`max_iterations` の値域外・非整数を load 時 Error Diagnostic にする。根拠: R-001 / B-001、R-002 / B-002 / B-030。ルート: 委任。
- `delegate` を宣言できる Node の制限: `artifact` を宣言した Session だけが `delegate` を宣言でき、`artifact` を宣言しない Session（`isolated` の有無を問わない）と Command / Fanout / Sequence での宣言を load 時 Error Diagnostic にする。根拠: R-002 / B-003 / B-027。ルート: 委任。
- child の条件の検査: `child` が存在する Node であること、Artifact を持つ Node（Command / Fanout / Sequence、`artifact` 宣言または `isolated` の Session）であること、親 Session 自身や親 Session を部分木に含む Node（`main` を含む）でないこと、合成子の child や別の delegate の child と共有されていないことを load 時に検査し、違反を Error Diagnostic にする。delegate の child としてだけ参照される Node を到達可能とみなす。根拠: R-001 / B-029、R-002 / B-002 / B-028 / B-029。ルート: 委任。
- 親 Artifact への `child` キーの合成と予約: delegate を宣言した Session の Artifact に engine が `child` キーを足す（提出直後 `null`、child 完了後は child Node の Artifact そのもの、最後の結果で上書き）。delegate を宣言した Session の Contract の直下に `child` field がある定義を load 時 Error Diagnostic にし、delegate を宣言しない Node の Contract は検査しない。根拠: R-008 / B-011 / B-012、R-009 / B-013。ルート: 委任。
- 合成 schema による静的な型検査: 親 Contract の `properties` に `child` → child Node の Artifact schema を足した合成 schema を load 時に作り、delegate の `when`、下流の配線 `inputs`（`<親Session>.child.<...>`）、親 Session を自 Node とする辺の `when.on` / `switch.on`（`child.<...>`）の各段を既存の検査規則で解決する。child が Sequence / Fanout の場合はそれぞれの map の規則で段を辿る。根拠: R-006 / B-008、R-010 / B-012 / B-014。ルート: 委任。
- `when` の受理と評価: delegate の `when` を #1731 の述語と同じ構造で受理し、親の提出時と child の完了時に評価する。参照先が未確定または boolean でない参照は false とし、親の提出時の `child.` 参照は false になる。根拠: R-006 / B-008 / B-009。ルート: 委任。
- `inputs` の配線: `<パラメータ名>: <供給元>` を受理し、配線先が child の宣言した input パラメータであること、供給元が親の `input` パラメータ・親 Session の Node 名で参照する親の Artifact（`<親Session名>.child.<field>` を含む）・`request` であることを既存の配線規則で検査する。親 Session の Node 名と input パラメータ名の衝突は既存の曖昧拒否と同じ扱いにし、無名インライン Session は自身の Artifact を供給元にできない。実行時は child の各起動でその時点の値を解決する。根拠: R-011 / B-015 / B-016。ルート: 委任。
- 発火と進行: 親 Session の Artifact 提出を発火点とし、提出時に `when` が真なら child を起動せず完了、偽なら child を起動して親を完了させずに待つ。child 完了時に `when` が真なら親を完了、偽なら child の Artifact を `child` キーとして注入し、同じ NodeExecution・attempt・AgentSession のまま親を続行させて再提出できるようにする。専用の typed command は設けない。根拠: R-003 / B-004 / B-005、R-004、R-005 / B-006 / B-007、R-012 / B-007。ルート: 委任（同一 attempt 内で提出と続行を繰り返せるようにする完了信号の扱い、稼働中の provider session への注入手段を含む）。
- `max_iterations` の上限: child を上限回数まで起動し、上限回数の child がすべて完了した後の提出では `when` を評価せず child を起こさず完了する。遷移先は持たない。根拠: R-007 / B-010。ルート: 委任。
- `require: approval` との併記: 述語成立または上限到達で完了する条件を満たした後に WaitingApproval とし、Approve で完了する。条件を満たす前は Approve の対象にしない。根拠: R-015 / B-021。ルート: 委任。
- 実行木での child の表現: child の NodeExecution が親 Session の NodeExecution を親に持つ部分木として載り、発火ごとに新しい NodeExecution と attempt を持つようにする。UI・local API の execution 取得・CLI `releash workflow status --json` で親 Session の下に発火ごとの行として並ぶ。根拠: R-012 / B-017。ルート: 委任（`ExecutionParentRef` への delegate child の表現を含む）。
- 注入の事実化と resume: child の結果の親 Session への注入を事実として記録し、child 完了後・注入前の中断からの resume で child を再実行せず注入から再開し、注入済みなら二重に注入しない。child 自身の resume は Fanout の子と同じ扱いにする。provider session を復元できない場合は既存の失敗経路（`on_failure` / 手動 Retry）に委ねる。根拠: R-013 / B-018 / B-019、R-014 / B-020。ルート: 委任。
- child の worktree: delegate の child の worktree を #1733 の規則に乗せる。`isolated` なら各起動が親 Session の実行 worktree の HEAD から生成された隔離 worktree で実行され Artifact に `worktree` キーが合成される。`shared`（省略を含む）なら親 Session の実行 worktree（親が `isolated` ならその隔離 worktree）を引き継ぐ。根拠: R-016 / B-022 / B-031。ルート: 委任。
- 正本サンプルへの delegate 適用: `workflows/examples/full-cycle-development.yml` の `implement_task` に `worktree: isolated` と Object Contract の `artifact` を宣言し、`completion.delegate` に `child: verify_task`、`inputs` の `task: task` と `spec: spec`、`when: child.complete`、`max_iterations` を書く。`implement_all` の children は `implement_task` を item ごとに直接展開し、Sequence `implement_and_verify` を削除する。`fix_and_verify` と他のループは変更しない。正本サンプルと builtin 8本が Diagnostic ゼロで load できることを保つ。根拠: R-017 / B-023。ルート: 委任（`implement_task` の Contract の field 構成、`max_iterations` の値、文面）。
- `docs/glossary/WORKFLOW.md`: 「Session」「completion」の各節に `delegate` の受理形、宣言できる Session の条件、child の条件、`max_iterations` の値域、`inputs` の供給元、発火、評価の時点と規則、上限の意味、`require: approval` との併記の意味、`child` キーの形と kind ごとの参照形、child の worktree の規則を記述し、「`require` 以外のキー（`delegate` を含む）は Error Diagnostic になる」を残さない。「予約語」「Contract / schemas」の各節に `child` を delegate 親の Artifact の予約キーとして記述する。「Lua」節と「Lua API」の表に、Lua の `completion` table が `require` だけを受理し `delegate` を受理しないことを記述する。根拠: R-019 / B-025。ルート: 委任（文面）。
- `docs/glossary/DOMAIN.md`: `completion` の要求に `delegate` が含まれること、delegate が Session の所有する同一 session 継続機構であること、child の NodeExecution が親 Session の部分木であることを記述する。根拠: R-019 / B-026。ルート: 委任（文面）。

## 固定するルート

固定する実装上の指定なし。

## 変えないもの

- Lua 表面の `completion` table。`require` だけを受理し、`delegate` を含む table は現行と同じ Error Diagnostic のままにし、生成 stub の型注釈も変えない。理由: Q-002 の自動判断（Lua での受理形と親 Session 自身の Artifact field を指す参照手段が正本に無い）。
- #1731 の述語の構造と評価規則、#1732 の `completion` map と `require: approval` の受理形と承認の実行時経路、#1733 の Node worktree 隔離の規則。理由: 各 Issue で確定した規則を delegate はそのまま使う（Non-goals）。
- Node が一つの親だけを持つ規則（`WFC006` / `WFC007`）と静的な包含 cycle の拒否。理由: Q-008 の自動判断。delegate の child もこの規則の下に置く。
- builtin 8本の定義本文。理由: Issue は正本サンプルへの適用だけを求める（Non-goals）。Diagnostic ゼロで load できることの確認だけを行う。
- Fanout の子と異なる delegate child 固有の resume・Retry 経路と、provider session を復元できない場合の新しい復旧経路。理由: design.md §2.9 と Non-goals。

## 未確定・リスク

- 自動判断（人間の確認を経ていない仮定）は Requirements の Assumptions が列挙する8件である。正本サンプルの適用先を `implement_task`（child `verify_task`）の1箇所に限定すること（Q-001）、Lua 表面の受理を未決として Non-goals に置き R-018 / B-024 を欠番にしたこと（Q-002）、`inputs` で親の Artifact を参照する名前を親 Session の Node 名にすること（Q-003）、`artifact` を宣言した Session だけが `delegate` を宣言できること（Q-004）、`child` 予約キーの検査を delegate 親の Contract に限ること（Q-005）、`max_iterations` を1以上とすること（Q-006）、child を Artifact を持つ Node に限ること（Q-007）、child の共有禁止・自己包含禁止・delegate だけからの参照を到達可能とみなすこと（Q-008）。人間が異なる判断をした場合、R-001 / R-002 / R-009 / R-011 / R-017 / R-019 と対応する Behavior が変わる。
- `[DEFERRED]` で人間へ渡した件: Q-002（Lua 表面での `completion.delegate` の受理形。親 Session 自身の Artifact field を指す Lua の参照手段を含む）。決まった時点で別途要求に戻す。
- 同一 attempt 内の完了信号。現行の `record_completion_signal` は attempt ごとに `Pending → SubmitReceived / StopReceived → Ready` を一度だけ辿り、二度目の Submit / Stop を捨てる。R-005 / B-007 は同じ NodeExecution・同じ attempt で提出と続行を複数回繰り返すことを求めるため、完了信号の扱いを attempt 単位からラウンド単位へ変える必要がある。R-015 / B-021 の「条件を満たす前は Approve の対象にしない」もこの扱いに依存する。想定が外れると B-007 / B-010 / B-021 を満たせない。
- 稼働中の provider session への注入手段。現行で engine が provider session へ文面を送る経路は起動時の初期指示（`AgentSessionInitialInstructionUsecase`）だけであり、Stop 後に `AwaitingInstruction` にある provider session へ後から指示を送って続行させることは未検証である。claude / codex の TUI が Stop 後の追加入力を受け付けなければ、B-007 / B-018 の「親 Session は続行する」を満たせない。
- Submit の「その turn を終了する」指示との整合。現行の起動プロンプト末尾は、提出が成功したら追加の tool 実行を行わず turn を終了するよう agent に指示する。delegate 親では提出後に child の結果を受けて再開するため、初期指示の文面（`prompt_composition.rs`）が delegate 親で再提出の可能性を伝えなければ、agent が続行を拒む可能性がある。想定が外れると B-007 を満たせない。
- child が失敗した場合の親の扱い。Requirements は child の完了時の評価だけを定め、child の NodeExecution が失敗（Failed）で終わった場合に親 Session をどう扱うかを定めていない。実装が既存の失敗伝播（合成子の child の failure と同じ扱い）に乗せる想定だが、`on_failure` は children エントリが持つため delegate の child には宣言箇所がなく、想定が外れると B-020 の「既存の失敗経路で扱われる」を満たせない。
