# Context

- 要求の正本: Issue #1200「A2: 全 req/resp コマンドの ws 被覆（A1 後にドメイン単位へ分割）」、milestone 77「02. ローカル server-client 化（基盤）」。
- 背景資料: Issue #1199（A1）、Issue #1201（A-flip）、`docs/specs/issues-1199/`、`src-tauri/src/adaptor/controller/command/mod.rs`、`src-tauri/src/adaptor/controller/command/client.rs`。
- milestone 77 で確定済みの設計判断のうち、本変更が従うもの。
  - 判断①: req/resp は Protocol Buffers で定義したエンベロープ（`request_id`＋`oneof command`）を経由して usecase 共有 dispatch へ渡す。push も proto message とする。`.proto` が protocol の正であり、Rust / client の型はそこから生成する。
  - 判断③: クライアント通信は WebSocket 単一トランスポートとする。
  - 判断④: ローカルは loopback＋token ファイル認証とする。
  - 判断⑦: クライアント token は discovery file の master token と分けて発行し、master token を renderer へ露出させない。
  - 判断⑧: terminal stream は汎用エンベロープに統合し、クライアント接続は 1 本にする。stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）はエンベロープ内に保持する。
  - 判断⑥: strangler 移行（A0 掃除 → A1 walking skeleton → ドメイン被覆 → 既定切替 → デーモン抽出 → 常駐）。本変更はドメイン被覆（A2）にあたる。
- 土台として確定している事項。
  - クライアント向け ws は MS82 で新設した local API（axum、127.0.0.1 bind、discovery file）の上に置く。
  - HTTP local API（workflow / provider-lifecycle）は CLI と provider hook のローカル専用入口として残し、クライアントの経路にはしない。
  - A0 で温存した旧 ws shell は #1338 で削除済みであり、再利用しない。
- milestone 77 の成果は、ローカルで desktop が daemon＋ws で完全動作すること（desktop 1 クライアント。CLI / hook の HTTP 入口は対象外）である。
- Issue #1200 は、`review-blob:` scheme の代替と、telemetry / watcher / application_lifecycle のうち UI shell に残す分の線引きを A-flip（#1201）で扱うと定めている。

# Outcome

- 対象者は、既定トランスポートの切替（A-flip）とデーモン抽出（A-daemon）を実装する開発者と、desktop 利用者である。
- 現在、クライアント ws（A1）で呼べる req/resp は `get_current_branch` の 1 本だけで、その他の command は Tauri invoke でしか呼べない。terminal はクライアント ws とは別の `/v1/terminal` route と Tauri Channel の 2 経路で動いており、クライアント接続が 1 本になっていない。A1 のエンベロープは JSON であり、`.proto` を正とする確定判断①と一致していない。このため desktop を ws クライアントとして完全動作させる前提が揃っていない。
- 変更後は、対象ドメインの command が提供する操作がクライアント ws の汎用エンベロープで行え、ws から呼んだ結果が Tauri invoke から呼んだ結果と一致する。desktop の該当 UI 機能はクライアント ws 経由で動作する。terminal もクライアント接続 1 本の上で動作し、`/v1/terminal` route と Tauri Channel fallback は存在しない。CLI と provider hook の HTTP 入口は変わらず動作する。

# Current Behavior

最初の周の開始時点（`feat/issues/1200`、`28f0eeed`）で確認した挙動。

## 登録済み command

- `adaptor/controller/command/` の各ドメインの `COMMAND_NAMES` の合計は 175 本である（Issue #1200 作成時点の記載は 159 本）。内訳は agent_session 16 / app_config 8 / application_lifecycle 10 / client 1 / code 24 / comment 8 / external_editor 5 / git_host 4 / menu 1 / notion 6 / repository 25 / telemetry 3 / terminal_surface 17 / watcher 3 / workflow 32 / workspace_state 2 / workspace_tree 10。
- client ドメインは A1 で追加された `get_client_endpoint` の 1 本で、クライアント ws の URL と認証 subprotocol（非 master token）を返す。
- menu ドメインは `set_menu_items_enabled` の 1 本である。
- 登録済み command のうち次の 34 本は、`src/` 配下にも `tests/` 配下にも command 名を文字列として呼び出す箇所がない（`tests/helpers/fixtures.ts` の mock のキーとしてだけ現れるものを含む）。
  - app_config: `get_crash_reporting_enabled`
  - code: `get_file_at_ref` / `get_staged_content` / `get_binary_staged_content` / `get_file_at_branch_base` / `get_binary_file_at_branch_base` / `get_binary_file_at_ref` / `get_branch_diff_summary` / `build_diff_file_tree` / `get_head_diff_file_tree_snapshot` / `compute_hidden_ranges` / `get_relative_path`
  - comment: `get_review_thread` / `get_review_thread_history`
  - git_host: `fetch_pr_status`
  - repository: `get_default_branch` / `get_git_status` / `get_git_status_snapshot` / `get_status_diff_stats` / `get_status_diff_stats_snapshot` / `get_git_log` / `get_worktree_dirty_count` / `get_repo_git_dir`
  - workflow: `approve_workflow_node` / `list_workflow_executions` / `get_workflow_execution` / `get_workflow_execution_log` / `get_workflow_node_detail` / `resolve_worktree_by_execution` / `list_facets` / `workflow_submit_output` / `workflow_validate_output` / `workflow_get_output`
  - application_lifecycle: `compact_application_shutdown_details`

## クライアント ws と共有 dispatch

- local API は `/v1/client`（クライアント ws）と `/v1/terminal`（terminal ws）を、非 master token と master token の双方を受理する bearer 認証の下に置く。workflow と provider-lifecycle の HTTP route は master token だけを受理する。
- クライアント ws の frame は JSON text である。要求は `CommandRequest{ request_id, command, args }` または `StreamEnvelope`（`stream { attachment_id, sequence, data }` / `ack { attachment_id, sequence }`）で、応答は `CommandResponse{ request_id, result | error }` である。
- `.proto` ファイルと Protocol Buffers の依存はリポジトリに存在しない。
- 共有 dispatch（`ClientCommandDispatch`）に登録された command は `get_current_branch` の 1 本だけである。Tauri invoke からの `get_current_branch` もこの共有 dispatch へ渡される。その他の command は Tauri invoke 用の handler 表だけに登録されている。
- stream frame の上限は 64 KiB と規約に定められているが、クライアント ws 上で stream frame は送受信されていない。

## push

- backend 状態の push 8 イベント（`agent-session-changed` / `branch-list-sync` / `file-change` / `git-status-changed` / `repo-paths-changed` / `repository-snapshot-changed` / `review-comments-changed` / `workflow-execution-changed`）は単一の sink を経由し、Tauri emit とクライアント ws の双方へ JSON（`status: "push"`、`event`、`payload`）で配られる。
- desktop がクライアント ws で購読する push は `workflow-execution-changed` だけで、その他は Tauri `listen` で購読する。

## desktop の呼び出し

- `@tauri-apps/api/core` を import する本番ファイルは 44、`invoke(` の呼び出し箇所は 149、文字列リテラルで呼ばれる command 名は 96 種類である。
- クライアント ws を使う箇所は `src/lib/clientSocket.ts` と、それを使う `useCurrentBranch` / `useWorkflowState` / `useWorkspaceTreeNodes` / `useWorkspaceNodeDetail` である。

## terminal

- terminal の attach・出力 stream・入力・ack・resize は `/v1/terminal` の独自エンベロープで行われる。`get_terminal_stream_endpoint` が `/v1/terminal` の接続情報を renderer へ渡す。
- `src/hooks/useTerminal.ts` は WebSocket を優先し、attach に失敗した場合と予期せず切断された場合は Tauri Channel（`attach_terminal_surface` の `onEvent`）へ切り替えて再同期する。

## 起動失敗時

- 起動 authority が失敗の場合、local API は bind されない。desktop は起動失敗画面で `get_application_startup_outcome` と `quit_after_startup_failure` を Tauri invoke で呼ぶ。

## その他

- `review-blob:` は Tauri の URI scheme protocol（`adaptor/controller/command/code/review_blob.rs`）であり、`COMMAND_NAMES` に含まれない。
- CLI と provider hook は HTTP local API（workflow / provider-lifecycle）を master token で呼ぶ。

# Scope / Non-goals

## 変更するもの

- 対象ドメイン（repository / code / comment / agent_session / terminal_surface / workflow / workspace_tree / workspace_state / app_config / notion / git_host / external_editor / telemetry / watcher / application_lifecycle）に登録された command が提供する操作の、クライアント ws 汎用エンベロープへの被覆。telemetry / watcher / application_lifecycle も登録済みの全 command を被覆する。desktop から呼び出す箇所がない command も被覆する。
- クライアント ws のエンベロープ（req/resp、push、stream）の `.proto` 定義と Protocol Buffers バイナリ frame への置き換え。
- desktop の該当 UI 機能の呼び出しの、Tauri invoke からクライアント ws 経由への差し替え。
- terminal の attach・出力 stream・入力・ack・resize のクライアント ws 汎用エンベロープへの移行と、stream フロー制御および frame 上限・分割の送受信時の適用。
- `/v1/terminal` route と、その接続情報を返す `get_terminal_stream_endpoint` の削除。
- desktop terminal の Tauri Channel fallback の削除と、クライアント ws の切断・接続失敗の後に接続が確立したときの terminal の再 attach。
- クライアント ws の切断により応答を受け取れなかった監視開始の要求で開始された監視を、backend に残さないこと。

## 変更しないもの

- menu（`set_menu_items_enabled`）。UI shell に残す。
- client ドメインの `get_client_endpoint`。クライアント ws の接続情報そのものであり、接続前に取得する必要があるためクライアント ws では被覆しない。
- desktop が `get_application_startup_outcome` と `quit_after_startup_failure` を呼ぶ経路（Tauri invoke）。起動失敗時は local API が bind されない。
- `review-blob:` URI scheme の代替。A-flip で扱う。
- telemetry / watcher / application_lifecycle のうち UI shell に残す分の線引き。A-flip で扱う。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口。
- push 8 イベントの desktop 購読経路（A1 でクライアント ws 経由にした `workflow-execution-changed` 以外）。Issue #1200 の対象は req/resp command であり、push は A1 でクライアント ws へ配られている。
- 不要になった invoke 経路の撤去、接続状態と再接続の全体設計、E2E 経路の置き換え。A-flip で扱う。
- UI shell に残す物（dialog、opener、updater＋process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）。
- headless デーモンの抽出と常駐。
- リモートアクセス経路。

# Requirements

- R-001: 対象ドメインに登録された command が提供する操作を、クライアント ws の汎用エンベロープで行える。req/resp の要求には同じ `request_id` を持つ応答が返り、成功時は結果、失敗時はエラーが返る。
- R-002: クライアント ws から呼んだ command は、同じ command を同じ引数で Tauri invoke から呼んだ場合と同じ usecase の結果（成功値またはエラー）を返す。
- R-003: クライアント ws の req/resp・push・stream の frame は、`.proto` に定義された Protocol Buffers メッセージのバイナリである。
- R-004: terminal の attach・出力 stream・入力・ack・resize が、クライアント ws の汎用エンベロープで、req/resp と push と同じ 1 本のクライアント接続上で行える。
- R-005: terminal の stream は `attachment_id` ごとに `sequence` で順序付けて送受信され、受信側は `attachment_id` と受信済みの `sequence` を `ack` として通知できる。
- R-006: frame 上限を超えるデータは規約に従って分割して送られ、各 frame は上限以下であり、受信側で欠落と順序の入れ替わりなく元のデータが得られる。
- R-007: local API に `/v1/terminal` route が存在せず、`get_terminal_stream_endpoint` も呼べない。
- R-008: desktop の terminal は、Tauri Channel を使わずクライアント ws だけで出力の表示と入力を行う。
- R-009: CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）は、変更前と同じ入口で同じ結果を返す。
- R-010: desktop の UI 機能が対象ドメインの command を呼ぶ操作は、クライアント ws 経由の要求で行われ、変更前と同じ結果になる。R-011 の 2 本を除く。
- R-011: desktop は `get_application_startup_outcome` と `quit_after_startup_failure` を変更前と同じく Tauri invoke で呼び、local API が bind されない起動失敗時にも起動失敗画面の表示と終了が行える。
- R-012: desktop の terminal は、クライアント ws が予期せず切断された後、またはクライアント ws に接続できず attach できなかった後、クライアント ws の接続が確立すると再 attach し、出力を再同期して表示と入力を再開する。
- R-013: desktop の terminal は、クライアント ws 上の attach 要求にエラー応答が返った場合、変更前と同じくその応答のエラーを terminal のエラーとして表示する。
- R-014: クライアント ws で受理された監視開始の要求（`start_watching` / `start_git_dir_watching`）への応答をクライアントが受け取る前にそのクライアント ws 接続が切断された場合、その要求で開始された監視は backend に残らない。

# Assumptions / Open Questions

すべて自動判断であり、人間の確認を経ていない。

- 自動判断（Q-001、規則1）: クライアント ws のエンベロープ（req/resp、push、stream）を、この変更で `.proto` 定義の Protocol Buffers バイナリ frame に置き換える。要求の正本である milestone 77 判断①が「`.proto` が protocol の正で、Rust / client の型はそこから生成する」「push も proto message」と定め、判断⑧が stream を同じ汎用エンベロープに統合すると定めているため、それに従う。背景資料との差: A1（PR #1768）の実装と `docs/specs/issues-1199/`（判断①を「push は typed を維持」とする改訂前の記載に基づく）は JSON text frame であり、proto 化を担う別 issue は milestone 77 に存在しない。
- 自動判断（Q-002、規則1）: desktop の該当 UI 機能の呼び出しを、この変更でクライアント ws 経由へ差し替える。要求の正本である Issue #1200 の完了条件「該当 UI 機能が ws で動作」に従う。背景資料との差: Issue #1201（A-flip）は「ws クライアントを 1 箇所に置いて呼び出し側を差し替える」を A-flip の作業に挙げている。Issue #1201 が A-flip で設計する物とする「Playwright の Tauri IPC mock に代わる E2E 経路」は Non-goals のままであり、Tauri IPC mock を前提とする既存 E2E との整合は Design で扱う。
- 自動判断（Q-002 に伴う）: `get_application_startup_outcome` と `quit_after_startup_failure` の desktop からの呼び出しは Tauri invoke のまま維持する。起動 authority が失敗した場合は local API が bind されず、`get_application_startup_outcome` は起動失敗かどうかを判定する前に呼ばれるため、クライアント ws へ差し替えると起動失敗画面の表示と終了ができなくなる。local API を起動失敗時にも bind する選択は新しい観測可能な挙動を増やし対象範囲を広げるため採らない（規則2）。backend 側の被覆は R-001 のとおり行う。
- 自動判断（Q-003、規則1）: telemetry / watcher / application_lifecycle は登録済みの全 command を被覆し、UI shell に残す分の線引きは A-flip で決める。要求の正本である Issue #1200 が 3 ドメインを対象ドメインに挙げ、線引きを A-flip と定めているため、それに従う。
- 自動判断（Q-004、規則1）: desktop から呼び出す箇所がない登録済み command 34 本（Current Behavior に列挙）は、他の command と同様にクライアント ws へ被覆する（R-001、R-002 の対象に含む）。要求の正本である Issue #1200 は被覆対象を「全 req/resp コマンド（Tauri command 159 本）」と登録済み Tauri command の数で定め、frontend からの呼び出し箇所（約 150 箇所）とは別に数えている。対象外は menu だけである。34 本のうち 26 本は Issue #1200 作成時点（2026-06-20）の `src-tauri/src` に既に存在しており、呼び出し箇所の有無で被覆対象を分けていない。被覆せず登録ごと削除する選択と、被覆せず現状のまま残す選択は、要求の正本が定める対象範囲と一致しないため採らない。
- 自動判断（Q-005、規則2）: Tauri Channel fallback の削除後、クライアント ws が予期せず切断された場合、またはクライアント ws に接続できず attach できなかった場合、desktop の terminal はクライアント ws の接続が確立した時点で再 attach し、出力を再同期する。変更前の terminal はこれらの場合に Tauri Channel で再 attach して再同期しており、出力の表示と入力が再開するという観測可能な結果を維持する選択である（規則2）。エラー表示に留める選択は変更前の回復を失わせるため採らない。接続状態と再接続の全体設計は Issue #1201 のとおり A-flip で扱う。
- 自動判断（attach 要求へのエラー応答、規則2）: クライアント ws 上の attach 要求にエラー応答が返った場合、desktop の terminal はその応答のエラーを terminal のエラーとして表示する（R-013）。変更前（`28f0eeed`）は WebSocket の attach に失敗すると Tauri Channel で attach し直し、その attach がエラーを返すと応答のエラーメッセージを terminal のエラーとして表示して、自動では attach を再試行しなかった。Tauri Channel fallback の削除後もエラーの表示という観測可能な結果を維持する選択である。エラー応答の後、クライアント ws の接続が続いている間に attach を自動で再試行することは要求しない。再試行を加える選択は変更前にない観測可能な挙動を増やすため採らない。エラー応答の後にクライアント ws が切断され接続が確立した場合は R-012 のとおりである。
- 自動判断（`get_terminal_stream_endpoint`）: `/v1/terminal` の接続情報を返す `get_terminal_stream_endpoint` は、`/v1/terminal` route とともに削除する。要求の正本である Issue #1200 が「旧 route を削除する」と定めており、削除後の route を指す接続情報を返す command を残すと存在しない接続先を返すことになるため、旧 route の一部として扱う（規則1）。
- 自動判断（Tauri Channel fallback の削除の帰属、規則1）: terminal の Tauri Channel fallback の削除はこの変更で行う。要求の正本である Issue #1200 の terminal_surface の項に従う。背景資料との差: Issue #1201 も「terminal の Tauri Channel fallback を撤去する」を A-flip の作業に挙げている。
- 自動判断（監視開始の要求への応答を受け取れなかった場合、規則2）: クライアント ws で受理された `start_watching` / `start_git_dir_watching` への応答をクライアントが受け取る前にクライアント ws 接続が切断された場合、その要求で開始された監視は backend に残さない（R-014）。desktop は監視開始の応答で受け取った監視の ID を保持し、画面の終了時にその ID で監視を停止する。変更前の Tauri invoke には、画面を維持したまま接続の切断によって監視開始の応答だけが失われる経路がなく、開始を要求した側が停止できない監視は残らなかった。クライアント ws では応答前の切断によって ID を受け取れず、停止できない監視が残る経路が生じるが、要求の正本はその扱いを定めていない。停止できない監視を残さないという変更前の状態を維持する選択である。切断時に接続上で開始された監視をすべて停止する選択は、応答を受け取って監視を保持している画面の監視を失わせるため採らない。接続の確立後に desktop が監視を開始し直すことは要求しない。開始し直す選択は変更前にない観測可能な挙動を増やし、接続状態と再接続の全体設計（A-flip）に踏み込むため採らない。受理済みの command を切断後も完了まで実行する性質は変えない。
- 自動判断（作業単位、規則1）: 本変更は Issue #1200 が挙げる対象ドメインすべてを扱う。Issue #1200 は「A1 のパターン確立後、ドメイン単位の issue に分割」と定めるが、分割された issue は存在せず、要求の正本は Issue #1200 そのものであるため。
