# leaf ドメイン一括移行 — あるべき姿（実装計画）

GitHub Issue: [#1168](https://github.com/siro33950/releash/issues/1168) / 親マイルストーン: [[12] クリーンアーキテクチャ移行](https://github.com/siro33950/releash/milestone/72)

本ドキュメントは [`docs/architecture/`](../../architecture/) の規約（README / DOMAIN / USECASE / GATEWAY / CONTROLLER / TEST）と、先行移行事例 `agent_session`（#977）/ `workflow`（#978, PR #1167）を前提に、**互いに型を共有しない 6 つの leaf ドメイン**を 1 バッチ（1 goal / 1 PR）で新クリーンアーキテクチャへ移行する際の **移行後のあるべき姿** を定義する。実装そのものではなく、ターゲット構造・責務境界・移行順序・設計判断を確定させることを目的とする。

> **本ドキュメントは Codex への実装計画である。** 記載はすべて確定済みの指示であり、§9 の当初未決 4 点も「最も正しい配置」で確定済み（合意取得済み）。本書の指示どおり実装すること。§1 の「スコープ境界」だけは越えてはならない（他 Issue の担当領域）。

---

## 0. 用語と前提（誤読防止のための定義）

| 用語 | 本ドキュメントでの意味 |
|---|---|
| **本バッチ** | 本 Issue #1168 で 1 PR にまとめる作業範囲。下記 6 ドメインのみ。 |
| **対象 6 ドメイン** | `pty_session` / `external_editor` / `workspace_state` / `remote_access` / `hooks` / `notification` |
| **no-shim** | 旧モジュールに互換用の re-export / ラッパーを一切残さず**物理削除**する方針（#977 / #978 を踏襲）。 |
| **外部契約** | Tauri command 名・WebSocket message 名・主要 request/response の JSON shape。 |
| **register_all** | `src-tauri/src/adaptor/controller/command/mod.rs` に存在するコマンド一括登録関数。各ドメインの `register` を集約する。 |

**依存方向は規約どおり内向きのみ:** `infrastructure → adaptor → usecase → domain`。逆依存に例外はない。

---

## 1. 目的とスコープ

### 目的

- 結合度が低く互いに型を共有しない 6 つの leaf ドメインを、クリーンアーキテクチャの層構成（`domain` / `usecase` / `adaptor` / `infrastructure`）へ移行する。
- 旧モジュールを **compatibility shim を残さず完全に削除** する（no-shim）。
- 外部契約（Tauri command 名・WS message 名・JSON shape）は**維持する**（純粋リファクタリング。振る舞いを変えない）。

### スコープに含む（対象 6 ドメインと現行の主な所在）

| ドメイン | 子 Issue | 現行の主な所在（実ファイル:行数） |
|---|---|---|
| `pty_session` | #975 | `src-tauri/src/pty/mod.rs`(1,317), `pty/backend.rs`(33), `pty/direct.rs`(151), `protocol/pty.rs`(257), `ws_server/handlers.rs` の `handle_pty_*` |
| `external_editor` | #987 | `src-tauri/src/external_editor.rs`(201) |
| `workspace_state` | #981 | `src-tauri/src/workspace_state_store.rs`(368) |
| `remote_access` | #984 | `src-tauri/src/vpn_detect.rs`(307), `qr_code.rs`(89), `tls.rs`(138) |
| `hooks` | #982 | `src-tauri/src/config.rs` の hooks 系 3 コマンド（`generate_hooks_config` L561 / `apply_hooks_config` L631 / `get_hooks_status` L666） |
| `notification` | #983 | `src-tauri/src/webhook.rs`(377), `config.rs` の `NotifySection`(L189-219) と notify 系 3 コマンド |

### スコープ境界（**本バッチに含めない** — 誤って手を出さないこと）

以下は他 Issue の担当範囲、または本バッチの no-shim 移行を成立させるための最小限の差し替えに留める。**構造を再設計してはならない。**

1. **`config.rs` 本体の分解は #1169（config 系）の担当。** 本バッチでは `config.rs` を**分解しない**。hooks / notification / external_editor / remote_access(TLS) は `config.rs` の設定を読むが、その読み取りは各ドメインの **gateway 経由**にする一方、`AppConfig` / `ReleashConfig` / `*Section` 構造体そのものの再配置・分割は #1169 に委ねる。本バッチでは「`config.rs` の関数を呼ぶ薄い gateway 実装」を置くに留める。
2. **`ws_server/` 基盤（session / routing / http / commands）の構造変更は #1172 の担当。** `remote_access` の `vpn_detect` / `qr_code` / `tls` を domain/gateway 化し、`ws_server/commands.rs` の `start_server` 等からの呼び出しを **gateway 経由に差し替える**が、`ws_server/` のディレクトリ構造・セッション設計は変更しない。
3. **`shell_integration.rs`（163 行）は `pty_session` ドメインの infrastructure として移動する（確定）。** `pty/mod.rs` と密結合し PTY の OSC 統合を担うため `infrastructure/pty_session/shell_integration.rs` へ物理移動する。hooks ドメインには含めない（責務が別 — §6.5）。
4. **フロントエンド（`src/`）の構造変更**はマイルストーン全体のスコープ外。`invoke(...)` の呼び出し名が変わらない限りフロントは一切変更しない。
5. **永続化フォーマットの変更**をしない。`workspace_state/*.json`、TLS 証明書（`$data_dir/tls/`）、`~/.claude/settings.json` への hooks マージ形式は現行を維持し、domain model とは gateway の mapper で変換する。

---

## 2. 現状の問題点（共通）

- 6 ドメインのロジックが `src-tauri/src/` 直下のフラットなファイル（`webhook.rs`, `vpn_detect.rs`, `workspace_state_store.rs` 等）に置かれ、`domain` / `usecase` / `adaptor` の層が存在しない。
- ビジネスロジック（VPN 判定、証明書有効期限、通知要否、エディタ検出、ワークスペース復元時のファイル削除フィルタ）が OS 呼び出し・HTTP・ファイル I/O と密結合し、純粋ロジックとして独立していない。
- Tauri コマンドが `config.rs`（hooks/notify/editor 設定）や各 leaf ファイルに散在し、`controller` 層に集約されていない。
- 通知送信（`notification`）が `infrastructure/agent_session/runtime/bridge_common.rs:1424` の `notify_status_transition` 内に埋め込まれ、agent_session の infra が通知の業務手順を抱えている。

---

## 3. あるべき姿（ターゲット構造） — ドメイン別

各ドメインは独立しており、互いに型を共有しない。以下を**各ドメインで個別に**満たす。

### 3.1 `pty_session`（#975）

**Ubiquitous language:** 「ターミナル / one-shot コマンド実行のための PTY セッション群を起動・読み書き・終了・worktree 単位で GC する」。

**集約根: `PtySessionRegistry`（全 PTY セッションの集合）。** 個別の `PtySession` は entity でありその内部要素。`gc_by_worktree` / `kill_by_worktree` / `find_by_worktree` / `list` は集合全体に作用する操作であり、Registry のメソッドとして表現される。worktree 単位の整合性（同 worktree の活きたセッション群を 1 トランザクションで GC できる）も Registry が保証する。

最大かつ唯一 WebSocket 経路を持つドメイン。`pty/mod.rs` に domain（`PtyManager` の集約ロジック）・infrastructure（`spawn_output_reader` のスレッド I/O・`app.emit`）・controller（8 Tauri コマンド）が混在している。

```text
domain/pty_session/
├── mod.rs
├── entities/
│   └── pty_session.rs          # PtySession 相当（pty_id, session_key, worktree_path, kind, label, exited, exit_code）
├── value_objects/
│   ├── pty_id.rs               # u64 newtype（単調増加 ID の生成規則は domain）
│   ├── pty_kind.rs             # PtyKind { Terminal, OneShot }
│   └── session_key.rs
├── gateway.rs                  # PtyBackendGateway（spawn/write/resize/kill）+ PtyOutputSink（出力通知）trait
├── services.rs                 # find_by_worktree / kill_by_worktree / gc_by_worktree / UTF-8 リングバッファ処理の純粋ロジック
└── error.rs                    # PtySessionError（thiserror）

usecase/pty_session/
├── mod.rs
├── spawn_usecase.rs            # get_or_spawn / spawn のオーケストレーション
├── io_usecase.rs               # write / resize
├── lifecycle_usecase.rs        # kill / kill_by_worktree / gc_by_worktree / remove_if_exited
├── query_service.rs            # list_pty_sessions（read model）
└── dto.rs                      # PtySessionInfo（serde, 現行 JSON shape を維持）

adaptor/gateway/pty_session/
├── mod.rs
├── backend_impl.rs             # PtyBackendGateway 実装（現 pty/backend.rs + pty/direct.rs を集約、portable_pty 依存をここに閉じる）
├── output_sink_impl.rs         # PtyOutputSink 実装（Tauri event "pty-output"/"pty-exit" emit + WsBroadcaster へのブロードキャスト）
└── session_models.rs           # domain ⇔ wire 変換

adaptor/controller/command/pty_session/
├── mod.rs                      # register(builder) -> Builder（8 コマンド）
└── commands.rs                 # spawn_pty / write_pty / resize_pty / list_pty_sessions / kill_pty /
                                #   get_or_spawn_pty / kill_ptys_by_worktree / gc_ptys_for_worktree

adaptor/controller/handler/pty_session/
└── mod.rs                      # handle_pty_input / handle_pty_spawn_request /
                                #   handle_pty_output_request / handle_pty_kill_request（現 ws_server/handlers.rs から移設）

adaptor/protocol/pty.rs         # 現 protocol/pty.rs を移設（PtyOutputMsg/PtyExitMsg/PtyInput/PtyResize/PtyReady/
                                #   PtySpawnRequest/PtySpawnResponse/PtyOutputRequest/PtyKillRequest/PtyKillResponse）

infrastructure/pty_session/
└── shell_integration.rs        # 現 src-tauri/src/shell_integration.rs を移設（PTY の OSC 統合・シェル初期化スクリプト生成）
```

- **外部契約維持:** 8 Tauri コマンド名、10 protocol メッセージ型、`ws_server/routing.rs` の `route_pty_*` 分岐先（関数名は移設可・WS message 名は不変）。
- **`shell_integration` の移設（確定 / §6.5）:** `src-tauri/src/shell_integration.rs` を `infrastructure/pty_session/shell_integration.rs` へ物理移動する。`pty/mod.rs:431` `create_shell_integration_files` / `:114` `strip_osc_cmd_done` の呼び出しは `backend_impl` / `output_sink_impl` から行う。移動に伴い `lib.rs:32` の `mod shell_integration;` 宣言と全参照（`pty/mod.rs` 等）を新パスへ更新する。
- **`protocol/pty.rs` の移設（確定 / 旧 §9-D）:** `protocol/pty.rs` を `adaptor/protocol/pty.rs` へ移設し、`protocol/mod.rs:13` の `pub use pty::*` 再エクスポートを廃して参照元を新パスへ切り替える。`protocol/` 基盤整理（#1171）に先行するが、pty_session の no-shim 完結のため本バッチで移設する。
- **出力バッファ（64KB リングバッファ、終了後 5 分保持）** は infrastructure / gateway の transient state。domain Entity に持ち込まない。

### 3.2 `external_editor`（#987）

**Ubiquitous language:** 「インストール済みエディタを検出し、選択されたエディタでファイル / フォルダ / ワークフロー定義を開く」。

**集約根: なし（ステートレスドメイン）。** 永続化される選択値（`AppSection.external_editor`）は value object（`SelectedEditor`）として表現するが集約ではない。検出・起動・パス検証は domain service。

`tauri_plugin_opener` への直接依存と `/Applications` スキャンが `external_editor.rs` に同居。

```text
domain/external_editor/
├── mod.rs
├── value_objects/
│   └── editor_info.rs          # EditorInfo { name, path }
├── gateway.rs                  # EditorLauncherGateway（open_path）+ InstalledEditorGateway（scan）trait
├── services.rs                 # KNOWN_EDITORS 定義・スキャン結果の重複排除/優先順位・パス検証（validate_path）
└── error.rs

usecase/external_editor/
├── mod.rs
├── detect_usecase.rs           # detect_editors
└── open_usecase.rs             # open_in_editor（file）/ open_folder_in_editor（folder）

adaptor/gateway/external_editor/
├── mod.rs
├── launcher_impl.rs            # EditorLauncherGateway 実装（tauri_plugin_opener::OpenerExt）
└── scanner_impl.rs             # InstalledEditorGateway 実装（application_dirs / scan_applications_in）

adaptor/gateway/external_editor/
├── ...                         # （上記 launcher_impl / scanner_impl に加えて）
└── settings_gateway_impl.rs    # EditorSettingsGateway 実装（config.rs の get/update_external_editor を呼ぶ薄いラッパー）

adaptor/controller/command/external_editor/
├── mod.rs                      # register(builder) -> Builder（5 コマンド）
└── commands.rs                 # detect_editors / open_in_editor / open_folder_in_editor /
                                #   get_external_editor / update_external_editor
```

- **WebSocket / protocol 型は無い**（Tauri コマンドのみ）。
- **既存の workflow 用 editor gateway との関係（重要・誤読防止）:** 既に `adaptor/gateway/workflow/editor_gateway.rs`(175) と `usecase/workflow/ports.rs:48` `ExternalEditorGateway` trait、`adaptor/controller/command/workflow/{definition.rs:70, facet.rs:102}` の `open_workflow_in_editor` / `open_facet_in_editor` が存在する。**これらは `workflow` ドメインの所有物であり、本バッチで移動・改名しない。** ただし `editor_gateway.rs` の実体（エディタ起動）は本ドメインの `launcher_impl` と重複するため、**workflow 側 gateway 実装が本ドメインの `EditorLauncherGateway` を内部利用する形に差し替える**（README「同じ操作の実装は 1 つに集約」）。`usecase/workflow/ports.rs` の trait 定義はそのまま残す。
- **設定コマンドの凝集（確定 / 旧 §9-B）:** `get_external_editor`（config.rs L968）/ `update_external_editor`（config.rs L976）の 2 コマンドを **external_editor ドメインの controller に含める**（計 5 コマンド）。設定の読み書きは `EditorSettingsGateway` 経由とし、§1 スコープ境界 1 に従い gateway 実装は `config.rs` の既存関数を呼ぶに留める（`config.rs` 本体の `AppSection` 分割は #1169）。これにより「外部エディタに関する操作」を 1 ドメインへ凝集させる。

### 3.3 `workspace_state`（#981）

**Ubiquitous language:** 「worktree 単位のエディタタブ・レイアウト状態を保存・復元する。復元時は実体が消えたファイルを取り除く」。

**集約根: `WorkspaceState`（worktree 単位）。** 1 worktree につき 1 集約。tabs / layout / active_editor_path / selected_diff_file は集約内の不変条件を共有する（例: active_editor_path が指すファイルは tabs に含まれていなければならず、消えていればフォールバックする）。集約跨ぎの整合性は無く、worktree 間は完全に独立。`HashMap<worktree_name, WorkspaceState>` のキャッシュ管理は gateway 側の責務。

`workspace_state_store.rs` 単独。CRUD 主体だが「削除されたファイルの除外・active path フォールバック」という純粋ロジックを持つ。

```text
domain/workspace_state/
├── mod.rs
├── entities/
│   └── workspace_state.rs      # WorkspaceState { version, tabs, layout }
├── value_objects/
│   ├── workspace_tabs_state.rs # WorkspaceTabsState / WorkspaceTabEntry
│   └── workspace_layout_state.rs
├── repository.rs               # WorkspaceStateRepository（load/save/set）trait
├── services.rs                 # 削除ファイル除外フィルタ・active_editor_path/selected_diff_file フォールバック（純粋関数）
└── error.rs

usecase/workspace_state/
├── mod.rs
├── usecase.rs                  # save_workspace_state（Command）
├── query_service.rs            # load_workspace_state（読み取り。services のフィルタを適用）
└── dto.rs                      # WorkspaceState の DTO（現行 camelCase JSON shape を維持）

adaptor/gateway/workspace_state/
├── mod.rs
├── repository_impl.rs          # WorkspaceStateStore 実装（RwLock<HashMap> インメモリ + JSON 永続化 + file_lock）
└── command_models.rs           # domain ⇔ 永続化モデル変換（serde camelCase）

adaptor/controller/command/workspace_state/
├── mod.rs                      # register(builder) -> Builder（2 コマンド）
└── commands.rs                 # load_workspace_state / save_workspace_state
```

- **永続化先 `{app_data_dir}/workspace_state/{safe_name}.json` と camelCase 形状を維持。**
- **WebSocket / protocol 型は無い。**

### 3.4 `remote_access`（#984）

**Ubiquitous language:** 「モバイル端末からデスクトップへ安全に接続できる状態を作る」。VPN 検出 / TLS 自己署名証明書 / QR コードはすべてこの単一概念の構成要素であり、別ドメインに割らない（インフラ特性の違いで分割しない）。

**集約根: `ServerEndpoint`（接続エンドポイント = bind IP / port / mode / token / TLS 有効化 / 証明書）。** ライフサイクル上の不変条件:
- bind IP と TLS 証明書の SAN は一致しなければならない（IP 変更時は証明書を再発行する）。
- TLS 有効時はスキーム `https`、無効時は `http`。`ConnectionUrl` はこの集約から導出される。
- QR コード（`QrCodeResult`）は `ServerEndpoint` のスナップショットからの read model。

`DetectedInterface` は集約外の値オブジェクト（OS 観測結果）であり、`ServerEndpoint` の `bind` 候補を選ぶための入力にすぎない。

`vpn_detect.rs` / `qr_code.rs` / `tls.rs` の 3 ファイル。ビジネス知識（VPN プレフィックス判定、プライベート IP 判定、証明書 365 日有効・IP 変更で再生成、QR URL 生成）が OS コマンド・ファイル I/O・暗号ライブラリに密結合。

```text
domain/remote_access/
├── mod.rs
├── value_objects/
│   ├── detected_interface.rs   # VpnInterface / DetectedInterface（kind: "vpn"|"lan"）
│   ├── connection_url.rs       # http/https スキーム + bind + port + token
│   └── qr_code_result.rs       # QrCodeResult { url, svg, token_svg }
├── gateway.rs                  # NetworkInterfaceGateway（list_interfaces/routes）+ CertificateGateway（load/ensure）+ QrRenderGateway
├── services.rs                 # is_vpn_interface / is_private_ip / has_active_routes 判定・is_cert_expired・build_connection_url（純粋ロジック）
└── error.rs

usecase/remote_access/
├── mod.rs
├── network_usecase.rs          # get_network_info / detect_vpn_tunnel
├── certificate_usecase.rs      # ensure_self_signed_cert / load_tls_config（ws_server から委譲される）
└── qr_usecase.rs               # get_connection_qr

adaptor/gateway/remote_access/
├── mod.rs
├── network_impl.rs             # NetworkInterfaceGateway 実装（ifconfig / netstat -rn 実行・パース）
├── certificate_impl.rs         # CertificateGateway 実装（rcgen / rustls_pemfile / tokio_rustls）
└── qr_impl.rs                  # QrRenderGateway 実装（qrcode クレート、SVG 生成）

adaptor/controller/command/remote_access/
├── mod.rs                      # register(builder) -> Builder（3 コマンド）
└── commands.rs                 # get_network_info / detect_vpn_tunnel / get_connection_qr
```

- **外部契約維持:** Tauri コマンド `get_network_info` / `detect_vpn_tunnel` / `get_connection_qr` の 3 つのみが本ドメインの controller。
- **ws_server との接続（§1 スコープ境界 2）:** `ws_server/commands.rs:39`（VPN 検出 → connection_mode 決定）と `:56`（TLS 証明書生成）、`ws_server/http.rs:74`（TLS アクセプタ構築）は、本ドメインの **usecase / gateway を呼ぶ形に差し替える**。`ws_server/commands.rs` の `start_server` / `stop_server` / `get_server_status` / `get_server_info` / `update_terminal_startup_command` の 5 コマンドは **#1172 の担当であり本バッチでは移動しない**（呼び出し先だけを gateway 経由に置換）。
- **`config.rs` の `TlsSection`(L177-229):** §1 スコープ境界 1 により本バッチでは分解せず、gateway が読み取る。

### 3.5 `hooks`（#982）

**Ubiquitous language:** 「Claude Code agent から Releash へ inbound でイベント通知させるための統合設定」。`~/.claude/settings.json` はその実体ファイルにすぎず、ドメインの本質は「agent → Releash 通知経路の設定」。`app_config`（Releash 自身の環境設定）とは別概念。

**集約根: `HooksSettings`（Claude Code agent の hooks 設定全体）。** 1 ユーザ環境につき 1 集約。集約内の不変条件:
- 期待する hook event の集合（UserPromptSubmit / Stop / Notification ×2 / PostToolUse / PostToolUseFailure / SessionStart）と、それぞれが指す endpoint（`http://localhost:{hook_port}/hooks/agent`）・Bearer token は整合していなければならない。
- `HooksStatus`（`active` / `not_configured` / `token_mismatch`）はこの不変条件の判定結果（read model）。

現状 `config.rs` 内の 3 コマンドのみ。`~/.claude/settings.json` への Claude Code フック定義の生成・マージ・状態確認。

```text
domain/hooks/
├── mod.rs
├── value_objects/
│   ├── hook_event.rs           # UserPromptSubmit/Stop/Notification/PostToolUse/PostToolUseFailure/SessionStart
│   └── hooks_status.rs         # "active" | "not_configured" | "token_mismatch"
├── services.rs                 # フック定義 JSON の生成ルール・settings.json マージ規則・status 判定（純粋ロジック）
└── error.rs

usecase/hooks/
├── mod.rs
├── usecase.rs                  # apply_hooks_config（settings.json へマージ）
└── query_service.rs            # generate_hooks_config（定義生成）/ get_hooks_status（状態確認）

adaptor/gateway/hooks/
├── mod.rs
└── settings_repository_impl.rs # ~/.claude/settings.json の読み取り・アトミックマージ書き込み

adaptor/controller/command/hooks/
├── mod.rs                      # register(builder) -> Builder（3 コマンド）
└── commands.rs                 # generate_hooks_config / apply_hooks_config / get_hooks_status
```

- **`hook_port`（`config.rs` の `ServerSection.hook_port` L160）と Bearer token は gateway/services が読み取る。** `config.rs` 本体は分解しない（§1 スコープ境界 1）。
- **WebSocket / protocol 型は無い。** hook 受信 HTTP サーバ（`/hooks/agent` エンドポイント）は現状未実装であり、**本バッチで新規実装しない**（純粋リファクタリングのため）。
- **`shell_integration.rs` は hooks に含めない**（§6.5 / §9-A）。

### 3.6 `notification`（#983）

**Ubiquitous language:** 「agent 状態変化を外部チャネル（Slack / Discord、将来は他チャネル）へ outbound に配信する」。`hooks`（inbound）とは方向と相手システムが違うので別ドメイン。

**集約根: なし（ステートレスドメイン）。** 永続化される設定（`NotifyConfig` = 現 `NotifySection`）は value object。1 件の通知イベントの判定・配信は単発の usecase 呼び出しで完結し、トランザクション境界としての集約を持たない。状態を持つのは外部（agent_session の `AgentStatusCenter` / `FocusTracker`）であり、notification ドメインはそれらをスナップショット入力として受ける。

`webhook.rs` の通知判定・ペイロード構築（純粋ロジック）と HTTP 送信が同居。トリガーは agent_session の infra。

```text
domain/notification/
├── mod.rs
├── value_objects/
│   ├── notify_config.rs        # NotifySection 相当（webhook_url, on_running/done/error/waiting, desktop_mode, inactive_timeout_minutes）
│   ├── desktop_notify_mode.rs  # Always | WhenInactive
│   └── notification_event.rs   # AgentStateSync から導出した通知イベント（state, branch, exit_code）
├── gateway.rs                  # WebhookSenderGateway（send）trait
├── services.rs                 # should_notify 判定・is_discord_webhook・build_slack_payload・build_discord_payload・extract_branch（純粋ロジック）
└── error.rs

usecase/notification/
├── mod.rs
├── usecase.rs                  # on_agent_status_changed(event)（should_notify → build_payload → gateway.send）
├── query_service.rs            # get_notify_config
└── dto.rs

adaptor/gateway/notification/
├── mod.rs
└── webhook_sender_impl.rs      # WebhookSenderGateway 実装（reqwest POST, 5s timeout）

adaptor/controller/command/notification/
├── mod.rs                      # register(builder) -> Builder（3 コマンド）
└── commands.rs                 # get_notify_config / update_notify_config / update_webhook_url
```

- **通知トリガーの接続（確定 / 旧 §9-C — 疎結合方式を採用）:** `notification` の業務手順（状態遷移を受けて通知要否を判定し送信する）を `infrastructure/agent_session/runtime/bridge_common.rs` から**完全に剥がす**。`bridge_common.rs:1424` `notify_status_transition` は `AgentStatusCenter` 更新・`agent_status_events` emit という agent_session 本来の責務のみを担い、**通知ロジック（`should_notify` 判定・webhook 送信）を一切持たない**。通知は agent_session の状態変更リスナー（#977 の `register_state_change_listener` パターン）に `notification` usecase（`on_agent_status_changed`）を購読登録し、状態確定後に発火させる。`step_lifecycle_adapters.rs:454` 経由の経路も同じリスナーに集約する。
  - **設計意図:** agent_session の infra が「通知すべきか・どこへ送るか」という notification ドメインの業務知識を抱える現状は依存方向の汚染である。リスナー経由にすることで agent_session → notification の依存をイベント購読（疎結合）に置き換え、agent_session は notification を知らずに状態変更を公開するだけになる。これがクリーンアーキテクチャ上「最も正しい」境界。
  - agent_session（#977 移行済み）への変更は「`notify_status_transition` から通知ロジックを除去し、リスナー発火点を残す」最小限に留めるが、**通知責務の移譲という構造変更は本バッチで完遂する**（妥協して通知ロジックを bridge_common に残さない）。
- **`AgentStateSync`（`protocol/agent.rs:28`）は agent_session / protocol の所有物。** notification はこれを入力 DTO として受けるが、protocol 型を移動しない。
- **`config.rs` の `NotifySection`(L189-219) と 3 コマンド:** §1 スコープ境界 1 により `config.rs` 本体は分解しない。3 コマンドは controller の薄い入口に移し、設定の読み書きは gateway 経由（`config.rs` の関数を呼ぶ）にする。`NotifySection` 構造体の最終的な再配置は #1169。
- **将来チャネル拡張（Email / Teams 等）** は `WebhookSenderGateway` の実装追加で対応できる構造にするが、**本バッチでは Slack / Discord のみ**（振る舞いを増やさない）。

---

## 4. DI 配線（`lib.rs` / `adaptor/controller/state.rs`）

- 各ドメインの usecase / query service / gateway 実装を **composition root（`lib.rs`）で生成**し、`AppState`（`adaptor/controller/state.rs`）へ注入して `builder.manage(...)`。
- `pty_session` の `PtyManager`（現 `lib.rs:78` で `manage`）は、gateway 実装として `AppState` に保持し、`ws_server`（`ws_server/mod.rs:72` `pty_manager`）からも参照できる形を維持する。
- **`lib.rs` の `invoke_handler!` 直接列挙を廃し、各ドメインの `register(builder)` を `adaptor/controller/command/mod.rs` の `register_all` から呼ぶ形に統一する**（受け入れ条件「`register_all` 経由に統一」）。Tauri の `invoke_handler` は 1 度しか呼べないため、`register_all` が全ドメインの `generate_handler!` を 1 回で合成する現行方式を踏襲する。
- `notification` のトリガー接続は、agent_session の状態変更リスナー（#977 の `register_state_change_listener` パターン）から `notification` usecase を呼ぶ形にする。

---

## 5. 移行計画（PR は 1 本 / コミット粒度をドメイン単位にする）

本 Issue は **1 goal / 1 PR**。ただしレビュー容易性と二分探索可能性のため、**コミットはドメイン単位**で積む。各ドメインのコミット時点で `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

依存が無いため順序は任意だが、**規模が小さく独立性が高い順**を推奨する:

| 順 | ドメイン | 規模目安 | 備考 |
|---|---|---|---|
| 1 | `workspace_state` | 368 行・WS 無し・config 非依存 | 最も独立。パターン確立用 |
| 2 | `external_editor` | 201 行・WS 無し | workflow 既存 gateway との接続（§3.2）に注意 |
| 3 | `hooks` | 3 コマンド | config 読み取りを gateway 化 |
| 4 | `notification` | 377 行 | agent_session トリガー差し替え（§3.6） |
| 5 | `remote_access` | 534 行（3 ファイル） | ws_server 呼び出し差し替え（§3.4） |
| 6 | `pty_session` | 1,758 行・WS 有り | 最大。最後に実施 |

### no-shim 移行の実施手順（各ドメイン共通）

1. `domain/<d>/` → `usecase/<d>/` → `adaptor/gateway/<d>/` → `adaptor/controller/{command,handler}/<d>/` の順に新規追加。
2. 各層の追加時にテストを追加（§7）。
3. 旧ファイルの呼び出し側（`lib.rs`, `ws_server/*`, `bridge_common.rs`, `config.rs` の該当コマンド等）を新 controller / usecase に切り替える。
4. **旧ファイルを物理削除**（`pty/`, `external_editor.rs`, `workspace_state_store.rs`, `vpn_detect.rs`, `qr_code.rs`, `tls.rs`, `webhook.rs`、および `config.rs` / `protocol/` から移設した分）。re-export を残さない。
5. そのドメインのコミット時点で CI 相当（fmt / clippy / test）を通す。

> 旧コードと新コードの一時併存は許容するが、**各コミット末ではビルド・テストが通る状態**にすること。コンパイル不能なコミットを積まない。

---

## 6. 主要な設計判断（確定事項）

### 6.1 6 ドメインは互いに型を共有しない

leaf ゆえ相互依存が無い。共通の値オブジェクト・エラー型を作って束ねない。各ドメインは独立した `domain/<d>/` を持つ。横断する唯一の接点は `AppState`（DI 受け皿）のみ。

### 6.2 外部契約の維持（純粋リファクタリング）

Tauri command 名・WS message 名・JSON shape を維持する。内部で新 usecase / VO / typed error を使っても、controller / handler が既存契約へ変換する。**整理目的だけの rename / shape 変更はしない**（マイルストーン「既存機能の振る舞い変更なし」）。

### 6.3 `config.rs` を分解しない（#1169 との境界）

hooks / notification / external_editor(設定) / remote_access(TLS) は `config.rs` の `*Section` を読むが、本バッチでは **gateway が `config.rs` の既存関数を呼ぶ**に留める。`config.rs` の構造分割は #1169。これにより本バッチと #1169 のコンフリクトを避ける。

### 6.4 `ws_server/` 基盤を再設計しない（#1172 との境界）

`remote_access` と `pty_session` は `ws_server/` から参照されるが、`ws_server/` の構造変更は #1172。本バッチは **呼び出し先を gateway/usecase 経由に差し替える**のみ。`ws_server/commands.rs` の 5 コマンド・`session.rs`・`routing.rs`・`http.rs` の構造は不変。

### 6.5 `shell_integration.rs` は `pty_session` の infrastructure（hooks ではない）

`shell_integration.rs`(163) は Claude Code のフック設定生成ではなく、**PTY シェルの OSC 統合（`__releash_precmd` / `strip_osc_cmd_done`）**で、責務が hooks と異なり `pty/mod.rs` と密結合する。よって hooks ドメインには含めず、`infrastructure/pty_session/shell_integration.rs` へ物理移動する（§3.1）。

### 6.6 read model と Entity の区別

- domain Entity / VO: `WorkspaceState`、`PtySession`、`DetectedInterface`、通知要否ルール、証明書ポリシー（アプリの都合で意味が決まるもの）。
- read model / DTO: `PtySessionInfo`、`WorkspaceState` の転送 DTO、`QrCodeResult`、`HooksStatus`（表示・転送の都合で形が決まるもの）。query_service が直接構築し、`Entity → DTO` の機械的詰め替えはしない。

---

## 7. テスト方針（受け入れ条件「domain / usecase / gateway 層のテストを追加」）

各ドメインで以下を満たす（[TEST.md](../../architecture/TEST.md) 準拠）:

| 層 | モック | テスト内容（例） |
|---|---|---|
| domain | 不要 | VO 不変条件・純粋ロジック。例: `is_vpn_interface`、`is_private_ip`、`is_cert_expired`、`should_notify`、`build_slack/discord_payload`、削除ファイル除外フィルタ、`validate_path`、`generate_pty_id` 単調増加 |
| usecase | gateway/repository モック注入 | 正常系・異常系。例: `on_agent_status_changed`（送信要否分岐）、`get_or_spawn_pty`、`save/load_workspace_state` |
| gateway | 一時リソース | trait 実装の正しさ。例: workspace_state の JSON ラウンドトリップ、TLS 証明書生成・再利用、settings.json マージ |
| controller | — | 薄さを保つ（ロジックを持たない） |

- **既存テストは移植する**（消さない）。現状のテスト所在: `pty/mod.rs:701+`(13), `external_editor.rs:108-200`(10), `workspace_state_store.rs:182-368`(8), `vpn_detect.rs:189+`/`qr_code.rs:54+`/`tls.rs:99+`(計 17), `config.rs` hooks テスト(L1202-1340), `webhook.rs:132-377`(24)。これらを新しい層配置に対応させて移す。
- 旧 edge case を固定しているテストは、仕様として残すか旧実装由来として整理するか個別判断する（振る舞い不変が原則のため、原則は残す）。

---

## 8. 受け入れ条件（Issue より / 完了判定チェックリスト）

- [ ] 6 ドメインすべてが `domain` / `usecase` / `adaptor`（/ `infrastructure`）へ配置され、依存方向が内向きのみ。
- [ ] 旧モジュール（`pty/`, `external_editor.rs`, `workspace_state_store.rs`, `vpn_detect.rs`, `qr_code.rs`, `tls.rs`, `webhook.rs` および移設分）を**完全削除**（no-shim）。
- [ ] domain / usecase / gateway 層のテストを追加（既存テストを移植）。
- [ ] `lib.rs` のコマンド登録を `register_all` 経由に統一。
- [ ] Tauri command 名・WS message 名・JSON shape が不変（フロント `invoke` 呼び出しが無変更で動く）。
- [ ] CI フルグリーン: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（`src-tauri/`）、`pnpm lint` / `pnpm test` / `pnpm build`（ルート）。

---

## 9. 確定した設計判断（旧・未決事項 — すべて「最も正しい配置」で確定）

当初の未決 4 点は、影響範囲・差分最小化ではなく**クリーンアーキテクチャ上の正しさ**を基準にすべて確定済み。Codex は以下を指示として実装すること。

| 項目 | 確定内容 | 反映箇所 |
|---|---|---|
| **9-A** `shell_integration.rs` の配置 | `infrastructure/pty_session/shell_integration.rs` へ物理移動。`lib.rs` の `mod` 宣言・全参照を更新。 | §1-3, §3.1, §6.5 |
| **9-B** editor 設定コマンドの帰属 | `get_external_editor` / `update_external_editor` を external_editor ドメインの controller に凝集（計 5 コマンド）。設定読み書きは `EditorSettingsGateway` 経由。 | §3.2 |
| **9-C** notification トリガー接続 | 通知ロジックを `bridge_common.rs` から完全に剥がし、agent_session の状態変更リスナーへ `notification` usecase を購読登録する疎結合方式。agent_session infra は通知業務知識を持たない。 | §3.6, §4 |
| **9-D** `protocol/pty.rs` の移設 | `adaptor/protocol/pty.rs` へ移設し `protocol/mod.rs` の再エクスポートを廃止。#1171 に先行して本バッチで完結。 | §3.1 |

> いずれも「振る舞いを変えない（外部契約維持）」制約は保ったまま、**内部の責務境界・ファイル配置のみを理想形へ寄せる**。差分が増えること・他 Issue 領域（agent_session / protocol / config）へ最小限踏み込むことは許容する。ただし §1 スコープ境界 1（`config.rs` 本体の構造分割）・2（`ws_server/` 基盤の構造変更）は #1169 / #1172 の担当として侵さない — これは「正しさ」ではなく**責務分担の境界**であり、本バッチで分割すると二重実装・コンフリクトを生むため。

---

## 10. 責務移動一覧（現所在 → 移動先）

「現状どの責務がどこにあり、どの層へ移動するか」を明示する。`§3.X` の構造図とこの表が一致することで、移行漏れを構造的に防ぐ。

### 10.1 `pty_session`

| 現所在 | 責務 | 移動先 |
|---|---|---|
| `pty/mod.rs` `generate_pty_id` | PTY ID 単調増加生成ルール | `domain/pty_session/value_objects/pty_id.rs` |
| `pty/mod.rs` `process_pty_output` 内 UTF-8 / リングバッファ | UTF-8 境界処理・64KB リング | `domain/pty_session/services.rs` |
| `pty/mod.rs` `find_session` / `kill_by_worktree` / `gc_by_worktree` 判定 | 集合操作の純粋ロジック | `domain/pty_session/entities/pty_session_registry.rs` のメソッド |
| `pty/mod.rs` `PtyManager` の `HashMap<u64, PtySession>` 保持 | transient state（in-memory index） | `adaptor/gateway/pty_session/backend_impl.rs`（gateway の保持物） |
| `pty/mod.rs` `spawn_output_reader` スレッド | I/O 読み込みループ | `adaptor/gateway/pty_session/output_sink_impl.rs` |
| `pty/mod.rs` `app.emit("pty-output"/"pty-exit")` / `WsBroadcaster.try_send` | Tauri event / WS broadcast | `adaptor/gateway/pty_session/output_sink_impl.rs` |
| `pty/mod.rs` 8 Tauri commands | 入口 | `adaptor/controller/command/pty_session/commands.rs` |
| `pty/backend.rs` `PtyBackend` / `PtyResizer` / `SpawnConfig` | trait 定義 | `domain/pty_session/gateway.rs` に再定義（domain 所有） |
| `pty/direct.rs` `DirectPtyBackend` / `DirectResizer` | `portable_pty` 実装 | `adaptor/gateway/pty_session/backend_impl.rs` |
| `protocol/pty.rs` 10 WS メッセージ型 | wire DTO | `adaptor/protocol/pty.rs`（`protocol/mod.rs:13` 再エクスポート廃止） |
| `ws_server/handlers.rs` `handle_pty_*` 4 関数 | WS ハンドラ | `adaptor/controller/handler/pty_session/` |
| `src-tauri/src/shell_integration.rs` 全体 | OSC シェル統合・初期化スクリプト生成 | `infrastructure/pty_session/shell_integration.rs`（§3.1） |
| `lib.rs:78` `manage(Arc::new(PtyManager::default()))` | DI 配線 | `lib.rs` で gateway / usecase を生成し `AppState` 注入 |

### 10.2 `external_editor`

| 現所在 | 責務 | 移動先 |
|---|---|---|
| `external_editor.rs` `KNOWN_EDITORS` / `scan_applications_in` / 重複排除 / 優先順位 | エディタカタログ・スキャン純粋ロジック | `domain/external_editor/services.rs` |
| `external_editor.rs` `validate_path*` | パス検証ルール | `domain/external_editor/services.rs` |
| `external_editor.rs` `application_dirs` / 実ディレクトリスキャン I/O | OS ファイルシステム読み取り | `adaptor/gateway/external_editor/scanner_impl.rs` |
| `external_editor.rs` `open_path_with_opener`（`tauri_plugin_opener` 呼び出し） | 起動 I/O | `adaptor/gateway/external_editor/launcher_impl.rs` |
| `external_editor.rs` 3 Tauri commands（`detect_editors` / `open_in_editor` / `open_folder_in_editor`） | 入口 | `adaptor/controller/command/external_editor/commands.rs` |
| `config.rs:968-993` `get_external_editor` / `update_external_editor` | 設定読み書きの Tauri 入口 | `adaptor/controller/command/external_editor/commands.rs`（凝集・§3.2） |
| `config.rs:968-993` 設定値読み書き実体 | `AppConfig` 経由の TOML I/O | `adaptor/gateway/external_editor/settings_gateway_impl.rs`（内部で `config.rs` の既存関数を呼ぶ薄いラッパー） |
| `adaptor/gateway/workflow/editor_gateway.rs` のエディタ起動部分 | 起動 I/O の重複実装 | external_editor の `EditorLauncherGateway` を内部利用する形に置換（`usecase/workflow/ports.rs` の trait 定義は不変） |

### 10.3 `workspace_state`

| 現所在 | 責務 | 移動先 |
|---|---|---|
| `workspace_state_store.rs` `WorkspaceState` / `WorkspaceTabsState` / `WorkspaceLayoutState` / `WorkspaceTabEntry` 構造 | エンティティ・VO | `domain/workspace_state/entities` + `value_objects`（camelCase JSON 形状は維持） |
| `workspace_state_store.rs` 削除ファイル除外フィルタ・`active_editor_path` フォールバック・`selected_diff_file` フォールバック | 集約内不変条件の復元ロジック | `domain/workspace_state/services.rs` |
| `workspace_state_store.rs` `WorkspaceStateStore`（`RwLock<HashMap>` キャッシュ + JSON 永続化 + `file_lock`） | gateway 実装 | `adaptor/gateway/workspace_state/repository_impl.rs` |
| `workspace_state_store.rs` ファイル名 safe 化（`/`, `\` → `_`） | 永続化キー変換 | `adaptor/gateway/workspace_state/command_models.rs` |
| `workspace_state_store.rs` 2 Tauri commands | 入口 | `adaptor/controller/command/workspace_state/commands.rs` |

### 10.4 `remote_access`

| 現所在 | 責務 | 移動先 |
|---|---|---|
| `vpn_detect.rs` `is_vpn_interface` / `is_private_ip` / `has_active_routes` / `parse_routes_for_interface` 判定 | ネットワーク判定の純粋ロジック | `domain/remote_access/services.rs` |
| `vpn_detect.rs` `VpnInterface` / `DetectedInterface` 型 | 値オブジェクト | `domain/remote_access/value_objects/detected_interface.rs` |
| `vpn_detect.rs` `list_network_interfaces` / `parse_ifconfig_output` / `ifconfig` / `netstat -rn` 実行 | OS コマンド実行・パース | `adaptor/gateway/remote_access/network_impl.rs` |
| `vpn_detect.rs` 2 Tauri commands（`get_network_info` / `detect_vpn_tunnel`） | 入口 | `adaptor/controller/command/remote_access/commands.rs` |
| `qr_code.rs` `build_connection_url`（http/https スキーム合成） | URL 構築の純粋ロジック | `domain/remote_access/services.rs` + `value_objects/connection_url.rs` |
| `qr_code.rs` `generate_qr_svg`（qrcode crate） | SVG 生成 I/O | `adaptor/gateway/remote_access/qr_impl.rs` |
| `qr_code.rs` 1 Tauri command（`get_connection_qr`） | 入口 | `adaptor/controller/command/remote_access/commands.rs` |
| `tls.rs` `is_cert_expired`（365 日判定）/ IP 不一致再生成判定 | 証明書ライフサイクル・集約不変条件 | `domain/remote_access/services.rs` + 集約 `ServerEndpoint` の不変条件 |
| `tls.rs` `load_tls_config` / `ensure_self_signed_cert`（`rcgen` / `rustls_pemfile` / `tokio_rustls`） | 暗号ライブラリ I/O・ファイル I/O | `adaptor/gateway/remote_access/certificate_impl.rs` |
| `tls.rs` `bind_ip` ファイル記録 | 永続化詳細 | `adaptor/gateway/remote_access/certificate_impl.rs` |
| `ws_server/commands.rs:39` VPN 検出呼び出し | start_server 内の責務混在 | remote_access usecase 呼び出しに差し替え（`ws_server/commands.rs` の関数構造は不変・§1 スコープ境界 2） |
| `ws_server/commands.rs:56` TLS 証明書生成呼び出し | 同上 | remote_access usecase 呼び出しに差し替え |
| `ws_server/http.rs:74` TLS アクセプタ構築 | 同上 | remote_access gateway 呼び出しに差し替え |
| `config.rs` `TlsSection`(L177-229) 読み取り | TLS 設定読み取り | remote_access gateway が `config.rs` の既存関数を呼ぶ薄いラッパー |

### 10.5 `hooks`

| 現所在 | 責務 | 移動先 |
|---|---|---|
| `config.rs:561` `generate_hooks_config` 内 フック定義 JSON 生成（6 event type） | 集約不変条件の生成ルール | `domain/hooks/services.rs`（純粋ロジック） |
| `config.rs:631` `apply_hooks_config` 内 settings.json マージ規則 | 集約マージの純粋ロジック | `domain/hooks/services.rs` |
| `config.rs:631` settings.json の atomic 読み書き | ファイル I/O | `adaptor/gateway/hooks/settings_repository_impl.rs` |
| `config.rs:666` `get_hooks_status` 判定（`active` / `not_configured` / `token_mismatch`） | 集約状態の判定（read model） | `domain/hooks/services.rs` + `value_objects/hooks_status.rs` |
| `config.rs` hooks 3 Tauri commands | 入口 | `adaptor/controller/command/hooks/commands.rs` |
| `config.rs` `ServerSection.hook_port`(L160) 読み取り | hook port 取得 | hooks gateway が `config.rs` の既存関数を呼ぶ薄いラッパー |

### 10.6 `notification`

| 現所在 | 責務 | 移動先 |
|---|---|---|
| `webhook.rs` `should_notify`（state / NotifySection / FocusTracker から要否判定） | 通知要否の純粋ロジック | `domain/notification/services.rs` |
| `webhook.rs` `is_discord_webhook` / `build_slack_payload` / `build_discord_payload` / `build_payload` / `extract_branch` | ペイロード構築の純粋ロジック | `domain/notification/services.rs` |
| `webhook.rs` `send_webhook`（reqwest POST, 5s timeout） | HTTP I/O | `adaptor/gateway/notification/webhook_sender_impl.rs` |
| `webhook.rs` reqwest::Client の毎回生成 | リソース効率改善余地あるが本バッチでは現状維持 | 同上（最適化は別 Issue） |
| `config.rs:189-219` `NotifySection`（`webhook_url` / `on_running/done/error/waiting` / `desktop_mode` / `inactive_timeout_minutes`） | 値オブジェクト構造 | `domain/notification/value_objects/notify_config.rs` + `desktop_notify_mode.rs`（`config.rs` 上の構造体定義そのものは #1169 まで残置、domain VO は同等形状で新規定義） |
| `config.rs:777-802, 929` notify 3 Tauri commands | 入口 | `adaptor/controller/command/notification/commands.rs` |
| `config.rs:777-802, 929` 設定値読み書き実体 | TOML I/O | `adaptor/gateway/notification/` 設定 gateway が `config.rs` の既存関数を呼ぶ薄いラッパー |
| `infrastructure/agent_session/runtime/bridge_common.rs:1424` `notify_status_transition` 内の `should_notify` 呼び出し・webhook 送信 spawn | 通知業務手順（domain 違反） | **削除**。同関数は `AgentStatusCenter` 更新・`agent_status_events` emit のみを担う |
| 同所 webhook 送信トリガー | 通知発火点 | agent_session の状態変更リスナー（#977 の `register_state_change_listener` パターン）に `notification` usecase `on_agent_status_changed` を購読登録する形に置換 |
| `infrastructure/agent_session/runtime/step_lifecycle_adapters.rs:454` `notify_status_transition` 呼び出し経路 | 同上 | 同じリスナーに集約され、追加配線不要 |
| `protocol/agent.rs:28-37` `AgentStateSync` | 通知入力 DTO | **移動しない**（agent_session / protocol の所有物）。notification は入力 DTO として `use` する |

### 10.7 共通（layer 横断）

| 現所在 | 責務 | 移動先 |
|---|---|---|
| `lib.rs` `invoke_handler!` 直接列挙 | コマンド登録 | 各ドメインの `register(builder)` を `adaptor/controller/command/mod.rs:register_all` から呼ぶ形に統一（§4 / §8 受け入れ条件） |
| `lib.rs:78` 等 各ドメインの `manage(...)` | DI 配線 | `AppState` に usecase / gateway を集約注入（composition root を controller に限定） |
