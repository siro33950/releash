# Context

## 入力文書

- 要求の正本: [Issue #1704](https://github.com/siro33950/releash/issues/1704)「fix(local-api): sandbox 内の provider session から submit できない — discovery の PID 照合が seatbelt で成立せず InvalidDiscovery に誤診される」（state: OPEN、label なし、milestone なし、2026-08-27 作成）
- 調査で参照した実装
  - `src-tauri/src/infrastructure/local_api/client.rs:59-88` — discovery の読み込みと検証（`discover`）、失敗種別（`LocalApiClientError`）
  - `src-tauri/src/infrastructure/local_api/client.rs:135-143` — `matches_server_instance`（`/.well-known/releash-local-api/<instance_id>` への無認証 GET が 204 を返すかの確認）
  - `src-tauri/src/infrastructure/local_api/discovery.rs:26-38` — `process_start_time`（sysinfo 経由の他プロセス参照）
  - `src-tauri/src/infrastructure/local_api/server.rs:25-137` — discovery file の書き出しと identity endpoint の登録
  - `src-tauri/src/cli/api_client.rs` — CLI から local API を呼ぶ入口と失敗の分類（`discover` の Err は `CliError::Other` へ落ちる）
  - `src-tauri/src/cli/common.rs:50-53` — CLI の stderr 表示形式（`error: <message>`）
  - `src-tauri/src/adaptor/gateway/provider_lifecycle/launch_spec.rs:182-206` — provider session 起動時の permission と sandbox flag の対応
- 規約: `AGENTS.md`「構成で押さえる点」「セキュリティ」

## 確定済みの背景と制約

- local API は 127.0.0.1 のみに bind し、`{data_dir}/local-api.json`（0600）へ port、token、instance_id、pid、process_started_at を書き出す。CLI はこの file から接続先と token を得る。
- CLI が local API へ接続する前の検査は 2 段ある。(1) discovery file の内容検査（port / token / instance_id / process_started_at の空・0 検査と、`process_start_time(pid)` が `process_started_at` と一致するかの照合）。(2) token を送る前に、接続先が discovery の instance_id を名乗る local API であることを無認証 GET で確認する。どちらが不成立でも `InvalidDiscovery` になる。
- Releash は provider session を codex の workspace-write sandbox 下で起動する（Manual は `--sandbox workspace-write`、Auto は `--approve-for-me`。後者は `codex --help` に「using the workspace-write sandbox」と明記されている）。permission 未指定時も codex の設定既定（`sandbox_mode = "workspace-write"`）で同じ sandbox になる。
- codex 0.149.1 に埋め込まれた seatbelt policy の base policy は `(deny default)` であり、loopback 通信の許可規則を持たない。実機計測では、sandbox 内から loopback への connect は既定設定でも `sandbox_workspace_write.network_access` を有効にしても拒否される。sandbox 下の provider session から local API へ到達する手段はない。
- CLI の失敗は stderr へ `error: <message>` の形で出る。

# Outcome

- 対象者: Releash の workflow で provider session に作業させ、その成果物を artifact として提出させる開発者。
- 現在の問題: sandbox 内の provider session から local API を使う CLI command が失敗するとき、失敗表示が原因を「discovery file が不正または古い」と断定する。実際の原因は loopback 接続の遮断であり discovery file は正常であるため、agent は discovery の再取得やアプリ再起動といった無関係な復旧を試み、session が空転する（Issue 記載の実例: 提出 4 回失敗のあと `open -a Releash` の実行に至った）。
- 変更後に実現する状態: 失敗表示が「接続先を確認できなかった」ことと「discovery が古い／別インスタンスを指す」ことを区別して示し、無関係な復旧へ誘導しない。

# Current Behavior

## 再現手順

1. Releash アプリを起動する（local API が稼働し discovery file が存在する状態にする）。
2. provider session と同じ sandbox 下で CLI を実行する。最小手順は `codex sandbox -- /Applications/Releash.app/Contents/MacOS/releash workflow diagnostics`（codex-cli 0.149.1、macOS Darwin 25.5.0、`sandbox_mode = "workspace-write"`、network access 無効）。
3. 同じ command を sandbox 外で実行して比較する。

## 実際の出力

- sandbox 内（終了コード 1）:

  ```
  error: local API discovery file が不正または古いです (/Users/siro33950/Library/Application Support/com.releash.app/local-api.json)
  ```

- sandbox 外: 診断結果（`info WFI000 [workflow=01_author-spec]: ビルトインワークフロー '01_author-spec'` 以下）を出力し、終了コード 0。
- Issue が報告する `workflow output submit` でも同じ文言で失敗し、同時刻の sandbox 外実行では discovery 検証を通過してサーバへ到達する（偽の node-execution に対し `Node execution not found`）。

## どの検査が失敗しているか

稼働中の Releash（pid 80733、port 56242）を対象に、上と同じ sandbox 内で各検査を個別に計測した。

| 検査 | sandbox 内の結果 |
|---|---|
| discovery file の読み込みと decode | 成功 |
| `proc_listallpids` | 成功（888 プロセスが可視） |
| `proc_pidinfo`（PROC_PIDTBSDINFO） | 成功（`start_tvsec` が discovery の `process_started_at` と一致） |
| `sysctl kern.procargs2` | 成功 |
| `proc_pidpath` | 成功 |
| `connect(127.0.0.1:56242)` | `EPERM`（Operation not permitted）で拒否 |
| AF_UNIX socket への `connect` | `EPERM` で拒否（socket 許可の明示指定なしの場合） |

- `process_start_time` が使う sysinfo 0.39.6 の macOS 経路は `proc_listallpids` → `proc_pidinfo` →（`sysctl kern.procargs2` または `proc_pidpath`）であり、いずれも sandbox 内で成功する。したがって PID 照合は sandbox 内でも成立する。
- 実際に失敗しているのは instance_id の照合であり、その原因は照合結果の不一致ではなく、loopback 接続そのものが sandbox に拒否されることである。`matches_server_instance` は送信失敗と instance 不一致を同じ `false` に潰し、`discover` はそれを `InvalidDiscovery` として返す。このため「接続できない」が「discovery file が不正または古い」と表示される。
- Issue が sandbox の遮断根拠として挙げた `pgrep` の失敗は、pgrep が使う sysmond への mach-lookup が禁じられていることによるもので、`process_start_time` が使う経路とは別である。

## 影響範囲

- discovery 検証は local API を使う CLI command で共有されるため、失敗は submit に限らない（上の再現は `workflow diagnostics`）。
- file fallback を持つ read 系 command も、discovery 検証が Err を返した時点で fallback へ進まずそのまま失敗する。
- loopback 接続が拒否される以上、discovery 検証を通過させても、続く request 自体が同じ理由で失敗する。

# Scope / Non-goals

## 変更する

- CLI が local API へ接続する際の discovery 検証の判定規則と、失敗の分類および表示。

## 変更しない

- sandbox 内の provider session から local API へ到達する経路の確保。sandbox は loopback 接続を拒否するため、本変更で提出が成立するようにはならない。到達手段の確保は別途扱う。
- local API の loopback 限定 bind、Bearer token 認証、terminal 用 token の分離、discovery file の 0600 権限という既存のセキュリティ前提。
- workflow の実行モデル、artifact 提出の意味論、`workflow output submit` の入力仕様。
- provider（codex / claude）の承認運用そのもの（escalation の禁止、承認 policy の変更など）。
- 各 CLI command に固有の振る舞い。discovery 検証は共有されるため変更の効果は他 command にも同じ規則で及ぶが、command 固有の要求は本変更に含めない。

# Requirements

- R-001: local API の接続先を確認できなかった場合に、その失敗を「discovery file が不正または古い」と断定しない。失敗表示は、確認できなかったこと（接続不能・環境による遮断）と、確認した上で不成立だったこと（別インスタンスを指す・陳腐化している）を区別して示す。
- R-002: プロセス情報を参照できる環境では、discovery が指すプロセスが存在しない場合、および開始時刻が discovery の値と一致しない場合に、従来どおり接続を拒否する。
- R-003: discovery の token を送信する前に、接続先が discovery の instance_id を持つ local API であることを確認する、という現在の保証を弱めない。
- R-004: local API が起動しておらず discovery file が存在しない場合の失敗表示と終了コードを変えない。
- R-005: プロセス情報を参照できない環境では、discovery を不正・陳腐化と断定せず、プロセス情報を参照できないため接続先を確認できないことを示す専用の失敗として拒否する。この場合は identity GET を含む接続先への request を送信しない。

# Assumptions / Open Questions

## Assumptions

- なし。

## Open Questions

- なし。
