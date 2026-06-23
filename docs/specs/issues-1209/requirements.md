# Requirements

## Type

新機能。性能・メモリ効率改善（マイルストーン M0「計測と予算を固定する」）の基盤として、Performance budget / telemetry を追加する。

関連: #1209 / #1191 / `docs/releash-performance-architecture-audit.md` M0（正本ドキュメント, commit `b0c5e4c2`）

## 背景と目的

速度・メモリ改善を進める前に、改善前後を比較できる共通指標が必要。現状は startup / repo snapshot / diff open / streaming / session IO / メモリ といった hot path を定量化する仕組みがなく、以降の改善 Issue（#1210 以降）が「数値を見ながら進める」ための基準値を持てない。

本 Issue では、ベンダ中立な計装（OpenTelemetry）で主要 hot path のメトリクスを取得し、本番テレメトリも取り込めるマネージド SaaS（New Relic 無料枠）へ送れるようにする。あわせて主要 hot path の性能予算をドキュメント化する。

ローカル dev 計測だけでは「単一マシン・特定 repo サイズ」しか見えず M0 の予算検証にならないため、SaaS へ送信できる前提とする点が本 Issue の核心。

さらに、New Relic は性能監視に加えてエラー監視（Errors Inbox）も備えるため、現状 Sentry が担っている crash / エラー監視と、現状 aptabase（`tauri-plugin-aptabase`）が担っている使用状況テレメトリ（`app_started` event 等）を New Relic + OpenTelemetry へ統合し、観測先 SaaS を 1 本化する。これにより N 重の計装基盤を避け、性能・メモリ・crash・利用イベントを同一アカウントで横断観測できるようにする。送信は OTel signal（log / span exception / span / metric）に統一し、ベンダ固有 SDK へのロックインを避ける。既存の Sentry 連携（`sentry`クレート / `SENTRY_DSN`）と aptabase 連携（`tauri-plugin-aptabase` / `telemetry_enabled` フラグ / `app_started` event 経路）は撤去する。

なお計測対象の中心は **AgentSession**（agent チャット）であり、session storage（list/get/save/save bytes）と応答ストリーミング（emit interval / dropped frame / payload size）が含まれる。直近の AgentSession 最適化（summary index + message paging への再設計 #1213、メッセージ保持のウィンドウ化 #1195）の効果を、本 Issue の計測で定量検証できるようにする。

## スコープ

### 計測対象メトリクス

以下を計測・観測できるようにする。括弧内は採用する OTel 計装種別（Issue 本文のマッピングに従う）。

- 起動・初期化（span）
  - startup time
  - first window ready
  - first repo snapshot ready
- Git / Diff hot path（span + latency histogram）
  - Git status scan duration
  - diff stats duration
  - review file open duration
- AgentSession streaming / payload（histogram / counter / gauge）
  - 対象は agent チャット応答のストリーミング（`AgentStreamSync` cumulative snapshot, `ws_bridge.rs`）。
  - Tauri event payload size（`serialized.len()`）
  - WS payload size（`serialized.len()`）
  - streaming emit interval
  - dropped frame count（累積 snapshot は reconnect 時のみ送る方針の検証用）
  - WS reconnect count（接続確立 = `ws_server/session.rs` の `set_sender(Some)` 経路, counter）
- AgentSession session IO（span + histogram / counter）
  - 対象は AgentSession のセッションストレージ（`adaptor/gateway/agent_session/session_storage`）。`list_sessions` / `get_session` / `get_session_page` / `save_session` 経路。
  - session list duration
  - session get duration（summary index / message paging の効果検証を含む）
  - session save duration
  - session save bytes
- リソース観測（gauge）
  - Rust プロセス RSS（native, `sysinfo`）
  - Rust プロセス CPU 使用率（native, `sysinfo` で RSS と同時取得）
  - mounted xterm count（frontend mount 数）
  - active PTY count（`pty_session` registry len）

### crash / エラー監視（Sentry からの統合）

現状 Sentry（`sentry_integration.rs`）が担う crash / エラー監視を New Relic + OpenTelemetry へ統合する。

- Rust の panic / 未捕捉エラーを捕捉し、OTel signal（log record ないし span の exception イベント）として New Relic へ送る（Errors Inbox で観測）。
- frontend（WebView）の未捕捉エラー / unhandled rejection を OTel web SDK で送る（現状 `VITE_SENTRY_DSN` 経路の置き換え）。
- 既存 Sentry の privacy 配慮（home dir / path を `~` へ scrub）を移植し、stacktrace の filename / path にユーザーの絶対パスを含めない。
- 既存の Sentry 連携（`sentry`クレート, `SENTRY_DSN`, `VITE_SENTRY_DSN`, `before_send` scrub, session tracking）は撤去する。
- crash / エラー送信は OTel に統一し、送信先 SaaS を設定で差し替え可能に保つ（ベンダ固有 SDK は導入しない）。

### 共通属性 / 操作 status（一般的な計測項目）

性能・メモリ計測で一般的に必須となる以下を、Issue 本文の項目に加えて計測する（M0「改善前後の比較」を成立させるため）。

- 共通 resource attribute: app version（`CARGO_PKG_VERSION`）、OS、build type（dev/release）。全 span / metric に付与し、ビルド間・環境間の比較軸とする。ユーザーデータは含めない。
- 操作 status: 主要 hot path（Git status scan / diff / review file open / session list・get・save）の成否（成功 / 失敗）を span status ないし counter で取得し、改善の副作用（失敗増）を検知できるようにする。

### 有効化範囲

- debug/dev build: 計測・送信を有効にし、列挙メトリクスを確認できる。
- release（配布）build: 既定 on で New Relic へ送信する。ユーザーは設定 UI から opt-out で送信を停止できる（A6）。送信内容は完全匿名（要求事項 5）。

### 計装基盤

- Rust: OpenTelemetry 計装（OTLP/gRPC）。span / metric を OTel API で発行する。
- WebView frontend: OpenTelemetry web SDK（OTLP/HTTP）。frontend 固有メトリクス（mounted xterm count 等）を発行する。
- 送信先: New Relic 無料枠（OTLP ネイティブ）。OTLP endpoint + license key を設定し、Rust / WebView を同一アカウントへ集約する。
- OTel に統一することで送信先 SaaS を後から差し替え可能にし、ロックインを避ける。

### 性能予算のドキュメント化

主要 hot path の初期性能予算（M0 案）を Spec / ドキュメントに固定する。

- app startup: orphan cleanup による visible startup block なし
- repo snapshot: 中規模 repo で 200ms 台を目標、長い scan は stale snapshot を返す
- file diff open: 小/中ファイル 500ms 未満、大ファイルは即 fallback 表示
- streaming event: 通常 frame payload は 64KB 未満、累積 snapshot は reconnect 時のみ
- session list: session 本文量に依存しない
- terminal: worktree あたり mounted xterm 数に上限

## 非スコープ

- **WebView JS heap の計測**。macOS の WKWebView では `performance.memory`（`usedJSHeapSize`）が Chromium 専用で取得不可、標準代替 `Performance.measureUserAgentSpecificMemory()` も cross-origin isolation 必須で Tauri 構成では非現実的なため除外する。メモリ観測は native プロセス RSS に一本化する。
- 計測値に基づく実際の性能改善・最適化（#1210 以降の後続 Issue）。
- 予算違反時のアラート / CI ゲート化。本 Issue は計測と予算ドキュメント化までとする。
- **UI 体感メトリクス（long task / input latency / frame rate 等）**。一般的な性能計測項目だが、JS heap 除外と同じ WKWebView 制約・取得コストがあるため別 Issue 候補とし、本 Issue では扱わない。

## 要求事項

1. 上記スコープに列挙したメトリクスを、debug/dev build で確認できる。加えて、release（配布）ビルドでも既定で New Relic へ送信する。ユーザーは opt-out で送信を停止できる（既定値は下記 A6 参照）。
2. Rust 側ロジックは Tauri バックエンドに実装する（プロジェクト方針 `rust-first-logic`）。frontend は frontend 固有メトリクス（DOM mount 数等）の発行と、表示用途に限定する。
3. 計装は OpenTelemetry に統一し、送信先 SaaS を設定で差し替え可能にする。
4. New Relic（OTLP endpoint + license key）へ Rust / WebView 双方のテレメトリを送信できる。
5. 通常動作時のログ・span・metric の attribute に、本文・tool 入出力・worktree path 等のユーザーデータを含めない（privacy 要求）。加えて、識別子について次の方針を満たす:
   - **付与しない識別子**: 個人を直接特定する識別子（氏名・メール・OS アカウント名等）、永続的なデバイス/インストール識別子（MAC ハッシュ・install UUID・OS マシン UUID 等）、**起動セッション ID**（プロセス単位の UUID）。これらはユーザー・マシン・起動を横断的に追跡可能にするため一切付与しない。
   - **付与してよい識別子**: OTel が span 発行に伴い既定で生成する trace ID / span ID。これは 1 操作の親子関係追跡に必須であり、起動間・マシン間で関連付け不能なため singling-out リスクは事実上ない。
   - **環境属性は最小限**: 共通 resource attribute は `service.version` / `os.type` / `releash.build_type` / `service.name` の 4 つに限定し、CPU model・画面解像度・locale 等のフィンガープリント材料を追加しない。
   - 計測は build（app version）単位の集計のみを目的とし、同一ユーザー・同一マシン・同一起動の横断追跡はしない。これにより送信データを GDPR Recital 26 の匿名データに寄せ、既定値（A6 / A7）を防御可能にする。
6. 主要 hot path の性能予算がドキュメント化されている。
7. テレメトリ送信先（endpoint / license key）未設定時は計装が no-op として安全に動作し、アプリ起動・通常動作を妨げない（仮定、下記参照）。
8. 全 span / metric に比較軸となる共通 resource attribute（app version / OS / build type）を付与する。
9. 主要 hot path の操作 status（成功 / 失敗）を計測し、改善の副作用（失敗増）を検知できる。
10. Rust の panic / 未捕捉エラーと frontend の未捕捉エラーを New Relic（Errors Inbox）で観測できる。送信は OTel signal に統一し、ベンダ固有 SDK を導入しない。
11. crash / エラー送信でも privacy 要求（要求事項 5）を満たす。stacktrace の filename / path 等に含まれるユーザーの絶対パスを scrub する。
12. 既存 Sentry 連携（`sentry`クレート, `SENTRY_DSN`, `VITE_SENTRY_DSN`）を撤去し、crash / エラー監視を New Relic に一本化する。crash 送信可否は既存同様ユーザーが切り替えられる。
13. 既存 aptabase 連携（`tauri-plugin-aptabase`, `telemetry_enabled` フラグ, `app_started` event 経路）を撤去し、使用状況イベントの観測を New Relic + OTel に統合する。`app_started` 相当は startup span / metric として OTel 経由で送る。N 重実装回避のため SaaS は New Relic に 1 本化する。

## 受け入れ基準の概要

- debug/dev build で、列挙した全メトリクス（JS heap を除く）を New Relic 上で確認できる。
- span / metric の attribute にユーザーデータ（本文・tool 入出力・worktree path 等）が含まれないことを確認できる。
- 主要 hot path の性能予算がドキュメント（本 Spec ないし `docs/releash-performance-architecture-audit.md`）に記載されている。
- 送信先未設定でもアプリが正常起動する。
- release ビルドで、既定状態（opt-out 操作なし）ではテレメトリが New Relic へ送信される。opt-out 状態では送信が行われない。
- 各メトリクスに app version / OS / build type が付与され、ビルド間で比較できる。
- 主要 hot path の成功 / 失敗を New Relic 上で区別できる。
- Rust の panic / 未捕捉エラー、frontend の未捕捉エラーを New Relic（Errors Inbox）で確認できる。
- crash / エラーの stacktrace・attribute にユーザーの絶対パス等が含まれない（scrub される）ことを確認できる。
- `sentry`クレート / `SENTRY_DSN` / `VITE_SENTRY_DSN` が撤去され、crash 監視が New Relic に一本化されている。
- `tauri-plugin-aptabase` / `telemetry_enabled` フラグ / `app_started` event 経路が撤去され、`app_started` 相当が New Relic 上で OTel startup span / metric として確認できる。

## 仮定

以下は Issue とリポジトリ現状から置いた仮定。誤りがあれば指摘で修正する。

- **A1: 設定の供給方法**。OTLP endpoint / license key は、既存 Sentry DSN と同様にビルド時の環境変数（`env!`）で埋め込み、有効化フラグは `releash.toml` の `[telemetry]` セクションで持つ。既存 `TelemetrySection { crash_reporting }` を拡張し `performance_telemetry`（仮称）等のフラグを追加する。
- **A2: 既存依存の活用と Sentry / aptabase 撤去**。`reqwest`（rustls）は導入済み。OTLP/HTTP 送信に再利用しうる。`opentelemetry` / `opentelemetry-otlp` / `opentelemetry_sdk` / `sysinfo` は未導入のため新規追加する。crash 統合に伴い `sentry`クレート（および関連プラグイン）を撤去し、利用イベント統合に伴い `tauri-plugin-aptabase` を撤去する。crash / エラーは OTel log signal、`app_started` 相当は OTel startup span / metric で送る（ベンダ固有 SDK は追加しない）。
- **A3: 環境区分**。Sentry と同様 `cfg!(debug_assertions)` で development / production を区別し、属性へ付与する（ユーザーデータは含めない）。
- **A4: no-op フォールバック**。license key / endpoint が空の場合、計装初期化を skip し、計測コードは no-op で素通りする。
- **A5: Spec ディレクトリ名**は `docs/specs/issues-1209` とする（近接 Issue の命名慣行に合わせる）。
- **A6: 既定値（確定）**。release ビルドの性能テレメトリは**既定 on（送信する）**とし、ユーザーが明示的に opt-out した場合に送信を停止する。既存 `[telemetry]` セクションを拡張し、有効化フラグを設定 TOML / 設定 UI から切り替えられるようにする。**根拠**: payload は完全匿名（要求事項 5: 永続デバイス ID / インストール識別子 / ユーザー識別子・パス・本文を一切含めない）で、個人を単一化できないため GDPR Recital 26 の匿名データに該当し同意要件の対象外。VS Code・Zed が既定 on を採用する根拠（匿名化）よりさらに厳格（識別子そのものを付与しない）。改善前後の比較に必要な母数を確保する観点でも既定 on が合理的。opt-out 操作は設定 UI から常時可能。
- **A7: crash 統合の既定値と経路**。crash / エラー監視は既存 Sentry の挙動を踏襲し、性能テレメトリ（既定 opt-out）とは別フラグ `crash_reporting`（既定 opt-in = 送信する）で制御する。観測先のみ Sentry から New Relic へ移すもので、ユーザーの crash 送信同意の既定値は変更しない。crash / エラーは OTel log signal（exception 属性付き）として OTLP で送り、New Relic Errors Inbox で観測する。送信先未設定（endpoint / license key が空）の場合は no-op（crash も送らない）。

## Open Questions

なし（Q1「有効化するビルド範囲」は確定: release も送信する。既定値は A6 のとおり既定 on（完全匿名のため）、opt-out 操作で停止可能。crash 統合（Sentry→New Relic）の方針は背景・スコープ・要求事項 10〜12・A7 に反映済み）。
