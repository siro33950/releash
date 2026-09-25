# Design 01

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `9194bd57`（`refactor(state): Workspaces の表示を全て状態購読に移す (#1885) (#1924)`）。worktree `feat/issues/1890` に未コミットの実装変更は無く、`docs/specs/issues-1890/` の文書だけが未追跡である。この commit の実装を今周の開始状態とする。
- 既存の `design-NN.md` は無い。初回であり、開始状態の挙動は `docs/specs/issues-1890/requirements.md` の Current Behavior を参照する。
- この周までに解消・見送りとなった Thread は無い。open Thread も無い。

## 変える部分

- 待ち時間の計算の新設: `min(初回 × 倍率^(n-1), 上限)` を 1 つの実装にし、初回・倍率・上限を処理の種類ごとに指定できる形にする。ばらつき（0.8〜1.2 倍）を掛ける。根拠: R-002「やり直しの待ち時間は、どの処理でも同じ式で決まる」、B-002。ルート: `src-tauri/src/domain/` 直下の値オブジェクトが式と初回・倍率・上限を持ち、ばらつきの係数は domain に持たせず呼び出し側が引数で渡す。置き換える既存の 3 表現は `domain/workflow/value_objects/node_execution.rs:3-5` の `startup_restart_delay`、`gateway/provider_lifecycle/event_repository_impl.rs:175-181` の待ち、`infrastructure/terminal/checkpoint_scheduler.rs` と `gateway/terminal_surface/runtime_gateway_impl.rs:52` の固定間隔。値は「固定するルート」の 4 組を使う。

- やり直しの頻度の上限の新設: daemon 全体のやり直しが単位時間あたりの回数の上限を超えない形にする。根拠: R-003「多数の対象が同時に失敗しても、daemon が行うやり直しは単位時間あたりの回数の上限を超えない」、B-003。ルート: `src-tauri/src/domain/` 直下の値オブジェクトが、現在時刻とトークン量から「今出してよいか・待つ長さ・次のトークン量」を返す純粋な計算として所有する。実際の待ちは domain の外に置く。外部 crate は追加しない。値は 10 回/秒・一度に 100。

- 作業列の新設: 失敗した対象ごとに間隔を延ばしてやり直し、回数で打ち切らない作業列を置く。根拠: R-001「失敗した対象は、やり直しの間隔を延ばしながらやり直される」「やり直しの回数に上限はなく、間隔が上限に達した後も、その間隔でやり直され続ける」、B-001。ルート: Kubernetes の controller の作業列（client-go の workqueue）に従う。回数による打ち切りを残さない。構造の詳細（対象の識別、重複した積み込みの畳み方、同時に処理するタスクの本数）は委任。

- やり直しの判断の新設: やり直さない／同じ段階でやり直す／上の段階からやり直す の 3 値を失敗の分類から導く規則を置く。`Temporary` は同じ段階、`RestartRequired` は上の段階、残る 14 変種はやり直さない。根拠: R-004「やり直すかどうかは、その失敗の分類だけで決まる」、R-005、B-004、B-005、B-006、B-007。ルート: `src-tauri/src/domain/failure.rs` の値オブジェクトが `FailureKind` から導く。

- 既存のやり直しの 5 箇所の置き換え: 待ち時間の計算と、分類による判断の呼び出しへ置き換える。回数による打ち切りを残さない。根拠: R-002、R-004、R-005、B-004〜B-007。ルート: 対象は `usecase/workflow/command/mod.rs:19-48` の `CONTROL_PLANE_MAX_ATTEMPTS` と `retry_control_plane_conflicts` / `retry_control_plane_operation`、`gateway/workflow/workflow_host.rs:255-263` の `retry_runtime_conflicts`、同 `2045-2060` の `settle_runtime_failure_for_node`、`gateway/provider_lifecycle/event_repository_impl.rs:171-187` の `resolve_bounded`、`usecase/workflow/node_startup.rs:22-61` の `retry_failed_nodes`。`command/mod.rs:19` の `CONTROL_PLANE_MAX_ATTEMPTS = 4`、`node_execution.rs:3-5` の 4 回、`event_repository_impl.rs:176` の 4 回は、いずれも回数による打ち切りを残さない形へ置き換える。

- 失敗の記録の新設: 処理の種類・対象・分類が同じ失敗を 1 件にまとめ、発生回数と最初・最後の時刻を持つ記録を置く。保持には件数の上限があり、daemon を起動し直すと引き継がれない。根拠: R-007「同じ失敗は 1 件の記録にまとまり、その記録は発生回数と最初・最後の時刻を持つ」、B-009、B-015。ルート: `src-tauri/src/domain/` の集約が記録の集まりを所有し、観測した失敗の受理、同じ失敗かの判定、発生回数と最初・最後の時刻の更新、保持の上限と追い出しを、この集約の操作としてのみ行う。同じ失敗かの判定に失敗の文面を使わない。保持の件数の上限の具体値は委任。

- 失敗の記録の観測の経路の新設: 記録を daemon が保持する状態として画面から購読できるようにする。根拠: R-007「この記録は daemon が保持する状態であり、利用者が観測できる」、B-009。ルート: 委任。

- 要対応の規則と状態の新設: やり直さない分類のうち `Cancelled` を除く 13 分類で失敗した対象を、その対象の「要対応」として持つ。根拠: R-006「やり直さない分類のうち、利用者またはシステムが意図して止めたことを表す分類（`CANCELLED` に相当）を除く分類で失敗した対象は、その対象が「要対応」であることを、その対象の表示から利用者が観測できる」、B-008、B-014。ルート: 「どの分類が要対応か」は `src-tauri/src/domain/failure.rs` が横断の規則として持つ。「対象ごとの要対応の状態」は各ドメインが所有し、`domain/workspace_tree` は既存の `WorkspaceNodeStatusClassification::Attention` と `classify_own_status`（`value_objects/mod.rs:53-86, 158-190`）を使い、`domain/repository`・`domain/terminal_surface`・`domain/agent_session` には状態を新設する。

- 要対応を画面へ出す経路の新設: 実行木・Node、terminal、agent session、repository の要対応を、その対象の既存の表示に出す。根拠: R-006、B-008。ルート: proto（`proto/client.proto`）の項目名と形、および画面（TypeScript）側の表示の仕方は委任。

- 起動時の再開処理の作業列への移行: `usecase/workflow/startup.rs:29-72` の `execute()` と `adaptor/controller/daemon.rs:356-361` の 1 度きりの `tokio::spawn` を、実行木ごとにやり直しの対象とする形へ移す。実行木の一覧の読み込み（`list_tree_ids()`）も同じくやり直しの対象にする。根拠: R-008「起動時の再開処理は、実行木ごとにやり直しの対象になる。実行木の一覧の読み込みも、同じくやり直しの対象になる」、B-016。ルート: 一覧の読み込みも作業列に載せる。`startup.rs:29-33` の `recovery_lock` を作業列へ移すときの具体的な形は委任。

- 再開の失敗による abort の廃止: `usecase/workflow/startup.rs:52-69` の `abort_startup_failure` による abort を、失敗の分類によらず行わない形にする。根拠: R-008「再開の失敗によって実行木が abort されることはない」、B-010。ルート: 分類によらず abort を行わない。

- 再開処理の経路で分類を潰している変換の是正: `Conflict` 以外を一律に `Internal` 相当へ落とす変換を、分類を保つ形にする。根拠: R-009「起動時の再開処理の失敗は、失敗が起きた場所の理由に対応した分類で呼び出し元に届く」、B-011。ルート: 対象は `gateway/workflow/startup_repository.rs:28-38` の `WorkflowError::external` への変換と、`gateway/workflow/workflow_host.rs:655-662` の `WorkflowRuntimeError::SessionStore` への変換。具体的な方法は委任。

- 繰り返す 4 処理の作業列への移行: terminal の保存（`infrastructure/terminal/checkpoint_scheduler.rs:20-70`）、provider session のタイトルの取得（`adaptor/controller/daemon.rs:159-166`、`usecase/agent_session/provider_session_title_ingestion.rs:38-100`）、review comment の監視（`infrastructure/comment/watcher.rs:102-131`）、repository の走査（`usecase/repository_state/worker.rs:60-133`）を作業列に載せる。監視を始められなかったときに `return` して以後動かない経路（`watcher.rs:95-107`）も、やり直しの対象にする。根拠: R-001、R-010「繰り返す処理を始められなかったときも、その失敗は R-005 の判断の対象になり、やり直す分類であればその処理の開始がやり直される」、B-001、B-012。ルート: 既存の周期の駆動（`watcher.rs` の `tokio::time::interval`、`checkpoint_scheduler.rs` の `DirtyCheckpointScheduler` のスレッドと dirty フラグ、`repository_state` の `RepositoryStateInvalidationReceiver`）の扱いは委任。

- 繰り返す処理の 1 回の試行の期限の新設: 終わらない試行が、同じ対象の以後の処理と他の対象の処理を止め続けない形にする。根拠: R-011「繰り返す処理の 1 回の試行には期限がある」、B-013。ルート: 期限は 20 秒。

## 固定するルート

- 規則の所有（やり直しの判断）: やり直さない／同じ段階でやり直す／上の段階からやり直す の 3 値を、`src-tauri/src/domain/failure.rs` の値オブジェクトが `FailureKind` から導く。範囲は `usecase/workflow/command/mod.rs:19-48` の `retry_control_plane_*`、`gateway/workflow/workflow_host.rs:255-263` の `retry_runtime_conflicts`、同 `2045-2056`、`gateway/provider_lifecycle/event_repository_impl.rs:171-186` の `resolve_bounded`、`usecase/workflow/node_startup.rs:22-61` の 5 箇所を、この規則の呼び出しへ置き換えること。粒度は所有者と置き換え対象の指定まで。理由は、規則の入力が `FailureKind` だけであり、`FailureKind` が既にドメイン横断の位置にあるため。

- 規則の所有（待ち時間の計算）: `min(初回 × 倍率^(n-1), 上限)` を `src-tauri/src/domain/` 直下の値オブジェクトが持ち、初回・倍率・上限を保持する。ばらつきの係数（0.8〜1.2）は domain に持たせず、呼び出し側が引数で渡す。範囲は `domain/workflow/value_objects/node_execution.rs:3-5` の `startup_restart_delay`、`gateway/provider_lifecycle/event_repository_impl.rs:173-180` の待ち、`infrastructure/terminal/checkpoint_scheduler.rs` と `gateway/terminal_surface/runtime_gateway_impl.rs:52` の固定間隔の 3 表現を置き換えること。粒度は所有者と、乱数を domain の外に置くことの指定まで。理由は `docs/architecture/DOMAIN.md` の「テスト容易性: 純粋関数として書ける範囲で書く」。

- 規則の所有（やり直しの頻度の上限）: `src-tauri/src/domain/` 直下の値オブジェクトが、現在時刻とトークン量から「今出してよいか・待つ長さ・次のトークン量」を返す純粋な計算として所有する。実際の待ちは domain の外に置く。外部 crate は追加しない。粒度は所有者と、待ちを外に置くこと、crate を足さないことの指定まで。理由は `docs/architecture/DOMAIN.md` が「閾値」を domain の規則として名指ししているため。

- 規則の所有（失敗の記録）: `src-tauri/src/domain/` の集約が、失敗の記録の集まりを所有する。観測した失敗の受理、同じ失敗かの判定、発生回数と最初・最後の時刻の更新、保持の上限と追い出しを、この集約の操作としてのみ行う。粒度は所有者と、受理と記録を一つの操作にすることの指定まで。理由は、状態とライフサイクルを持つ概念であり、`docs/architecture/DOMAIN.md` の「観測した事実を状態へ反映する遷移は、事実の受理と記録を一つの操作にする」に該当するため。

- 規則の所有（要対応）: 「どの分類が要対応か」は `src-tauri/src/domain/failure.rs` が横断の規則として持つ。「対象ごとの要対応の状態」は各ドメインが所有する。範囲は、`domain/workspace_tree` は既存の `WorkspaceNodeStatusClassification::Attention` と `classify_own_status`（`value_objects/mod.rs:53-86, 155-186`）を使い、`domain/repository`・`domain/terminal_surface`・`domain/agent_session` には状態を新設すること。粒度は横断の規則と状態の所有者の分担まで。理由は `docs/architecture/DOMAIN.md` の「一つの概念に一つの表現」と「別ドメインの関心を別ドメインの型に常設してはならない」。

- 規則の所有（repository の走査）: repository の要対応の状態は `src-tauri/src/domain/repository/` が所有する。既存の走査の進行の状態（`refreshing`・`loading`・requested/applied generation・`scan_lock`）は `usecase/repository_state/worktree.rs` の `WorktreeState` に残し、移設しない。粒度は新設する状態の所有者と、既存の状態を動かさないことの指定まで。理由は #1890 の要求が走査の進行に触れないため。

- 待ち時間の値（処理の性質ごとに 4 組）。(1) 対象ごとのやり直し（terminal の保存、repository の走査、provider session のタイトルの取得、review comment の監視、起動のやり直し）は初回 5 ms / 倍率 2 / 上限 1000 秒。根拠は client-go の既定で、正本が明記。(2) 実行木の一覧の読み込みと起動時の再開は初回 800 ms / 倍率 2 / 上限 30 秒。根拠は client-go Reflector。(3) 競合のやり直し（`RestartRequired`）は初回 10 ms / 倍率 5 / 上限 1 秒。根拠は client-go `retry.DefaultBackoff`。(4) 外部サービスの呼び出し（`resolve_bounded` など）は初回 1 秒 / 倍率 1.6 / 上限 120 秒。根拠は gRPC connection backoff。粒度は値の指定まで。理由は、正本が「値は処理の種類ごとに指定する」と書き、標準も用途ごとに別の既定値を持つため。

- 共通の値: ばらつきは 0.8〜1.2 倍（gRPC の JITTER 0.2、正本が明記）。全体の頻度の上限は 10 回/秒・一度に 100（client-go の既定、正本が明記）。繰り返す処理の 1 回の試行の期限は 20 秒（gRPC の MIN_CONNECT_TIMEOUT）。粒度は値の指定まで。理由は、前 2 つは正本の明記、期限は正本が「この ISSUE で決める」とした未決であるため。

- 作業列は Kubernetes の controller の作業列（client-go の workqueue）に従う。待ち時間の計算は 1 つの実装にし、この ISSUE の全ての処理と #1879 が使う。粒度は従う標準と、実装を 1 つにすることの指定まで。構造の詳細は委任。理由は正本の指定。

- やり直しの回数に上限を設けない。`usecase/workflow/command/mod.rs:19` の `CONTROL_PLANE_MAX_ATTEMPTS = 4`、`domain/workflow/value_objects/node_execution.rs:3-5` の 4 回、`event_repository_impl.rs:174` の 4 回は、いずれも回数による打ち切りを残さない形へ置き換える。粒度は上限を置かないことの指定まで。理由は、上限を持つ標準（Kubernetes controller の `maxRetries`）は informer の resync が積み直す前提に立ち、Releash にその前提が無いため。

- 起動時の再開処理は、実行木の一覧の読み込みも含めて作業列に載せる。再開の失敗で実行木を abort しない。`usecase/workflow/startup.rs:52-69` の `abort_startup_failure` による abort を、失敗の分類によらず行わない形にする。粒度は abort を行わないことと、一覧の読み込みも対象に含めることの指定まで。理由は、Kubernetes API conventions が controller の失敗を対象の終了状態に変換しないため。

- 「変えないもの」の範囲を維持する。対象は `gateway/desktop_client.rs:46-63`、`domain/daemon_supervision.rs`、`gateway/notion/service_impl.rs:71-107`、lock の空きを待つ処理、store の読み書きの入口、画面側のつなぎ直し、呼び出しから始まる処理の期限。粒度は対象の指定まで。理由は正本の指定で、いずれも他 ISSUE（#1879、#1904、#1893、#1881・#1892、#1891）の担当であるため。

## 変えないもの

- 各処理が成功しているときの実行の周期。terminal の保存の 250 ms、provider session のタイトルの取得の 20 秒、review comment の監視の 1 秒、repository の走査の変化による起動を変えない。理由は、正本の実装境界が作業列への移行だけを挙げており、周期の見直しを含まないため。
- `usecase/repository_state/worktree.rs` の `WorktreeState` が持つ走査の進行の状態（`refreshing`・`loading`・requested/applied generation・`scan_lock`）の domain への移設。理由は #1890 の要求がこれらに触れないため。所在と問題は報告済みで、移設は別途合意とする。
- やり直しの判断の根拠に、失敗の文面・原因の型・エラーの変種を使わないこと。同じ失敗かどうかの判定にも文面を使わない。理由は R-004 と R-007 の要求。
- #1879（生存の判定）と #1894（同時実行の枠の拒否の記録）の振る舞い。この変更は両者が使う待ち時間の計算と失敗の記録を用意するに留める。理由は正本が両者を別 ISSUE の担当としているため。

## 未確定・リスク

- 自動判断（Requirements・Behavior を最小の解釈で修正した箇所）。3 件。
  - R-001 の「失敗したとき」を「R-005 でやり直す分類の失敗をしたとき」に限定した。あわせて B-001 の GIVEN を「やり直す分類で失敗する」に揃えた。
  - R-010 の「その処理はやり直される」を R-005 の判断の対象に含めた。あわせて B-012 の GIVEN と AND を揃えた。
  - B-013 の AND を、R-011 の本文どおり「止め続けられることはない」へ揃えた。
- 未決のまま残した要求は無い。Assumptions に「自動判断: 未決」は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
- やり直しの回数に上限を置かないため、やり直す分類（`Temporary`・`RestartRequired`）で恒久的に失敗し続ける対象があると、その対象のやり直しは上限まで延びた間隔で daemon が動く限り続く。この場合、やり直さない分類ではないため R-006 の要対応にはならず、失敗の記録（R-007）の 1 件として回数が増え続けるだけになる。Requirements・Behavior はこの状態に別の結果を定めていない。
