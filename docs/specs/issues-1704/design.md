# Design

## The actual design

### Architecture

#### 判定規則を domain が所有し、gateway が外部信号を調停する

`src-tauri/src/domain/local_api_discovery/` を discovery 受理判定のドメイン境界とする。ここに discovery の内容を表す `DiscoveryContent`、プロセス観測の三値を表す `ProcessObservation`、接続先観測の三値を表す `ConnectionObservation`、受理・拒否と失敗種別を決める純粋な `DiscoveryAdmissionService` を置く。domain は reqwest、sysinfo、filesystem の型を受け取らず、transport error の実体も受け取らない。

`src-tauri/src/infrastructure/local_api/` は discovery file の読み込みと decode、sysinfo から得た開始時刻とプロセス一覧の有無、identity endpoint の応答 status または transport error、認証済み request の transport だけを扱う。これらの生信号を受理可否へ分類しない。

infrastructure の local API 用 reqwest client は redirect を追従しない。identity GET と以降の認証済み request の接続先を discovery が指す `127.0.0.1:<port>` に固定する。identity GET が 3xx を返した場合、gateway はその status を生の応答として domain へ渡し、domain は「応答はあるが 204 でない」接続先観測として `InstanceMismatch` に分類する。

`src-tauri/src/adaptor/gateway/local_api.rs` の `LocalApiClientGateway::discover` が生信号を集め、domain の値へ変換して `DiscoveryAdmissionService` に判定を委ねる。拒否理由を `LocalApiClientError` へ写し、CLI へ出す失敗文言を生成する。プロセス観測を受理した場合だけ identity GET 用の client を組み立て、接続先観測も受理した場合だけ呼び出し元へ返す。CLI 側（`src-tauri/src/cli/api_client.rs`）は gateway の入口を使い、discovery 失敗を既存の catch-all で `CliError::Other` へ落とす。

所有者を CLI 側へ移す設計（`ApiRequestError` に失敗種別を足す）は採らない。`ApiRequestError` の `Unavailable` は「local API へ到達できないので fallback してよい／アプリ起動を促す」という意味を持ち、R-004（`Unavailable` に対応する既存文言）と B-005（確認不成立時は command が失敗する）を同時に満たせなくなるためである。詳細は Alternatives Considered。

#### discovery 検証を内容・プロセス・接続先の順で行う

gateway は次の順で観測し、domain の判定へ渡す。

1. discovery file の読み込みと decode
2. 内容の検査（`port` / `token` / `instance_id` / `pid` / `process_started_at` の空・0 検査と PID 照合）
3. 接続先の確認（`/.well-known/releash-local-api/<instance_id>` への無認証 GET が 204 か）

接続先観測は次の三値とする。

- **確認できた（204）** — 接続を許可する。
- **確認した上で不成立（応答は得られたが 204 でない）** — `DiscoveryInstanceMismatch`。
- **確認できなかった（transport error）** — `DiscoveryUnreachable`。

「応答を得られたか」で分ける。connect 拒否、環境による遮断、timeout、protocol error はすべて「確認できなかった」に属する。B-001 の GIVEN は接続拒否だが、timeout も「確認できなかった」であり、同じ側へ寄せる。

2 段目では、内容の空・0検査とPID照合を分離する。内容の検査が不成立なら従来どおり `InvalidDiscovery` とする。PID照合は infrastructure の生信号から domain が構築する `ProcessObservation` の三値で分岐する。

- **プロセス情報を参照できない** — 不正・陳腐化とは区別した `ProcessInformationUnavailable` として拒否し、接続先確認へ進まない。
- **参照できたが対象プロセスが不在、または開始時刻が0** — 従来どおり `InvalidDiscovery` とする。
- **開始時刻を取得できた** — discoveryの `process_started_at` と一致すれば接続先確認へ進み、不一致なら従来どおり `InvalidDiscovery` とする。

`lookup_process_start_time` はまず対象PIDだけを参照する。開始時刻を取得できなかった場合に限り全プロセスを参照し、開始時刻の有無とプロセス一覧の有無を生値のまま gateway へ返す。参照不能か対象不在かの分類は `ProcessObservation::from_raw` が行う。`process_start_time` の既存 `Option<u64>` interface はlocal API起動処理のために残すため、起動時に自プロセスの開始時刻を解決できなければ失敗する既存動作は変わらない。

#### 受入条件の検証手段

- 内容、プロセス観測、接続先観測の判定規則は `domain/local_api_discovery/local_api_discovery_test.rs` の純粋な単体テストで検証する。
- discovery file、sysinfo、HTTP status の生信号取得は `infrastructure/local_api/{client,discovery}_test.rs` で検証する。
- B-001 の「接続不能」は、応答を得られない接続先を与える gateway 単体テストで検証する。「環境による遮断」は自動テストで再現せず、同じ「応答なし」へ変換される transport error で代表させる。
- B-002 は、identity path が 204 以外を返す偽サーバを用いた gateway 単体テストで検証する。
- B-003 / B-004 は既存の単体テストが維持対象として残る。
- B-005 の「token を含む request を送信しない」は、偽サーバが受信した request を検査する単体テストで検証する。
- B-006 は既存経路（discovery file 不在 → `Ok(None)` → `ApiRequestError::Unavailable` → `app_must_be_running_error`）の変更なしで満たす。
- B-007 は、gateway へプロセス参照の生信号を注入し、専用失敗が返ることと、偽サーバが接続を1件も受理しないことを検証する。三値の分類規則は domain の単体テストで個別に検証する。

### Interface

外部から観測できる契約は CLI の stderr 表示と終了コードである。追加するのは次の 3 分類で、いずれも `CliError::Other` として `error: <message>` 形式・終了コード 1 で出る（`cli/common.rs` の既存整形）。

- 接続先を確認できなかった場合:
  `local API の接続先 (127.0.0.1:<port>) へ接続できず、接続先を確認できませんでした。接続が拒否されたか、実行環境が loopback 接続を許可していません: <transport error>`
  discovery file のパスも「不正」「古い」という語も含めない（B-001）。
- 接続先が別インスタンスを指す場合:
  `local API discovery が別のインスタンスを指しているか、古くなっています (<discovery file path>)`
  「接続できなかった」という語を含めない（B-002）。
- プロセス情報を参照できない場合:
  `プロセス情報を参照できないため、local API の接続先を確認できませんでした`
  discovery file のパス、token、「不正」「古い」という語を含めず、接続先へ request を送信しない（B-007）。

変えない契約:

- discovery file 不在時の `この操作には Releash アプリの起動が必要です` と終了コード 1（R-004 / B-006）。
- 内容の検査、およびプロセス情報を参照できる場合のプロセス不在・開始時刻不一致時の `local API discovery file が不正または古いです (<path>)`。
- local API の wire 契約（`/.well-known/releash-local-api/<instance_id>` への無認証 GET と 204、以降の Bearer 認証）。server 側は変更しない。
- gateway の `LocalApiClientGateway::discover` は `Result<Option<Self>, LocalApiClientError>` を返し、`Ok(None)` は discovery file 不在のみを意味する。

内部境界として、infrastructure は `ProcessStartTimeLookup`（開始時刻とプロセス一覧の有無）および identity GET の status / transport error を返す。gateway はそれぞれを domain の `ProcessObservation` と `ConnectionObservation` へ変換する。

gateway の `LocalApiClientError` に `ProcessInformationUnavailable`、`DiscoveryUnreachable`（port と transport error を保持）、`DiscoveryInstanceMismatch`（discovery file path を保持）を追加する。`Unavailable` は従来どおり「確認済みの接続先に対する本 request の connect 失敗」に限って使い、意味を変えない。

### Data Model

discovery file のスキーマ（`LocalApiDiscovery` の `port` / `token` / `instance_id` / `pid` / `process_started_at`）は変更しない。追加も削除もないため versioning は不要で、旧版アプリが書いた discovery file を新版 CLI が読む組み合わせ、およびその逆も従来どおり成立する。

新たに永続化する record はない。

### Database

該当なし。

### UI/UX

該当なし。CLI の失敗表示は Interface に記載した。

### Algorithm

該当なし。

### Infra

該当なし。

## Alternatives Considered

- **接続不能を `ApiRequestError::Unavailable` へ分類する案。** 既存の「到達できない」分類に寄せれば CLI 側の変更が不要になる。採らない。`Unavailable` は `mutation` で `この操作には Releash アプリの起動が必要です` に落ちるため、R-004 が固定する discovery file 不在時の表示と同じ文言になり、B-001 の「接続先を確認できなかったことを理由として示す」を満たせない。加えて `read_with_fallback` が file fallback へ進んで command が成功してしまい、B-005 の「command は失敗する」に反する。
- **`Unavailable` に理由を持たせて `require_running` で文言を分ける案。** 上記の文言問題だけは解けるが、`read_with_fallback` の fallback 判定が `Unavailable` 全体に掛かるため B-005 を満たせない。
- **PID 照合を撤廃して接続先確認だけに一本化する案。** R-002（B-003 / B-004）を満たせない。

## Cross-cutting concerns

- **token の非送信順序（R-003 / B-005）。** gateway はプロセス観測を受理した後にだけ identity GET を送り、接続先確認が成立した後にだけ認証済み request を送る。`ProcessInformationUnavailable` は identity GET より前に、`DiscoveryUnreachable` / `DiscoveryInstanceMismatch` は認証済み request より前に返る。接続先確認そのものは従来どおり無認証 GET のままにする。
- **失敗表示に秘密を含めない。** 新しい 3 分類の文言には discovery の `token` を含めない。`ProcessInformationUnavailable` は固定文言だけを持ち、`DiscoveryUnreachable` が保持するのは port と transport error であり、identity 確認 URL に token は現れない。

## Risks

- **実 sandbox 下の挙動を自動テストで再現できない。** B-001 の実環境成立は cargo test では確認できず、手動検証に依存する。手動検証の環境が requirements の計測条件（codex-cli 0.149.1、macOS Darwin 25.5.0、`sandbox_mode = "workspace-write"`、network access 無効）から外れると、確認したことにならない。
- **プロセス一覧が空であることを参照不能の判定に使う。** 対応プラットフォームのmacOSでは正常な一覧が空になることはないという前提に立つ。参照不能と判定した場合は専用失敗として接続前に拒否するため、判定が安全側を外れても接続先への情報送信には進まない。

## 接続先確認の失敗分類に対する既存の手動検証

以下はPID参照不能分岐を追加する前に、B-001とB-006を2026-08-27に確認した記録である。記載するworktree diffとbinaryのSHA-256はその時点の検証対象を識別するものであり、現在の domain / gateway 境界とPID参照不能分岐を含むファイルを指すものではない。B-007は「受入条件の検証手段」に記載した注入可能な単体テストで検証する。

- codex-cli 0.149.1
- macOS 26.5.2（Darwin 25.5.0、arm64）
- `~/.codex/config.toml` の `sandbox_mode = "workspace-write"`
- codex-cli 0.149.1 の組み込み permission profile `:workspace` と `--sandbox-state-disable-network` を明示
- worktree: `/Volumes/siro33950_SSD_1/workspace/releash-worktrees/feat-issues-1704`
- Git HEAD: `092d3fb09109ff8147291d43cba86a293c370fa9`
- `client.rs` と `client_test.rs` の worktree diff SHA-256: `6014a76938c5d391561071ab4c16d29755c065c320dd7e12bb1f9b08ab812eae`
- 検証 binary: `src-tauri/target/debug/releash`
- binary SHA-256: `74084adcb0b7b204a69ff4aaac51f9eded0636669b48a81971df9d4120f379ea`
- binary mtime: `2026-08-27T19:22:12+0900`

`pnpm tauri:dev` がこの worktree の `releash v0.4.7` をコンパイルして上記 binary を起動したログを確認した。Releash (Dev) の local API は `/Users/siro33950/Library/Application Support/com.releash.app.dev/local-api.json` に discovery file を作成し、検証時は PID 65820、port 54400 で稼働していた。token は表示・記録していない。

### sandbox 内

実行 command:

```bash
codex sandbox \
  -P :workspace \
  --sandbox-state-disable-network \
  -C /Volumes/siro33950_SSD_1/workspace/releash-worktrees/feat-issues-1704 \
  -- env \
  RELEASH_DATA_DIR='/Users/siro33950/Library/Application Support/com.releash.app.dev' \
  /Volumes/siro33950_SSD_1/workspace/releash-worktrees/feat-issues-1704/src-tauri/target/debug/releash \
  workflow diagnostics
```

stdout は空。stderr 全文:

```text
error: local API の接続先 (127.0.0.1:54400) へ接続できず、接続先を確認できませんでした。接続が拒否されたか、実行環境が loopback 接続を許可していません: error sending request for url (http://127.0.0.1:54400/.well-known/releash-local-api/74f6fd2fb4eb4bbf9de6f24407ba0ba8)
```

終了コードは 1。stderr に `local API discovery file が不正または古いです` は含まれなかった。

### sandbox 外

同じ Releash (Dev) が稼働している間に、同じ binary と data directory で実行した。

```bash
RELEASH_DATA_DIR='/Users/siro33950/Library/Application Support/com.releash.app.dev' \
  /Volumes/siro33950_SSD_1/workspace/releash-worktrees/feat-issues-1704/src-tauri/target/debug/releash \
  workflow diagnostics
```

stdout は `info WFI000 [workflow=01_author-spec]: ビルトインワークフロー '01_author-spec'` から始まる診断結果を出力し、末尾は `0 error, 58 info` だった。stderr は空。終了コードは 0。

### アプリ停止時

Releash (Dev) の PID 65820 が停止したことを確認した。discovery file 不在の条件を明示するため、新規の空 data directory `/tmp/releash-1704-stopped-data.oJJgZG` を使った。

```bash
RELEASH_DATA_DIR=/tmp/releash-1704-stopped-data.oJJgZG \
  /Volumes/siro33950_SSD_1/workspace/releash-worktrees/feat-issues-1704/src-tauri/target/debug/releash \
  workflow output submit \
  --node-execution 00000000-0000-0000-0000-000000000000
```

stdout は空。stderr 全文:

```text
error: この操作には Releash アプリの起動が必要です
```

終了コードは 1。
