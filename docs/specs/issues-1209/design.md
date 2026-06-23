# Design

Performance budget / telemetry（#1209）の実装設計。`requirements.md` / `behavior.md` を満たす具体的な責務分割・型・処理フロー・テスト方針を定める。

実装経路・モジュール配置は本書で確定する。requirements の仮定 A1〜A7、behavior の Rule を前提とする。

---

## 1. 概要

主要 hot path（startup / Git・Diff / AgentSession streaming / AgentSession session IO / リソース観測）に OpenTelemetry の span / metric を仕込み、New Relic（OTLP ネイティブ）へ送信できる計装基盤を追加する。あわせて主要 hot path の初期性能予算（M0 案）をドキュメントへ固定する。

設計の柱は次の 4 点。

1. **横断的計装モジュールの新設**。OTel SDK 初期化（infrastructure 層）と、計測コードから呼ぶ薄い記録 API（`other/telemetry`）を分離する。hot path 側は記録 API を 1〜2 行呼ぶだけにし、no-op フォールバックを記録 API 内部で吸収する。
2. **既存 Sentry 計装パターンの踏襲＋Sentry/aptabase 撤去**。endpoint / license key は `env!` ビルド時埋め込み、有効化は `[telemetry]` セクション拡張、build type 区分は `BuildType` enum（D13）、空設定は no-op。既存の Sentry（crash）と aptabase（usage）は本 Issue で撤去し、OTel + New Relic に 1 本化する（D6 / 要求事項 12・13）。
3. **共通 resource attribute の一元付与**。app version / OS / build type は OTel の `Resource` に一度だけ設定し、全 span / metric へ自動付与する。各 hot path 側で個別付与しない。識別子は粒度別ポリシー（D8）で限定。
4. **ユーザーデータ非混入の構造的保証**。記録 API が受け取る attribute を「型で限定したホワイトリスト」に絞り、本文・tool 入出力・worktree path 等を物理的に渡せないシグネチャにする。
5. **crash / エラー監視の New Relic 統合**。現状 Sentry が担う crash / エラー監視を New Relic + OpenTelemetry へ一本化する（要求事項 10〜12 / A7）。Rust panic・未捕捉エラーと frontend 未捕捉エラーを OTel log signal（exception 属性付き）として OTLP 送信し、New Relic Errors Inbox で観測する。既存 Sentry の path scrub を移植し、`sentry`クレート / `SENTRY_DSN` / `VITE_SENTRY_DSN` を撤去する。crash 送信は性能テレメトリとは別フラグ `crash_reporting`（既定 on）で制御し、観測先のみ移す。
6. **ingest 量制御と frontend 経路の単純化**。streaming hot path は histogram / counter に集約し ingest 量を per-event 数から切り離す（D9 / 案 A）。frontend は OTel SDK を持たず、未捕捉エラー / mounted xterm count を Tauri コマンドで Rust に転送し Rust が一括 OTLP 送信する（D10）。license key は WebView に露出しない。

それ以外のロジックは全て Rust に置く（`rust-first-logic`）。

> **確定事項（本書 D1〜D13 / Q1〜Q3 解決済み）**: (D2/Q1) mounted xterm count は frontend→Tauri コマンド→Rust gauge。(D5/Q2) 性能予算の正本は `docs/releash-performance-architecture-audit.md` M0。(D7/Q3) OTLP endpoint / license key は既存 Sentry と同方式（`env!`＋`build.rs` 空 default＋release.yml 実値注入）。(D6) Sentry / aptabase 撤去し New Relic + OTel に 1 本化。(D8) 識別子粒度別ポリシー。(D9) streaming は histogram / counter 集約。(D10) frontend は Rust 転送経路に統一。(D11) session 計測は 5 metric 別建て（業務名固定）。(D12) provider 構築は同期、exporter ネットワークは background。(D13) `BuildType` enum を 1 箇所で確定。詳細は 10 章「仮定」参照。

---

## 2. 変更対象

### 2.1 新規追加（Rust）

| パス | 役割 | レイヤー |
|---|---|---|
| `src-tauri/src/infrastructure/telemetry/mod.rs` | OTel SDK 初期化（tracer / meter provider 構築、OTLP exporter 設定、Resource 構築、shutdown guard） | infrastructure |
| `src-tauri/src/infrastructure/telemetry/config.rs` | `env!` 埋め込み endpoint / license key、有効判定（空/opt-out で無効） | infrastructure |
| `src-tauri/src/other/telemetry/mod.rs` | hot path から呼ぶ記録 API（`record_*`）。global meter/tracer への薄い委譲と no-op 吸収 | other（横断） |
| `src-tauri/src/other/telemetry/attributes.rs` | 共通 attribute キー定数、許可された attribute 値の型（status enum 等） | other（横断） |
| `src-tauri/src/other/telemetry/resource.rs` | RSS / CPU を `sysinfo` で取得する resource observer | other（横断） |
| `src-tauri/src/infrastructure/telemetry/crash.rs` | panic hook 登録、crash / エラーを OTel log signal（exception 属性）として発行、path scrub の移植 | infrastructure |
| `src-tauri/src/adaptor/controller/command/telemetry/commands.rs` | frontend からの未捕捉エラー報告・mounted xterm count 報告・有効設定取得コマンド | adaptor/controller |

> **仮定（D1）**: 計装は「外部クライアント＝infrastructure」「記録 API＝横断的関心事 other」に分ける。OTel SDK 構築は git2/reqwest と同じ外部依存であり infrastructure、hot path から広く呼ばれる記録 helper は error.rs / utils.rs と同じ other に置く。domain / usecase は OTel 型に依存させない（後述 2.4）。

### 2.2 変更（Rust）

| パス | 変更内容 |
|---|---|
| `src-tauri/src/lib.rs` | `init_sentry()` 呼び出しと `tauri-plugin-aptabase` 初期化（`track_event("app_started", ...)` 含む）を `init_telemetry()` ＋ panic hook 登録 ＋ startup span 計測に置換。setup 内で OTel 初期化、first window ready / first repo snapshot ready の記録、shutdown guard 保持。 |
| `src-tauri/src/sentry_integration.rs` | **撤去**。path scrub（`scrub_home_dir`）は `infrastructure/telemetry/crash.rs` へ移植。`crash_reporting` ランタイムフラグ（AtomicBool）は telemetry 側へ移す。 |
| `src-tauri/src/adaptor/gateway/app_config/config_models.rs` | `TelemetrySection` に `performance_telemetry: bool`（**既定 true = 送信する**）を追加。**`telemetry_enabled`（aptabase 用）フィールドを `ReleashConfig` から削除**（migration: 既存値は読み飛ばす）。 |
| `src-tauri/src/adaptor/controller/command/app_config/commands.rs` | `update_performance_telemetry(enabled: bool)` コマンド追加。設定保存＋ランタイム有効化フラグ更新。 |
| `src-tauri/src/adaptor/gateway/repository/status.rs` | `get_git_status` / `get_status_diff_stats` を span + status counter で計測。 |
| review file open hot path（diff/review 経路の usecase or command） | review file open duration を span 計測。 |
| `src-tauri/src/adaptor/gateway/agent_session/session_storage.rs` | `list_metas` / `get_session_meta` / `get_session_page` / `append_message` / `save_full_session_for_migration_or_restore` を span + histogram + status counter で計測（5 metric 別建て、D11 対応表参照）。save 系は bytes も記録。 |
| `src-tauri/src/ws_bridge.rs` | `send_stream_sync` / event payload emit 経路で payload size（`serialized.len()`）・emit interval・dropped frame count を記録。 |
| `src-tauri/src/ws_server/session.rs` | `set_sender(Some(_))` 経路で WS reconnect count を increment。 |
| `src-tauri/src/domain/pty_session/entities/pty_session_registry.rs` 周辺 | active PTY count（registry len）を gauge observe できる経路を追加。 |
| `src-tauri/Cargo.toml` | `opentelemetry`（trace/metrics/logs）/ `opentelemetry-otlp` / `opentelemetry_sdk` / `sysinfo` を追加。`sentry`（および関連 plugin）と `tauri-plugin-aptabase` を削除。build 時 env（`OTLP_ENDPOINT` / `NEW_RELIC_LICENSE_KEY`）は既存 `SENTRY_DSN` と同じく `build.rs` で空 default 保証＋release.yml で実値注入。 |
| `src-tauri/build.rs` | `SENTRY_DSN` の空 default 行を `OTLP_ENDPOINT` / `NEW_RELIC_LICENSE_KEY` に置換（同パターン）。 |
| `.github/workflows/release.yml` | `SENTRY_DSN` / `VITE_SENTRY_DSN` 注入を撤去し、`OTLP_ENDPOINT` / `NEW_RELIC_LICENSE_KEY` 注入に置換。Rust のみ参照するため frontend 向け `VITE_*` の注入は不要（D10）。 |

### 2.3 変更（frontend）

| パス | 変更内容 |
|---|---|
| `src/types/settings.ts` | `performanceTelemetry: boolean` を追加。 |
| `src/components/panels/SettingsModal.tsx` | 「Send performance metrics（既定 on / 匿名）」トグル追加（既存 telemetry トグル群の隣）。`update_performance_telemetry` を invoke。 |
| `src/hooks/useTerminal.ts` ないし terminal mount 集計箇所 | mounted xterm count を Tauri コマンド `report_mounted_xterm_count(n)` で Rust に通知（D2 / Q1 確定）。 |
| frontend エラー転送（新規 `src/lib/telemetry.ts` 等） | `window.onerror` / `unhandledrejection` を捕捉し、Tauri コマンド `report_frontend_error(payload)` で Rust に転送（D10 / 下記）。Rust 側が OTel log signal として OTLP 送信する。frontend に OTel web SDK は導入しない、license key を WebView に露出させない。 |
| 既存 Sentry frontend 連携（`VITE_SENTRY_DSN` 利用箇所） | 撤去し、上記 Tauri コマンド転送経路へ置換。`VITE_*` 系の telemetry 環境変数も不要。 |

> **確定（D2: mounted xterm count の発行経路）**: frontend は「mount 数の変化」を Tauri コマンド `report_mounted_xterm_count(n)` で Rust に通知し、Rust 側 gauge で観測する。理由は (a) license key を WebView に露出させずに済む、(b) リソース観測（RSS/CPU/PTY）と同一の Rust gauge 系に揃えられ集約が単純、(c) `rust-first-logic` に沿う。requirements は「frontend 固有メトリクスを web SDK で発行」とも書くが、Rust 集約を採用（Q1 確定）。

### 2.4 レイヤー方針（依存方向の遵守）

`infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller`。

- OTel 型（`opentelemetry::*`）は **infrastructure と other/telemetry にのみ** 出現させる。
- domain / usecase の hot path は OTel 型を直接 import しない。計測が必要な箇所は other/telemetry の記録 API（OTel 非依存のシグネチャ）を呼ぶ。`other` は横断的関心事として全層から参照可（error.rs と同じ扱い）。
- これにより domain の「外部依存を持たない」規約を破らずに計測を差し込める。記録 API が内部で global provider を引くため、呼び出し側は provider を引数で受け取らない。

---

## 3. アーキテクチャと責務分割

### 3.1 初期化フロー（infrastructure/telemetry）

`init_telemetry()` を `lib.rs` setup で 1 回呼ぶ。戻り値は shutdown guard（`Option<TelemetryGuard>`）で、旧 `init_sentry()` と同様にアプリ生存期間保持する。tracer / meter に加え **LoggerProvider**（crash / エラー log 用）を構築し、panic hook（`infrastructure/telemetry/crash.rs`）を登録する。

判定ロジック（A3/A4/A6 準拠）。`BuildType` は D13 で 1 箇所に集約した enum:

```
enum BuildType { Dev, Release }

fn telemetry_active(build: BuildType, endpoint: &str, key: &str, perf_enabled: bool) -> bool {
    if endpoint.is_empty() || key.is_empty() {
        return false                      // 送信先未設定 → no-op（A4 / behavior Rule「no-op」）
    }
    match build {
        BuildType::Dev => true,           // dev は送信先設定済みなら有効
        BuildType::Release => perf_enabled, // release は既定 true（A6 / 完全匿名）。opt-out で false
    }
}
```

判定関数は `cfg!(debug_assertions)` を**直接参照しない**。呼び出し側（lib.rs setup）が `BuildType::current()`（内部で `cfg!(debug_assertions)`）を 1 度だけ評価して渡す。これにより判定関数は純粋関数となりユニットテスト可能（D13）。

`telemetry_active == false` の場合、global provider を **NoopMeterProvider / 未設定 tracer** のままにし、記録 API は素通りする。初期化失敗（exporter 構築エラー等）でも `None` を返し、アプリ起動は継続する（behavior「送信先設定が空ならアプリは正常起動する」）。

**起動ブロック回避（D12）**: provider / panic hook の構築は **同期** で行い lib.rs setup を待たせるが、OTLP exporter のネットワーク接続・TLS handshake・初回 export は **バックグラウンドスレッド（OTel SDK の batch / periodic reader が内部で spawn する tokio task）に委ねる**。ネットワーク不通・New Relic 側遅延が起動 path をブロックしないことを実装で保証する（requirements 要求事項 7「アプリ起動を妨げない」）。OTLP exporter 構築自体が失敗した場合は同期で握りつぶし `None` 返却。

OTLP exporter:
- protocol: Rust は OTLP/gRPC（requirements 計装基盤）。endpoint = New Relic OTLP（例 `https://otlp.nr-data.net`）。
- 認証: header `api-key: <NEW_RELIC_LICENSE_KEY>` を OTLP metadata に付与。
- export 周期: metric は periodic reader（例 10s）。span は batch exporter。

> **仮定（D3）**: New Relic は OTLP/gRPC・OTLP/HTTP 双方対応。Rust は gRPC（`opentelemetry-otlp` の tonic feature）、frontend web SDK は HTTP。endpoint/header 詳細は実装時に New Relic ドキュメントで確定する。

### 3.2 共通 resource attribute（resource.rs / 初期化時）

OTel `Resource` に一度だけ設定し全 signal へ自動付与（behavior「全メトリクスに比較軸が付与される」を構造的に保証）:

| attribute key | 値 | 出所 |
|---|---|---|
| `service.version`（app version） | `env!("CARGO_PKG_VERSION")` | Sentry release と同一 |
| `os.type` | `std::env::consts::OS` | native |
| `releash.build_type` | `BuildType::current().as_str()`（D13）| A3 |
| `service.name` | `"releash"` | 固定 |

ユーザーデータ（home/worktree path, ユーザー名等）は Resource に含めない。識別子の粒度別ポリシー（requirements 要求事項 5）:

| 識別子 | 付与 | 理由 |
|---|---|---|
| 個人特定 ID（氏名・メール・OS アカウント名・IP） | ✗ | 直接 PII |
| 永続デバイス ID（MAC ハッシュ・install UUID・OS マシン UUID） | ✗ | 起動・マシン横断追跡を可能にする |
| 起動セッション ID（プロセス単位 UUID） | ✗ | 1 起動内のイベントを束ね singling-out に近づく |
| trace ID / span ID（OTel 既定） | ✓ | 1 操作の親子関係追跡に必須。起動間で関連付け不能 |
| 環境属性（version/os/build/service の 4 つ） | ✓ | フィンガープリント力極小 |
| CPU model / 画面解像度 / locale 等 | ✗ | フィンガープリント材料の追加を避ける |

比較は app version（build）単位の集計で行い、同一ユーザー・同一マシン・同一起動の横断追跡はしない。

> **残存ベクトルの明示**: OTLP 送信時の source IP は New Relic ingest 側で観測され得る（トランスポート層メタデータ、アプリ制御外）。これは attribute ではなく、唯一の留保点。New Relic 側の IP 取り扱い設定で緩和を検討する（実装時）。

### 3.3 記録 API（other/telemetry）

hot path から呼ぶ薄い API。OTel 型を露出せず、attribute は型で限定する。

```rust
// status は enum に限定（任意文字列を attribute に流さない）
pub enum OpStatus { Success, Failure }

// span 計測ヘルパ（RAII or クロージャ）。Result 系 hot path の所要時間。
pub fn record_duration(metric: HotPathMetric, status: OpStatus, dur: Duration);

// streaming は histogram / counter に集約（D9: ingest 量を固定化）
pub fn record_payload_size(channel: PayloadChannel, bytes: usize); // histogram。PayloadChannel: TauriEvent | Ws
pub fn record_emit_interval(dur: Duration);                         // histogram
pub fn incr_dropped_frame();                                        // counter
pub fn incr_ws_reconnect();                                         // counter

// session save
pub fn record_session_save(dur: Duration, bytes: usize, status: OpStatus); // histogram + counter

// resource gauges（observe callback で定期取得）
pub fn observe_resource(rss: u64, cpu: f64, xterm: u64, pty: u64);
```

`HotPathMetric` は enum（`GitStatusScan` / `DiffStats` / `ReviewFileOpen` / `SessionList` / `SessionGetMeta` / `SessionGetPage` / `SessionLoadFull` / `SessionAppend` / `SessionPersistParts` / `SessionSaveFull`）。metric 名・attribute キーは enum→定数マッピングで固定し、呼び出し側が自由文字列を渡せないようにする（**ユーザーデータ非混入を型で担保** = behavior「attribute にユーザーデータが含まれない」）。

各 `record_*` は内部で「global provider が有効か」を確認し、無効なら即 return（no-op）。

### 3.4 操作 status の計測（behavior「成功/失敗を区別できる」）

Result を返す hot path（git status / diff / review open / session list/get/save）は、`Ok`→`Success` / `Err`→`Failure` を `OpStatus` に変換し、duration histogram の attribute と status counter の双方へ付与する。span を使う場合は span status（OK / Error）も設定。

計測パターン（例: git status）:

```
let start = Instant::now();
let result = inner_get_git_status(repo_path);
let status = if result.is_ok() { Success } else { Failure };
record_duration(HotPathMetric::GitStatusScan, status, start.elapsed());
result
```

失敗 attribute にエラーメッセージ本文は含めない（path / 本文混入防止）。status enum のみ。

### 3.5 streaming 計測（ws_bridge / session）

**ingest 量制御の方針（D9 / 案 A）**: streaming は高頻度 hot path のため、per-event の span / log は**出さない**。全て OTel **histogram / counter** に集約し、periodic reader（3.1 の 10s）で 1 metric data point として送る。これにより ingest 量は per-event 数に依存せず固定化される。

- **payload size**: `serialized.len()` を算出している既存箇所（Tauri event emit / WS broadcast）で **histogram** `record_payload_size` に投入。p50/p99/sum/count を 10s ごとに送る。
- **emit interval**: 直前 emit 時刻を `WsBroadcaster` 内に保持し差分を **histogram** `record_emit_interval` に投入。
- **dropped frame**: cumulative snapshot 置換（`send_stream_sync` で既存スナップショットを上書き＝送信前に捨てられた frame）を **counter** `incr_dropped_frame` で累積。
- **WS reconnect**: `ws_server/session.rs` の `set_sender(Some(_))`（接続確立）経路で **counter** `incr_ws_reconnect` で累積。

behavior「累積 snapshot は reconnect 時のみ送る方針を検証」: reconnect count と dropped frame count を別 counter で出し、New Relic 上で相関を見られるようにする（本 Issue は計測のみ。送信方針自体の変更は非スコープ）。

> **個別 frame trace の扱い**: 案 A では「ある 1 frame だけ巨大だった」等の個別異常は histogram の p99 で間接的に観測する。frame 単位の trace 追跡が必要になったら後から tail sampling（案 B）を追加導入できる。本 Issue では実装しない。

### 3.5.1 session storage 計測の対応表（D11）

behavior の「session list / get / save」は論理粒度。design では Rust メソッドごとに別 metric を建てて計測し、summary index / message paging の効果を区別できるようにする。

| behavior 論理名 | Rust メソッド | HotPathMetric | metric 名（業務名で固定） |
|---|---|---|---|
| session list | `list_metas` | `SessionList` | `releash.agent_session.list.duration` |
| session get（軽量・summary index） | `get_session_meta` | `SessionGetMeta` | `releash.agent_session.get_meta.duration` |
| session get（message paging） | `get_session_page` | `SessionGetPage` | `releash.agent_session.get_page.duration` |
| session get（full restore load） | `load_full_session_for_restore` | `SessionLoadFull` | `releash.agent_session.load_full.duration` |
| session save（高頻度 append） | `append_message` | `SessionAppend` | `releash.agent_session.append.duration` / `.bytes` |
| session save（低頻度 full save） | `save_full_session_for_migration_or_restore` | `SessionSaveFull` | `releash.agent_session.save_full.duration` / `.bytes` |

metric 名は **業務名で固定**し、Rust 関数名のリファクタリングと独立に履歴を維持する。behavior の Examples（list/get/save）は 6 metric の論理グループ名として扱う。

### 3.6 リソース観測（resource.rs, gauge）

OTel の observable gauge callback を 1 つ登録し、periodic export 時に下記を一括 observe:

| metric | 取得元 |
|---|---|
| プロセス RSS | `sysinfo` で自プロセス（pid）の memory |
| プロセス CPU | `sysinfo` で同 pid の cpu usage（RSS と同時取得） |
| active PTY count | `PtySessionRegistry` の len（snapshot len） |
| mounted xterm count | frontend からの最新報告値（D2: コマンド経由）／web SDK 直送（代替） |

`sysinfo::System` はプロセスを保持し refresh して取得。registry / xterm count は `AtomicU64` 等で最新値を保持し callback から読む。

### 3.7 frontend

frontend は OTel SDK を持たず、観測対象の **イベントを Tauri コマンドで Rust に転送する**だけに留める（D10 / rust-first-logic）。Rust 側が一括で OTLP 送信するため、license key を WebView に渡す必要がなくなり、`VITE_*` 系の telemetry 環境変数も不要。

- mounted xterm count: D2 / Q1 確定。mount/unmount で `report_mounted_xterm_count(n)` を invoke。Rust 側 gauge が観測。
- 未捕捉エラー: `window.onerror` / `unhandledrejection` を捕捉し、`report_frontend_error({ type, message, stack })` を invoke。Rust 側で path scrub を適用し OTel log signal として OTLP 送信。
- SettingsModal: opt-out トグル。release で既定 on（完全匿名のため）、変更即 invoke（behavior「設定から切り替えられる」）。
- 有効フラグの参照: 送信判定（送信先設定 / 同意フラグ）はすべて Rust 側で行う。frontend は転送するだけで、Rust が無効と判断したら no-op で破棄。

### 3.8 crash / エラー監視の統合（infrastructure/telemetry/crash.rs）

現状 `sentry_integration.rs` の機能を OTel log signal へ移植する（requirements 10〜12 / A7）。

| 既存 Sentry 要素 | 統合後 |
|---|---|
| `sentry::init` + `before_send` | OTel `LoggerProvider`（OTLP log exporter）＋ panic hook |
| panic 捕捉（sentry panic integration） | `std::panic::set_hook` で panic を捕捉し OTel log record（severity=ERROR, `exception.type` / `exception.message` / `exception.stacktrace` 属性）として emit |
| `scrub_home_dir` / `scrub_event` | crash.rs に移植。stacktrace の filename / path を `~` へ scrub してから emit（behavior「絶対パスが含まれない」） |
| `CRASH_REPORTING_ENABLED: AtomicBool` | telemetry 側 `CRASH_REPORTING_ENABLED` に移設。panic hook 内で参照し false なら emit skip |
| `environment`（dev/prod） | 共通 resource attribute の build_type に統合（3.2） |
| frontend `VITE_SENTRY_DSN` | frontend OTel web SDK の error log（3.7）|

crash 送信判定（A7）:

```
fn crash_emit_active(config) -> bool:
    if endpoint or key 空: return false            # no-op（behavior「送信先空なら crash も送らない」）
    return CRASH_REPORTING_ENABLED                  # 既定 true（既定 on / 送信する）。release/dev 共通
```

性能テレメトリ（`PERF_TELEMETRY_ENABLED`, 既定 on / A6）と crash（`CRASH_REPORTING_ENABLED`, 既定 on / A7）は**独立フラグ**。両者とも既定 on（完全匿名のため）で、それぞれ独立に opt-out 可能。endpoint/key 未設定ならどちらも no-op。

panic hook は OTel log provider が無効でも `default_hook`（標準のエラー出力）を必ず呼び、デバッグ可観測性を失わない。

---

## 4. データモデルまたは型

### 4.1 設定（config_models.rs）

```rust
fn default_performance_telemetry() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetrySection {
    #[serde(default = "default_crash_reporting")]
    pub crash_reporting: bool,
    #[serde(default = "default_performance_telemetry")]   // 既定 true = 送信する（A6 / 完全匿名のため）
    pub performance_telemetry: bool,
}
```

`Default` impl にも `performance_telemetry: true` を追加。`crash_reporting`（既定 true）と既定値を揃える。payload が完全匿名（要求事項 5: 永続識別子なし）であり、GDPR Recital 26 の匿名データに該当し同意要件の対象外であることが既定 on の根拠（A6）。

### 4.2 記録 API 型（other/telemetry/attributes.rs）

- `enum OpStatus { Success, Failure }`
- `enum HotPathMetric { GitStatusScan, DiffStats, ReviewFileOpen, SessionList, SessionGetMeta, SessionGetPage, SessionLoadFull, SessionAppend, SessionPersistParts, SessionSaveFull }`
- `enum PayloadChannel { TauriEvent, Ws }`
- metric 名定数（`releash.git.status_scan.duration` 等の dot 区切り命名）と attribute キー定数。

### 4.3 frontend 型（settings.ts）

`performanceTelemetry: boolean` を `AppSettings` に追加。Rust↔TS のフィールド対応は既存 `enableCrashReporting`↔`crash_reporting` と同じ変換層に従う。

---

## 5. 処理フロー

### 5.1 起動

1. `lib.rs` setup: `init_telemetry(&config)`（旧 `init_sentry()` を置換）。内部で tracer/meter/logger provider 構築＋ panic hook 登録。
2. `init_telemetry` が `telemetry_active`（性能）/ `crash_emit_active`（crash）を判定。無効なら該当 provider は noop、guard は `None`。
3. startup span 開始。`first window ready`（メインウィンドウ ready）・`first repo snapshot ready`（初回 snapshot 完了）で span event / 個別 span を記録し startup span を閉じる。
4. resource gauge callback を登録。

### 5.2 hot path 実行（dev, 設定済み）

1. 利用者が Git status / diff / review open / session 操作 / agent streaming を行う。
2. 各 hot path が record API を呼ぶ（duration + status / payload size 等）。
3. periodic reader / batch exporter が New Relic へ送信。
4. resource attribute（version/OS/build）が全 signal に自動付与済み。

### 5.3 release 送信同意フラグ切替

1. SettingsModal トグル（既定 on）→ ユーザーが off に切替 → `update_performance_telemetry(false)` invoke。
2. コマンドが設定保存＋ランタイム有効フラグ（`AtomicBool`）更新。
3. 以後の record API は no-op に切り替わる。再度 on に戻すと送信が再開する（behavior「切り替えられる」）。

> **仮定（D4: ランタイム切替の実現）**: 旧 Sentry の `CRASH_REPORTING_ENABLED: AtomicBool` を telemetry 側へ移設し、これと同型の `PERF_TELEMETRY_ENABLED: AtomicBool` を併設する。record / crash emit API がこれらを参照し、opt-out→送信即停止を再起動なしで実現する。endpoint 未設定時は AtomicBool に関わらず no-op。

### 5.4 送信先未設定

`OTLP_ENDPOINT` or license key 空 → `init_telemetry` が guard `None`、record API は AtomicBool 以前に「provider 無効」で素通り。アプリ・hot path は通常動作（behavior no-op Rule）。

---

## 6. エラー処理

- **初期化失敗**: exporter / provider 構築の `Result` は `init_telemetry` 内で握り、`tracing::warn!` ログのみ出し `None` 返却。起動は止めない。
- **export 失敗**: OTel SDK の batch/periodic exporter が内部でリトライ・ドロップする。アプリスレッドはブロックしない（非同期 export）。送信失敗をユーザーに伝播しない。
- **record API**: panic させない。provider 無効・lock 競合時は黙って no-op。hot path の本来の結果（Result）は計測の成否に関わらずそのまま返す（behavior「機能の結果は通常どおり得られる」）。
- **sysinfo 取得失敗**: 取得不能な値は observe をスキップ（その周期だけ欠測）。クラッシュさせない。
- **frontend 初期化失敗**: telemetry 初期化失敗は console warn のみ。UI 動作を妨げない。

---

## 7. テスト方針

`docs/architecture/TEST.md` と CLAUDE.md 規約に従う。外部送信（New Relic）はテストで実行しない。

### 7.1 Rust 単体テスト

- **設定**: `TelemetrySection` の serde default で `performance_telemetry == true`、`crash_reporting == true` を検証。round-trip（toml→struct→toml）。
- **有効判定 `telemetry_active`**: endpoint/key 空→false、`BuildType::Dev`＋設定済み→true、`BuildType::Release`＋既定（on）→true、`BuildType::Release`＋opt-out→false を網羅（behavior 各 Rule に対応）。判定関数は `cfg!(debug_assertions)` を直接参照せず、引数化（`build: BuildType`, `endpoint`, `key`, `perf_enabled: bool`）して純粋関数化する（D13）。
- **record API no-op**: provider 無効時に record_* を呼んでも panic せず副作用なしを確認。
- **OpStatus 変換**: `Ok`→Success / `Err`→Failure のマッピング。
- **attribute 非混入**: 記録 API のシグネチャが path/本文を受け取らないことをコンパイル時に担保（型テスト＝レビュー観点）。値 attribute の許可キー集合をテストで固定。
- **識別子非混入**: Resource / attribute の許可キー集合に (a) 個人特定 ID、(b) 永続デバイス/インストール ID、(c) 起動セッション ID、(d) CPU model / 解像度 / locale 等のフィンガープリント材料、が含まれないことをテストで固定（behavior 識別子 Rule）。共通 attribute は `service.version` / `os.type` / `releash.build_type` / `service.name` の 4 つのみであることを assert。trace/span ID は OTel 既定どおり付与される（許容）。
- **resource observer**: `sysinfo` で自プロセス RSS/CPU が取得でき、registry len と整合する観測値を返すことを検証（値域チェック）。
- **crash scrub**: 移植した `scrub_home_dir` の既存テスト（home→`~`置換／非 home は不変）を維持。crash log record 生成時に filename / path が scrub 済みであることを検証。
- **crash emit 判定 `crash_emit_active`**: endpoint/key 空→false、設定済み＋`crash_reporting=true`→true、opt-out→false を網羅（behavior crash Rule に対応）。判定関数を引数化してテスタブルにする。
- **panic hook**: provider 無効でも `default_hook` が呼ばれること、emit が skip されてもアプリがクラッシュ継続処理を妨げないこと。

### 7.2 hot path 計測の回帰

- git status / diff / session list/get/save の既存テストに「計測呼び出しが結果を変えない」回帰を加える（計測有無で戻り値同一）。

### 7.3 frontend テスト

- SettingsModal: performance トグルが `update_performance_telemetry` を正しい引数で invoke（Tauri API は `vi.mock`）。
- mounted xterm count 報告: mount/unmount で報告コマンドが呼ばれる（D2 採用時）。

### 7.4 受け入れ確認（手動 / behavior 対応）

- dev ビルド＋設定済みで全メトリクス（JS heap 除く）が New Relic 上で確認できる。
- attribute にユーザーデータが無い（送信データ目視）。
- release 既定（on）で送信される／opt-out で送信が停止する。
- 送信先未設定で正常起動。
- 各 metric に version/OS/build が付与。
- 性能予算ドキュメントが所定箇所に記載（8 章）。
- Rust panic / frontend 未捕捉エラーが New Relic Errors Inbox で確認でき、stacktrace の path が scrub 済み。
- `sentry`クレート / `SENTRY_DSN` / `VITE_SENTRY_DSN` が撤去されている。
- `tauri-plugin-aptabase` / `telemetry_enabled` フィールド / `app_started` aptabase 経路が撤去され、`app_started` 相当が OTel startup span / metric として New Relic 上で確認できる。

---

## 8. 性能予算ドキュメント化

> **確定（D5: 予算正本の配置）**: 正本は `docs/releash-performance-architecture-audit.md` の M0 セクション（既存 audit ドキュメント＝requirements の参照正本）に追記し、本 Spec からは参照に留める。audit が M0 の正本である以上、予算は audit に集約するのが一貫する（Q2 確定）。

固定する初期予算（requirements / behavior の表をそのまま正本化）:

| hot path | 予算 |
|---|---|
| app startup | orphan cleanup による visible startup block なし |
| repo snapshot | 中規模 repo で 200ms 台を目標、長い scan は stale snapshot を返す |
| file diff open | 小/中ファイル 500ms 未満、大ファイルは即 fallback 表示 |
| streaming event | 通常 frame payload 64KB 未満、累積 snapshot は reconnect 時のみ |
| session list | session 本文量に依存しない |
| terminal | worktree あたり mounted xterm 数に上限 |

本 Issue は計測と予算ドキュメント化まで。予算違反アラート / CI ゲートは非スコープ。

---

## 9. リスクと代替案

| リスク / 論点 | 内容 | 対応・代替案 |
|---|---|---|
| 依存追加の重量 | `opentelemetry` 系 + tonic（gRPC）はビルド時間・バイナリ増。 | 代替: OTLP/HTTP（`reqwest` 既存・rustls）で gRPC 依存を避ける。gRPC が New Relic 推奨だが HTTP も native 対応。実装時に比較。 |
| frontend への key 露出 | web SDK 直送は license key を WebView に渡す必要。 | **確定（D10）**: frontend は OTel SDK を持たず Tauri コマンド経由で Rust に転送する経路に統一。license key は Rust にのみ存在し WebView に渡さない。`VITE_*` 系の telemetry 環境変数は不要。 |
| dev での計測オーバーヘッド | hot path 毎の record が高頻度 streaming で負荷。 | record API を no-op 即 return で軽量化。streaming は histogram / counter 集約で per-event 出力を回避（3.5 / D9）。 |
| New Relic 無料枠（100 GB/月・uncompressed）超過 | streaming hot path を per-event span / log で出すと、配布規模が増えた際に枠を圧迫。超過時はプラットフォームアクセス停止（クレカ未登録時）または従量課金。 | **確定（D9 / 案 A）**: streaming は histogram / counter に集約し periodic reader（10s）で送る。ingest 量は per-event 数に依存せず固定。span は Result 系 hot path のみで使用し高頻度ループでは出さない。個別 frame trace が必要になれば後から tail sampling（案 B）を追加。 |
| 既存 telemetry 概念との混同 | `crash_reporting`（crash, 既定 on）と新 `performance_telemetry`（既定 on / 完全匿名）が並立。aptabase 経路と Sentry 経路は本 Issue で撤去し、観測先は New Relic に 1 本化。 | UI / 設定キーで明確にラベル分離。2 者は独立フラグ。両者は同一 SaaS（New Relic）で既定 on を揃え、それぞれ独立に opt-out 可能。`telemetry_enabled`（旧 aptabase 用）は config から削除し残存させない。 |
| build env 供給 | `OTLP_ENDPOINT` / `NEW_RELIC_LICENSE_KEY` を `env!` で要求すると未設定ビルドが壊れる。 | **確定（D7）**: 既存 `SENTRY_DSN` と同方式。`build.rs` が未設定時に空文字を `cargo:rustc-env` で供給し `env!` を必ず通す。release.yml でのみ実値注入。 |
| crash の OTel log 対応 | New Relic が OTLP log の exception 属性を Errors Inbox に正しく取り込むか実装時に要確認。 | 実装時に New Relic ドキュメントで属性マッピング（`exception.*` / severity）を確認。取り込まれない場合は span exception イベント経路を代替に検討。 |
| Sentry 撤去の影響 | `sentry` 撤去で release-health / session tracking 等の既存機能が失われる。 | 本 Issue は性能・メモリ・crash 監視への一本化が目的。release-health 相当が必要なら別途 New Relic 機能で代替検討（本 Issue 非スコープ）。 |

---

## 10. 仮定（本書で置いたもの）

- **D1**: OTel SDK 初期化は infrastructure、記録 API は other（横断）に配置。domain/usecase に OTel 型を漏らさない。
- **D2**: mounted xterm count は frontend→Tauri コマンド報告→Rust gauge を第一候補（key 非露出・集約単純・rust-first）。
- **D3**: New Relic は Rust=OTLP/gRPC、frontend=OTLP/HTTP。endpoint/header は実装時に公式ドキュメントで確定。
- **D4**: 送信同意フラグのランタイム即時切替（既定 on→opt-out / 再 on）は `PERF_TELEMETRY_ENABLED: AtomicBool`（Sentry パターン）で実現。
- **D5**: 性能予算の正本は `docs/releash-performance-architecture-audit.md` M0 に集約、本 Spec は参照（Q2 確定）。
- **D6**: crash / エラー監視を New Relic + OTel log signal へ統合。panic hook ＋ LoggerProvider で実現し、既存 path scrub を移植。crash は `crash_reporting`（既定 on）、性能は `performance_telemetry`（既定 on）の独立フラグ。両者とも完全匿名のため既定 on（A6/A7）、それぞれ独立に opt-out 可能。`sentry` と `tauri-plugin-aptabase` は撤去し、`app_started` 相当は OTel startup span / metric に統合（観測先 SaaS を New Relic に 1 本化）。
- **D7**: OTLP endpoint / license key の供給は既存 Sentry と同方式（`env!`＋`build.rs` 空 default＋release.yml 実値注入）に揃える（Q3 確定）。
- **D8**: 識別子の粒度別ポリシー（3.2 表）。付与しない: 個人特定 ID / 永続デバイス・インストール ID / 起動セッション ID / フィンガープリント材料（CPU model 等）。付与: OTel 既定の trace/span ID（1 操作スコープ）、限定 4 つの環境 attribute。比較は build（app version）単位の集計のみ（同一ユーザー・同一マシン・同一起動の横断追跡なし）。これにより GDPR Recital 26 の匿名データに該当し同意要件の対象外となるため、**perf=既定 on（A6）/ crash=既定 on（A7）** を採用。ユーザーはそれぞれ独立に opt-out 可能。唯一の残存ベクトルは送信時 source IP（アプリ制御外、New Relic 側設定で緩和検討）。
- **D9**: streaming hot path は histogram / counter に集約し periodic reader（10s）で送る（per-event span / log は出さない）。ingest 量を per-event 数に依存させず、New Relic 無料枠 100 GB/月の超過リスクを最小化。個別 frame trace が必要になれば後から tail sampling を追加（本 Issue 非スコープ）。
- **D10**: frontend は OTel SDK を持たず、観測対象イベント（未捕捉エラー / mounted xterm count）を Tauri コマンドで Rust に転送するだけに留める。Rust が一括で OTLP 送信する。license key を WebView に露出させず、frontend 用の `VITE_*` telemetry env も不要。rust-first-logic と整合。
- **D11**: session storage は Rust メソッドごとに 5 metric を別建てで計測（3.5.1 対応表）。metric 名は業務名で固定し、Rust 関数名のリファクタリングと独立に履歴を維持する。behavior の論理 3 粒度（list/get/save）は design で 5 metric にマッピング。
- **D12**: provider / panic hook 構築は同期で行うが、OTLP exporter のネットワーク接続・初回 export は OTel SDK 内部の tokio task でバックグラウンド化し、起動スレッドをブロックしない（requirements 要求事項 7 を実装で保証）。exporter 構築失敗は同期で握りつぶし `None` 返却。
- **D13**: `BuildType { Dev, Release }` を 1 箇所で定義し、`BuildType::current()` のみが `cfg!(debug_assertions)` を評価する。判定関数・resource attribute はこの enum を受け取って動作し、`cfg!` を直接参照しない。判定関数はユニットテスト可能な純粋関数となる。
- requirements A1〜A7、behavior の各 Rule を前提とする。

---

## 11. Open Questions

なし。Q1（mounted xterm count 経路）= D2 / Rust gauge 集約、Q2（性能予算の正本）= D5 / audit ドキュメント集約、Q3（build env 供給）= D7 / Sentry 同方式、で確定。crash 統合（Sentry→New Relic）・aptabase 撤去・性能テレメトリ既定値（perf=on / crash=on）・識別子粒度別ポリシー・ingest 量制御（histogram 集約）・frontend Rust 転送経路・session 5 metric・起動非ブロック・BuildType 集約はユーザー合意のもと requirements / behavior / 本書へ反映済み。
