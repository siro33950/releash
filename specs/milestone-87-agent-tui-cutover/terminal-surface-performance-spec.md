# Terminal Surface 性能改善 Spec

作成日: 2026-08-06

状態: 調査完了・実装計画
関連: #1597 / `issue-1597-agent-session-vertical-slice.md` / `acceptance-contract.md`

## 1. 目的

TUI の入力、描画、Terminal Surface の永続化、Provider AgentSession の起動を、体感ではなく再現可能な性能契約で改善する。

本 Spec は #1597 の機能仕様を変更しない独立した性能 Spec である。Terminal Surface の所有、AgentSession lifecycle、Hook warning、Provider 選択、archive / delete / resume の仕様は #1597 と正本の契約に従う。

## 2. 結論

現在の遅さは xterm や PTY 自体の単一の性能限界ではない。Releash の現行経路には、Orca が計測によって除去した次のボトルネックが同時に存在する。

1. 通常入力が `write_terminal_surface` の request-response 完了待ちで直列化される。
2. PTY 出力を 4 KiB read ごとに backend event と Tauri Channel message にする。
3. renderer が 16 Ki characters を 1 回 `xterm.write()` するたび、次の chunk を `setTimeout(0)` へ送る。
4. 出力中は 250 ms ごとに全 scrollback から replay を再生成し、JSON 化、`sync_all()`、rename を行う。replay 生成中は backend-owned terminal model の mutex を保持する。
5. renderer queue の size 上限、parse 完了に基づく pacing、producer 側の backpressure がない。

特に 4 は、Orca が「履歴量に比例して PTY pump と入力 IPC を停止させる方式」として廃止した full snapshot checkpoint と同じ構造である。Orca の旧方式は 5 秒間隔だったのに対し、Releash は 250 ms 間隔である。

ただし、Releash には現時点で key-to-echo、event-loop drift、queue depth、checkpoint stall、Session 起動区間の性能 harness がない。このため、上記はコード上確定した待機・全量処理経路であり、各経路が実環境の遅延へ占める比率はまだ未計測である。実装は最初に測定可能な RED を作り、数値を確認してから Green にする。

## 3. 調査対象と revision

### Releash

- branch: `feature/1597-agent-session-tui`
- 調査対象: 現在の未コミット #1597 実装を含む worktree
- 主な対象:
  - `src/hooks/useTerminal.ts`
  - `src-tauri/src/adaptor/controller/command/terminal_surface/commands.rs`
  - `src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs`
  - `src-tauri/src/infrastructure/terminal/native_pty.rs`
  - `src-tauri/src/infrastructure/terminal/terminal_emulator.rs`
  - `src-tauri/src/infrastructure/terminal/checkpoint_scheduler.rs`
  - `src-tauri/src/usecase/agent_session/provider_agent_session_launch.rs`
  - `src-tauri/src/adaptor/gateway/agent_session/provider_agent_launch_gateway.rs`

### Orca

- repository: Releash と同じ親 directory の `orca`
- revision: `99b94a38eb53b9c90efe21a0d2609ee483b72ff3`
- revision date: 2026-08-02
- 過去の性能調査資料は、現行実装へ統合された performance initiative commit `e84a8ddec` の `notes/terminal-performance-initiative.md` と `notes/orca-performance-branch-guide.md` も参照した。

## 4. Releash の現在の経路

### 4.1 入力

```text
xterm.onData
  -> pendingInput へ結合
  -> invoke("write_terminal_surface")
  -> Tauri command の Result 応答を待つ
  -> 応答完了後だけ次の pendingInput を送る
  -> Rust registry lookup
  -> bounded native input queue
  -> writer thread が write_all + flush
```

`useTerminal.ts` の `inputWriteInFlight` は同時送信を 1 件に制限し、前回の `invoke` が resolve / reject するまで次の入力を送らない。IME の確定文字、Enter、次の入力が別 `onData` で到着すると、後続入力は前回の Tauri round trip 待ちになる。

Rust の `NativePtyRuntime` はすでに capacity 1024 の ordered queue と writer thread を持ち、writer thread 側で連続入力を結合する。したがって hot path の frontend request-response pacing は、native writer の順序保証に追加された別の待機である。

### 4.2 出力と描画

```text
PTY reader (4 KiB)
  -> UTF-8 decode / OSC filter
  -> avt model へ apply
  -> sequence 更新
  -> global broadcast event 1 件
  -> attachment task
  -> Tauri Channel message 1 件
  -> Promise chain
  -> renderer queue
  -> xterm.write(最大 16 Ki characters)
  -> setTimeout(0)
  -> 次の 16 Ki characters
```

確認できた特性は次のとおり。

- backend read buffer は 4 KiB であり、backend 側の時間・byte batch がない。
- `TerminalSurfaceEventHub` は全 Terminal Surface 共通の capacity 256 の broadcast channel である。attachment は全 Terminal Surface の event を受信してから対象 `session_key` だけを選ぶ。
- Tauri Channel への送信には renderer の parse 完了を表す ACK がない。
- frontend の `pendingLiveOutput` に size 上限がない。
- 16 Ki characters ごとに必ず `setTimeout(0)` を挟む。Chromium の nested timer clamp が発生すると、parse 時間を除いても約 4 ms / 16 Ki characters、約 4 Mi characters/s の構造的な上限になる。ASCII fixture では約 4 MiB/s に等しい。
- Channel message 自体は 4 KiB 単位のため、xterm queue より前に IPC callback と Promise microtask が大量発生する。
- event receiver が lag すると sequence resync のため full snapshot を作る。correctness は回復するが、出力負荷時に full replay 生成が重なる可能性がある。

### 4.3 headless state と checkpoint

Releash は `avt::Vt` を backend-owned terminal model として保持している。この境界自体は正しい。問題は保存方式である。

現在の dirty checkpoint は出力中 250 ms ごとに次を行う。

1. terminal model の mutex を取得する。
2. bounded scrollback 1,000 行を含む replay ANSI を全量生成する。
3. registry へ full checkpoint を反映する。
4. full JSON を一時 file へ書く。
5. `sync_all()` する。
6. rename する。

worker thread は renderer thread ではないが、full replay 生成中は PTY output reader が同じ terminal model へ `apply` できない。保存 payload と CPU 時間も直近 delta ではなく保持履歴量に依存する。

### 4.4 Provider AgentSession 起動

新規 Session は次を直列に完了した後で `create_provider_agent_session` が返る。

```text
Provider availability
  -> per-session operation lock
  -> Provider lifecycle arm + AgentSession create の durable SQLite commit
  -> Session 固有 launch / Hook file の materialize
  -> Terminal checkpoint file の read
  -> CLI alias / child environment の準備
  -> portable-pty open + provider process spawn
  -> output reader start
  -> command response
  -> center selection
  -> get_terminal_surface
  -> attach_terminal_surface
  -> initial snapshot の xterm parse
```

Hook health の永続化は spawn 後の background task であり、現在の起動応答を block しない。Session create と Provider lifecycle arm は 1 atomic commit に統合済みであり、durability を保つために無条件で外してよい処理ではない。

現在は各区間の duration、PTY spawn から first byte、first byte から xterm parse / paint までの計測がない。したがって Session 起動については、file I/O、SQLite commit、PTY spawn、provider CLI のどれが支配的かは未確定である。

## 5. Orca の性能契約

### 5.1 現在の実行可能な gate

Orca の `.github/workflows/terminal-perf.yml` は daily / manual で terminal scale perf を実行し、machine-readable report を保存する。workflow 自身が目的を「利用者が typing lag を報告する前に regression を検出する」と記載している。

`config/scripts/check-terminal-perf-report-budgets.mjs` の現在値は次のとおり。

| 指標 | gate |
| --- | ---: |
| key latency median | 75 ms 以下 |
| key latency worst | 300 ms 以下 |
| revisit latency | 300 ms 以下 |
| unloaded timer drift | 150 ms 以下 |
| injected load timer drift | 2,500 ms 以下 |
| scroll latency | 150 ms 以下 |
| restore latency | 1,000 ms 以下 |
| renderer current queue | 2,097,152 UTF-16 code units 以下 |
| renderer peak queue | 2,097,152 UTF-16 code units 以下 |
| renderer dropped backlog | 0 |

Releash は現在 1 つの表示中 Terminal Surface を対象とし、Orca の 5 pane、17 pressure pane、100 pane 規模を要求していない。このため、最初の Releash gate は single active Terminal Surface に上表の strict budget を適用する。複数 pane 向けの 3,000 ms worst tolerance は採用しない。

### 5.2 Orca 内の gate の不整合

Orca には次の複数 tier が残っている。

- 古い `terminal-typing-latency.spec.ts` は median 250 ms / worst 1,000 ms の smoke budget。
- `artificial-opencode-terminal-load.spec.ts` の unloaded baseline は median 75 ms / worst 300 ms / timer drift 250 ms。
- 同 test の injected multi-pane load は median 75 ms を維持する一方、CI shard の CPU 変動を許容して worst 3,000 ms / drift 2,500 ms。
- 同 test の renderer queue は 5,242,880 UTF-16 code units だが、daily report checker は 2,097,152 UTF-16 code units。
- `config/reliability-gates.jsonc` の `terminal-performance.input-throughput` は、実際には daily workflow と report checker が存在する現在の source と矛盾して「executable coverage 未接続」と記載している。

本 Spec は、古い smoke 値や緩い multi-pane 値ではなく、現在の daily report checker の strict 値を Releash の基準にする。Orca 側の registry metadata の不整合は Releash へ持ち込まない。

### 5.3 throughput の扱い

Orca の current report checker は output MB/s の絶対下限を持たない。10 MiB agent-TUI fixture、DSR latency、queue pressure、drop、event-loop stall を組み合わせて回帰を判断する。

過去の Orca 計測値は比較資料としてのみ使用する。

| 状態 | agent-TUI throughput | DSR under load |
| --- | ---: | ---: |
| v1.4.91 baseline | 2.0 MiB/s | p50 134 ms / p99 292 ms |
| bare headless xterm | 約 103 MiB/s | 対象外 |
| daemon Session ingest | 約 103 MiB/s | 対象外 |
| 3 つの hot-path 修正後の dev | 11.5 MiB/s | p50 18.8 ms / p99 24.9 ms |
| v1.4.121-rc.0 production | 11.2 MiB/s | p50 18.6 ms / p99 29.7 ms |
| current-build 10 MiB audit | 10.14 MiB/s | p50 6.72 ms / p90 9.86 ms / p99 13.87 ms |

この履歴は、terminal emulator 自体ではなく scheduler、main-process hot path、batch window が end-to-end の支配要因だったことを示す。Releash でも xterm や `avt` の置換を先に行う根拠はない。

### 5.4 Session / PTY 起動の要求

Orca は `ORCA_PTY_SPAWN_TIMING=1` で次の区間を計測する。

- `preflight`
- `auth`
- `host_env`
- `options`
- `provider_spawn`
- `total`

一方、調査した current source、perf workflow、reliability gate には PTY / AgentSession 起動時間の合否閾値がない。過去の Windows 計測 artifact にある warm spawn 約 91–152 ms、end-to-visible 約 497 ms は観測値であり、Provider CLI ready timeでも正式な requirement でもない。

したがって Releash の Session 起動へ継承できる Orca の絶対 budget はない。継承するのは「単一の total ではなく区間別に計測し、同一 machine、同一 build 条件で比較する」という方法である。

## 6. Orca が採用した構造

### 6.1 入力

- 通常入力は `ipcRenderer.send('pty:write')` / `ipcMain.on('pty:write')` の one-way path。
- Ctrl+C / Escape 等、到達確認自体に意味がある操作だけ `writeAccepted` を使う。
- 4,096 UTF-16 code unit 以下の連続した小入力を同一 PTY ごとに結合する。
- hot small input は UTF-8 byte 数の事前計測を省く。
- backlog が残る場合だけ event loop へ yield する。
- write 失敗は応答待ちではなく `pty:writeUnavailable` で out-of-band 通知する。

### 6.2 output transport

- daemon と main の batch window は各 2 ms。
- main から renderer へは最大 16 Ki characters の chunk。
- input 直後 100 ms の小さな echo / ANSI redraw は bulk batch を bypass できる。
- renderer in-flight は per PTY / global の character-count budget を持つ。
- producer pause / resume と parse 後 ACK で backlog を制限する。
- hidden pane は backend model を正本とし、drop 後に snapshot から回復する。

### 6.3 renderer scheduler

- 16 Ki characters の chunk。
- active / high priority は 1 drain あたり最大 8 write。
- 1 drain の仕事時間は最大 8 ms。
- xterm の parsed callback を次の high-priority drain の clock にする。
- zero-delay continuation は `MessageChannel` を使い、nested `setTimeout(0)` の約 4 ms clamp を避ける。
- background は 16 ms cadence に落として active input / paint を守る。
- queue は character-count 上限を持ち、drop / restore / ACK を全経路で accounting する。

### 6.4 checkpoint

Orca は full snapshot を通常 tick から外した。

- dirty tick は 5 秒ごとに pending output record の incremental log を append する。
- pending record は 2 MiB で bounded。
- full snapshot は clean disconnect、pending overflow、log cap 等に限定する。
- streaming 中の full snapshot は 45 秒 cooldown で連発を防ぐ。
- full checkpoint file write は async の temp + rename。
- crash restore は base snapshot + ordered incremental log で再構築する。

### 6.5 Orca が実測で除去したもの

- renderer の fixed timer nap は background throughput を約 1.9 MiB/s に制限していた。
- parse callback を clock にすると isolated renderer ceiling は 27 MiB/s から 117.6 MiB/s へ上がった。
- main の `onPtyData` は flood 中に event loop の約 93% を使用していた。原因は retained tail への O(tail) redraw / scan。
- bounded redraw window と keyword-prefiltered / throttled scan を入れ、dev agent-TUI は 0.7 MiB/s から 11.5 MiB/s、load DSR p50 は 161 ms から 18.8 ms へ改善した。
- daemon / main の batch window を 8 ms から 2 ms へ短縮すると、dev load DSR p50 は約 19 ms から 8 ms へ改善した。
- full terminal snapshot の周期保存は履歴量に比例して PTY pump と input IPC を止めるため、incremental checkpoint へ置換された。
- main hop を消す UtilityProcess / MessagePort 案は、実測した追加 hop が idle 約 0.5 ms だったため優先されなかった。

## 7. Releash と Orca の差分

| 境界 | Releash 現在 | Orca 現在 | Releash 方針 |
| --- | --- | --- | --- |
| 通常入力 | invoke 応答待ちで 1 件ずつ | one-way、small input coalesce | ordinary input を response-paced にしない |
| 入力失敗 | invoke reject | out-of-band unavailable | ordered ingress + out-of-band error |
| backend read | 4 KiB ごとに publish | 2 ms batch、最大 16 Ki characters delivery | 2 ms / 最大 16 Ki characters を初期値にする |
| interactive echo | bulk と同じ | input 直後は bypass | latency-sensitive bypass を持つ |
| renderer drain | 16 Ki characters ごとに timer | parse-clock + MessageChannel | Orca scheduler の最小 subset を採用 |
| renderer queue | unbounded | character-count bounded | 2,097,152 UTF-16 code units gate と pressure metric |
| backpressure | なし | parse ACK + producer flow | 最初は snapshot resync を利用し、必要時だけ ACK を追加 |
| durable state | 250 ms ごとに full replay + fsync | incremental tick + rare full | incremental journal へ移行 |
| headless emulator | Rust `avt` | daemon `xterm/headless` | `avt` を維持し、計測で不足時だけ再検討 |
| daemon | なし、app process と同居 | daemon + Unix socket | 性能改善のためには追加しない |
| spawn timing | なし | phase timing あり | 同等以上の phase timing を追加 |
| spawn budget | なし | なし | baseline 後に別途固定 |

## 8. 採用しない Orca の複雑性

次は Orca の remote / daemon / multi-pane 要求に必要だが、現在の Releash の性能改善には導入しない。

- persistent daemon と Unix socket
- remote runtime / SSH relay / mobile driver の ownership 調停
- hidden pane 17–100 個を前提にした priority policy
- viewport claim、mobile resize hold、window graph
- dropped DSR / DA / OSC query byte の salvage
- provider ごとの複雑な ACK self-heal
- headless emulator の置換

Releash には backend-owned model、sequence、snapshot resync がすでにある。まず次の最小構成で strict gate を満たす。

1. ordinary input の応答待ちを外す。
2. backend output を 2 ms / 最大 16 Ki characters で coalesce する。
3. renderer を parse-clocked / cooperative drain にする。
4. renderer queue を 2,097,152 UTF-16 code units で bound し、overflow 時は delta を捨てて backend snapshot から 1 回 resync する。
5. periodic persistence を incremental にする。

この構成で acceptance workload 中に queue overflow / resync が発生する場合だけ、parse ACK と producer pause / resume を追加する。測定前に Orca の全 flow-control state machine を移植しない。

## 9. 性能契約

### 9.1 共通条件

- packaged または production-equivalent build を性能判定の正本にする。
- A/B は同じ machine、同じ power state、同じ fixture、他の高負荷処理なしで行う。
- warmup 後に測定する。cold start は別行として残す。
- report は JSON artifact と human-readable summary の両方を出す。
- path、入力本文、Session ID 等の user data を telemetry / artifact に含めない。
- 10 MiB の agent-TUI-shaped ANSI fixture を使う。plain ASCII だけで性能判定しない。

### 9.2 single active Terminal Surface

Orca daily report checker の strict 値を継承する。

- 16 key sample の key-to-visible-marker median: 75 ms 以下
- worst: 300 ms 以下
- revisit latency: 300 ms 以下
- unloaded renderer event-loop drift: 150 ms 以下
- injected-load renderer event-loop drift: 2,500 ms 以下
- renderer current / peak queued code units: 2,097,152 UTF-16 code units 以下
- dropped backlog: 0
- scroll response: 150 ms 以下
- restore: 1,000 ms 以下

さらに、現在報告されている「Terminal が遅い間に他 UI も固まる」症状と、Releash の snapshot recovery 方式を対象に次を追加する。これらは Orca の current report checker に存在する値ではなく、Releash 固有の gate である。

- acceptance workload 中の snapshot resync: 0
- 10 MiB fixture 中に 100 ms を超える renderer long stall: 0

throughput MiB/s は必ず記録するが、Orca の current executable gate に絶対下限がないため、Orca の 10 MiB/s 前後の観測値を Releash の合否値として偽装しない。初回 Releash baseline を保存し、上記 responsiveness gate を満たした Green build の throughput を回帰 baseline に固定する。

### 9.3 IME

- preedit は PTY へ送らない。
- commit text は exactly once。
- commit text と直後の Enter の byte order を保存する。
- IME commit と Enter の各 marker latency を通常 key と同じ report に記録する。
- 性能改善のために composition correctness test を変更しない。

### 9.4 他 UI の応答

Terminal load 中も同じ renderer event loop 上の非 Terminal UI 操作を行う。

- UI heartbeat の drift: 150 ms 以下
- sidebar / workspace selection の input-to-state worst: 300 ms 以下

これは「Terminal が遅い間に他 UI も固まる」という回帰を、Terminal 内の marker だけで見逃さないための条件である。

### 9.5 checkpoint

- 通常の dirty tick で full replay を生成しない。
- 通常 tick の保存量は保持 scrollback 全量ではなく、前回 durable point 以後の delta に比例する。
- 1,000 行履歴の有無で通常 tick の PTY pump gap が履歴量比例に増えない。
- clean shutdown、強制終了相当、incremental log cap 後の復元で、最終 durable sequence まで同じ visible state、cursor、rows / cols を再構築する。
- old checkpoint format の migration は実施しない。

### 9.6 Session 起動

次を別々に記録する。

- frontend request -> command ingress
- availability / operation lock
- durable create commit
- launch / Hook file materialize
- checkpoint lookup
- child environment preparation
- PTY open / provider process spawn
- output reader ready
- first provider byte
- first xterm parsed callback
- first paint

Orca に起動の絶対 budget がないため、起動時間そのものへ合否閾値は置かない。2026-08-07 の同一production-equivalent buildによる各30 warm runは次のとおり。

| 対象 | total p50 / p95 / max | Provider first byte p50 / p95 / max | first xterm parsed p50 / p95 / max | first paint p50 / p95 / max | Releash-owned phase p50合計 |
| --- | ---: | ---: | ---: | ---: | ---: |
| deterministic fixture | 45 / 48 / 53 ms | 3.79 / 4.02 / 4.03 ms | 21.5 / 23 / 28 ms | 30.5 / 37 / 38 ms | 5.01 ms |
| Claude | 289 / 292 / 299 ms | 264.23 / 269.98 / 271.19 ms | 277 / 282 / 284 ms | 277 / 283 / 284 ms | 6.20 ms |
| Codex | 164 / 184 / 184 ms | 135.81 / 138.24 / 138.95 ms | 146 / 150 / 150 ms | 155 / 162 / 163 ms | 4.80 ms |

全phaseのp50 / p95 / maxは `terminal-launch-performance-baseline.json` と `terminal-launch-provider-observations.json` を正本とする。Releash-owned区間はいずれも支配的ではなく、実Providerではfirst byteが支配している。この結果からdurable commit、Session固有Hook file、checkpoint lookupを削る変更は行わず、Provider内部最適化やcreate accepted/runtime readyの契約変更も行わない。

## 10. Red-Green-Refactor 実装計画

各 cycle は、失敗する新規 test / performance gate、最小の Green、構造を単純化する Refactor の順で独立して完了させる。複数 cycle を一度に Green にしない。

### Cycle 0: measurement contract

Red:

- Releash には key latency、event-loop drift、queue depth、drop / resync、checkpoint stall、spawn phase を含む report command が存在せず、performance contract を実行できない。
- 既存 build に strict budget を適用した baseline artifact を保存する。

Green:

- deterministic PTY echo fixture と agent-TUI-shaped output fixture を追加する。
- production-equivalent Tauri E2E で 9 章の値を JSON に出す。
- backend に input ingress、PTY write、output read、model apply、event publish、checkpoint、spawn phase の duration probe を置く。
- frontend に Channel receive、queue、xterm parsed callback、event-loop drift、first paint の probe を置く。

Refactor:

- 計測 clock / reporter を production logic から分離する。
- test 用 probe は本文や path を保持しない集計値だけを公開する。

### Cycle 1: ordinary input を response-paced にしない

Red:

- first write の Promise を未解決にしたまま、IME commit、Enter、次 key が backend ordered ingress へ到達することを要求する test を追加する。
- queue overflow / writer unavailable が out-of-band error として 1 回通知される test を追加する。

Green:

- ordinary input は Tauri command response を次入力の送信条件にしない。
- adaptor 境界に owner ごとの ordered / bounded ingress を置く。
- native writer の既存 coalesce と capacity を再利用する。
- 到達確認が外部仕様になる特殊操作だけ acknowledged path を許す。

Refactor:

- `useTerminal` 内の `pendingInput` / `inputWriteInFlight` を削除する。
- input transport と error notification を Terminal Surface adaptor の明示的な責務にする。

### Cycle 2: backend output batching

Red:

- 4 KiB の断片入力を連続投入し、内容 / sequence を変えず 2 ms window、最大 16 Ki characters へ coalesce される test を追加する。
- input 直後の小さい echo / ANSI redraw が 2 ms batch を待たない test を追加する。
- 別 owner の output が active attachment の順序や resync を乱さない test を追加する。

Green:

- UTF-8 decode と backend model apply 後、wire event だけを 2 ms / 最大 16 Ki characters で batch する。
- sequence は raw output の順序を維持する。
- active owner の latency-sensitive output を bounded budget 内で即時 publish する。
- attachment routing を owner 単位にし、無関係な Terminal Surface event を全 attachment が読む構造を解消する。

Refactor:

- batching は domain ではなく infrastructure / protocol adaptor に置く。
- `TerminalSurfaceEvent` の correctness と wire delivery policy を分離する。

### Cycle 3: renderer parse-clocked scheduler

Red:

- 10 MiB fixture で 16 Ki characters ごとの nested timer clamp に依存しないことを検証する。
- 1 drain が 8 write または 8 ms を超えないことを fake clock で検証する。
- xterm parsed callback 前に処理済みと数えない test を追加する。
- queue 2,097,152 UTF-16 code units、drop / resync 0 の E2E gate を追加する。

Green:

- zero-delay continuation を `MessageChannel` にする。
- high priority は最大 8 x 16 Ki characters、最大 8 ms で cooperative drain する。
- xterm parsed callback が次の active drain を進める。
- queue pressure を UTF-16 code unit で計測する。
- cap 超過時は中間 delta を保持し続けず、既存 sequence / backend snapshot から単発 resync する。

Refactor:

- scheduler を `useTerminal` の effect local closure から独立した pure state machine へ抽出する。
- workspace Terminal と AgentSession TUI が同じ scheduler を使う。

### Cycle 4: incremental durable checkpoint

Red:

- continuous output の通常 dirty tick が full snapshot materialization を呼ばない test を追加する。
- 0 行と 1,000 行の履歴で同じ delta を保存したとき、通常 tick の保存量が履歴全量へ比例しない test を追加する。
- output / resize / clear と sequence を含む crash-restore test を追加する。
- incremental log cap 到達後の compaction と復元を検証する。

Green:

- base checkpoint と ordered incremental output / resize journal を保存正本にする。
- current 250 ms durability cadence は最初の Green で維持し、full replay 生成だけを通常 tick から外す。
- full compaction は clean stop、journal cap、明示 flush 等へ限定する。
- repeated full compaction に cooldown を設ける。
- temp + fsync + rename による durable boundary を維持する。

Refactor:

- emulator、increment collector、file store、compaction policy を分離する。
- checkpoint worker が terminal model mutex を保持したまま disk I/O しないことを構造 test で固定する。

250 ms の incremental fsync 自体が計測上の支配要因なら、Orca の 5 秒 tick を参考に durability window の変更を別途合意する。最初から無断で 5 秒へ緩和しない。

### Cycle 5: end-to-end flow control の要否判定

Cycle 1–4 後に 9 章の gate を再実行する。

- peak queue 2,097,152 UTF-16 code units 以下、drop / resync 0、latency budget 内なら、Orca の full ACK state machine は追加しない。
- queue cap または resync が残る場合だけ、xterm parsed callback 後の cumulative ACK と backend producer pause / resume を Red-Green-Refactor で追加する。
- ACK を追加する場合も Terminal Surface sequence と snapshot resync を recovery の正本とし、wall-clock timeout だけで byte を消費済み扱いしない。

### Cycle 6: Session 起動

Red:

- 9.6 の全 phase が report に存在し、total と整合する test を追加する。
- deterministic provider fixture で 30 warm run を出す。

Green 1 — 計測:

- Orca と同様の opt-in phase timing を追加する。
- Provider CLI 内部時間と Releash-owned pre-spawn / attach 時間を分ける。
- Codex / Claude の実 process run は観測、deterministic fixture は回帰 gate とする。

Green 2 — 計測結果に応じた限定修正:

- launch file materialize が支配的なら、不変 file だけを app 初期化へ移し、Session 固有 file は維持する。
- alias / child env が支配的なら、不変な解決結果を app-owned state として再利用し、wrapper 内容一致時の再書込を避ける。
- checkpoint lookup / initial replay が支配的なら、新規 owner と restore owner を backend の事実で分け、不要な read / replay を行わない。
- durable commit が支配的でも、commit を省略または spawn 後へ移動しない。SQLite transaction 自体を計測して改善する。
- provider process spawn / provider first byte が支配的なら Releash の transport 最適化と区別し、Provider 固有時間として報告する。
- command response 前の直列 contract 自体を変える必要がある場合は、Session create accepted と runtime ready / failed の外部仕様を別途合意してから行う。

Refactor:

- phase timing を launch usecase の business decision に混ぜない。
- cache を追加する場合は app-owned immutable initialization state に限定し、Session lifecycle state を frontend cache にしない。

## 11. 実装順序

1. Cycle 0: 計測と baseline
2. Cycle 1: input response pacing の除去
3. Cycle 2: backend batching / owner routing
4. Cycle 3: renderer scheduler
5. Cycle 4: incremental checkpoint
6. Cycle 5: ACK / producer flow-control の要否判定
7. Cycle 6: Session 起動の計測と支配区間だけの改善

Cycle 0 の結果で checkpoint stall が key latency / UI freeze の直接支配要因と確認された場合、Cycle 4 を Cycle 1 の直後へ繰り上げる。順序変更は測定 artifact を根拠に行う。

## 12. 完了条件

- 9 章の strict gate が production-equivalent build で Green。
- IME commit、Enter、通常 key の順序と exactly-once が Green。
- 10 MiB agent-TUI fixture 中も非 Terminal UI が Green。
- ordinary input が Tauri response completion に pace されない。
- output delivery が 4 KiB ごとの無制御 IPC ではない。
- renderer が nested `setTimeout(0)` に throughput を支配されない。
- ordinary dirty checkpoint が full replay / full JSON を生成しない。
- durable restore と live/reload 相当の Terminal Surface correctness が維持される。
- Session 起動 phase の p50 / p95 / max が report され、Releash-owned と provider-owned が分離される。
- Orca の remote / daemon 向け複雑性を追加していない、または追加した各 mechanism に budget failure の証拠がある。
- 各 cycle に RED の失敗証拠、Green の通過証拠、Refactor 後の再通過証拠がある。
- `pnpm lint`、`pnpm test`、`pnpm build`、`cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test`、関連 integration / performance gate が通る。

## 13. 非スコープ

- daemon 化
- remote / mobile terminal
- 複数 pane UI の新設
- AgentSession lifecycle / archive / delete / resume の変更
- Hook warning / Provider availability の仕様変更
- Provider CLI 自体の内部最適化
- terminal emulator library の置換
- old terminal checkpoint format の migration
- performance と無関係な UI redesign

## 14. 主な risk と防止策

| risk | 防止策 |
| --- | --- |
| one-way input で失敗が見えなくなる | bounded ingress と out-of-band unavailable event |
| async command arrival で順序が崩れる | owner ごとの ordered ingress と sequence test |
| batch が echo を遅らせる | input 直後の bounded interactive bypass |
| cooperative drain が throughput を落とす | 8 write / 8 ms と parse-clock、10 MiB fixture |
| queue cap で画面が欠ける | backend model + sequence の単発 snapshot resync |
| resync storm | 1 回の in-flight resync guard、acceptance では resync 0 |
| incremental log の欠落 / 重複 | monotonic sequence、base + delta の crash-restore test |
| compaction が再び hot path を止める | 通常 tick から分離、cap / cooldown、stall probe |
| 起動高速化が durability を壊す | durable commit を先行する現契約を維持し、区間計測後に限定修正 |
| benchmark だけが速くなる | agent-TUI ANSI、IME、他 UI interaction、production-equivalent build を同時に gate |

## 15. 参考 source

### Orca current source

- `.github/workflows/terminal-perf.yml`
- `config/scripts/check-terminal-perf-report-budgets.mjs`
- `config/reliability-gates.jsonc`
- `tests/e2e/artificial-opencode-terminal-load.spec.ts`
- `tests/e2e/terminal-typing-latency.spec.ts`
- `src/preload/index.ts`
- `src/main/ipc/pty.ts`
- `src/main/ipc/pty-spawn-timing.ts`
- `src/renderer/src/components/terminal-pane/pty-input-write-queue.ts`
- `src/renderer/src/components/terminal-pane/pty-transport.ts`
- `src/renderer/src/lib/pane-manager/pane-terminal-output-scheduler.ts`
- `src/main/daemon/daemon-stream-data-batcher.ts`
- `src/main/daemon/session.ts`
- `src/main/daemon/daemon-pty-adapter.ts`
- `src/main/daemon/history-manager.ts`
- `src/main/daemon/session-ingest-throughput.bench.test.ts`
- `src/main/daemon/headless-emulator-snapshot-cost.bench.test.ts`

### Orca performance history

- commit `e84a8ddec`: `notes/terminal-performance-initiative.md`
- commit `e84a8ddec`: `notes/orca-performance-branch-guide.md`
