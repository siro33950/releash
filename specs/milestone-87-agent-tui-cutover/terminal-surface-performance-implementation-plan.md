# Terminal Surface 性能改善 実装計画

作成日: 2026-08-07

状態: 実装完了・最終監査済み

## Goal

`terminal-surface-performance-spec.md`の性能契約を、#1597のAgentSession外部仕様と既存Terminal Surfaceの継続性を一切弱めず、すべてRed-Green-RefactorのTDDでproduction pathへ実装する。

完了時には次の状態をすべて満たす。

- 通常キー、IME commit、直後のEnterがexactly onceかつ順序通りPTYへ到達し、前回のTauri command response完了に送信をpaceされない。
- Provider process exit後、queue overflow、writer unavailableを入力成功として扱わず、入力経路を止めずに一回の明示的errorとして観測できる。
- PTY出力は4 KiB readごとの無制御なbackend event / Tauri Channel messageにならず、backend-owned terminal modelとsequenceを維持したまま2 ms window、最大16 Ki charactersを初期値としてcoalesceされる。
- 別Terminal Surfaceの出力は対象attachmentのhot pathへ流れず、ownerごとの順序、欠落検出、snapshot resyncが維持される。
- xterm描画はnested `setTimeout(0)`をthroughput clockにせず、parse callbackと`MessageChannel`を使ったcooperative schedulerで進む。
- rendererのqueue量をUTF-16 code unitで計測してboundedにし、overflow時はbackend-owned snapshotへ一回だけresyncする。通常acceptance workloadではdropとresyncが0になる。
- 通常の250 ms dirty tickは全scrollback replayまたはfull JSONを生成せず、前回durable sequence以後のordered deltaだけを保存する。
- clean shutdown、crash相当、journal上限後のcompactionから、visible state、cursor、rows / cols、bounded scrollbackを最終durable sequenceまで復元できる。
- old Terminal checkpoint formatをmigration、backfill、互換読取りしない。
- key-to-visible marker、IME marker、renderer drift、非Terminal UI heartbeat、queue、drop、resync、scroll、restore、checkpoint stall、throughputを再現可能なreportとして取得できる。
- single active Terminal Surfaceで、Orca由来のmedian 75 ms以下、worst 300 ms以下、revisit 300 ms以下、unloaded drift 150 ms以下、injected-load drift 2,500 ms以下、queue 2,097,152 UTF-16 code units以下、drop 0、scroll 150 ms以下、restore 1,000 ms以下を満たす。
- Releash固有gateとして、10 MiBのagent-TUI-shaped ANSI workload中のsnapshot resync 0、100 ms超renderer stall 0、非Terminal UI heartbeat drift 150 ms以下、sidebar / workspace selection worst 300 ms以下を満たす。
- Session起動をavailability / lock、durable commit、launch file、checkpoint lookup、child env、PTY open / spawn、output reader ready、first provider byte、first xterm parse、first paintに分けて記録し、deterministic fixtureの30 warm runでp50 / p95 / maxを報告する。
- Session起動ではReleash-owned時間とProvider-owned時間を分離し、計測で支配区間と確認できたReleash-owned処理だけを改善する。durable commitを省略せず、Provider起動を先に成功表示せず、直列外部契約を無断変更しない。
- ATUI-010、ATUI-011、ATUI-012、ATUI-030と#1597のProvider選択、Hook warning、Open / Paused / Archived、Archive / Restore / Delete / GC、history resume、Workflow初期指示、Node完了後の追加質問を変更しない。
- frontendは描画・入力受付・計測点の採取だけを持ち、Terminal Surface identity、sequence、lifecycle、GC、永続化、Provider判定のstate authorityを持たない。
- daemon、remote / mobile、複数pane UI、terminal emulator置換、Provider CLI内部最適化、旧Agent GUI削除を追加しない。
- 各cycleに意図した理由で失敗したRED、最小のproduction実装で通過したGREEN、単純化後も通過したREFACTORのcommandと結果を残す。
- 最終quality gateをすべて成功させ、Issue本文、全採用コメント、Milestone、Spec、規約を再読した最終reviewで指摘が残らない。

## 正本と優先順位

外部挙動は次の順で解決する。

1. GitHub Issue #1597本文
2. #1597「AgentSession lifecycle 採用仕様」コメント
3. #1597「Provider起動・Hook・Workflow接続境界 採用仕様」コメント
4. `acceptance-contract.md`
5. Milestone 87本文
6. `issue-1597-agent-session-vertical-slice.md`
7. `terminal-surface-performance-spec.md`
8. root `AGENTS.md`、`src-tauri/AGENTS.md`、`.claude/rules/rust-first-logic.md`
9. `docs/architecture/README.md`、`DOMAIN.md`、`USECASE.md`、`GATEWAY.md`、`INFRASTRUCTURE.md`、`CONTROLLER.md`、`TEST.md`

性能Specは性能、transport、描画、checkpoint、起動計測だけを追加し、1から6のAgentSession外部挙動を上書きしない。Milestone 87以前のMessage中心Glossaryまたは「TerminalはWorkspaceだけに属する」という記述が#1597のAgentSession専用Terminal Surfaceと衝突する場合、#1597とacceptance contractを優先する。

## 変更前baseline

- branch: `feature/1597-agent-session-tui`
- base HEAD: `4aad297cb`
- #1597の既存変更はstaged / unstagedの両方に存在する。今回の変更はそれらを保持して追加する。
- `pnpm exec vitest run src/hooks/useTerminal.test.ts`: 41件Green。
- `cargo test --locked --test terminal_surface_acceptance`: 9件中8件Green。
- `test_atui_030_provider_cliがterminal_surfaceのroot_processとして終了する`は、Provider exit観測後の`runtime.write(...)`が成功するためRED。単独再実行でも同じ理由でRED。

コード上で確定している現行hot pathは次のとおり。

```text
input:
xterm.onData -> pendingInput -> invoke(write_terminal_surface) response待ち
  -> TerminalSurfaceApplication -> gateway lookup -> NativePtyRuntime bounded queue

output:
PTY 4 KiB read -> UTF-8 decode -> avt apply -> global broadcast
  -> attachment filter -> Tauri Channel -> Promise chain
  -> xterm 16 Ki characters write -> setTimeout(0) -> next chunk

checkpoint:
250 ms dirty tick -> avt全scrollback replay -> full JSON -> sync_all -> rename

Session launch:
availability -> operation lock -> durable commit -> launch files
  -> checkpoint lookup -> child env -> PTY spawn -> attach -> initial parse
```

どの区間が実機遅延に占める割合は未計測である。構造上の待機を確認したことと、支配率を測ったことを混同しない。

## レイヤー配置

| 関心 | 所有層 |
| --- | --- |
| Terminal Surface identity、process state、sequence受理、snapshot recoveryの不変条件 | `domain/terminal_surface` |
| attach、resync、spawn、stop、flushの手順 | `usecase/terminal_surface` |
| domain eventとowner別transport、checkpoint表現とdomain checkpointの変換 | `adaptor/gateway/terminal_surface` |
| PTY read/write、output batch mechanics、clock、raw checkpoint base/journal、file I/O | `infrastructure/terminal` |
| Tauri引数、Channel転送、diagnostic command | `adaptor/controller` / `adaptor/protocol` |
| xterm write、MessageChannel scheduling、queue表示用metric採取 | frontendのrendering infrastructure |
| performance histogram、匿名phase metric、report集計 | `other/telemetry`またはtest/report harness |

性能値やbatch policyをAgentSession aggregateへ追加しない。Infrastructureはdomain型をimportせず、Gatewayは独自lifecycle state machineを持たず、ControllerはRepositoryまたはQueryServiceを直呼びしない。

## Red-Green-Refactor共通手順

各cycleは次の順を崩さない。

1. production behaviorまたは構造契約を表すtestを先に追加する。
2. focused testを実行し、未実装・現行誤動作という狙った理由で失敗した出力を記録する。
3. testの期待値を現行実装へ合わせず、production codeを変更してfocused testをGreenにする。
4. 関連test moduleと該当acceptanceを実行する。
5. 重複、旧経路、二重authority、不要な防御を削除する。
6. focused testと関連testを再実行し、REFACTOR後のGreenを記録する。
7. 次cycleへ進む。

時間閾値は通常unit testへ直接埋め込まない。unit / integration testではbatch上限、順序、boundedness、full snapshot非呼出し等を決定論的に検証し、実時間budgetはproduction-equivalent performance report checkerで判定する。

## Cycle P0: baseline correctnessをGreenへ戻す

### RED

- 既存ATUI-030の「Provider root process exit後のwriteは失敗する」を再現する。
- `NativePtyRuntime`のsenderが残っていても、Domainのprocess stateがExitedなら入力を受理しないgateway testを追加する。

### GREEN

- `TerminalSurfaceRuntimeGateway::write`はowner解決時に登録済みSurfaceのprocess stateを確認し、Exitedへnative inputをqueueしない。
- 生死不明を成功へ丸めず、既存error経路へ返す。

### REFACTOR

- session keyからgenerationとwritable stateを別々のlockで解決する重複を、一回のregistry readへ集約する。
- ATUI-030全体を再実行する。

## Cycle P1: performance report contractと計測harness

### RED

- Orca由来の全budget fieldが欠けたreportをcheckerが拒否するtestを追加する。
- median / p95 / max、queue peak、drop、resync、stall countの境界値と超過をtestする。
- plain ASCIIだけのfixture、16 key未満、user dataを含むreportを拒否する。
- 現行production pathのbaseline reportを取得し、strict budgetが未達であることをREDとして保存する。

### GREEN

- machine-readable JSON schemaとhuman-readable summaryを同じraw sampleから生成する。
- 10 MiBのANSI、Unicode、wide character、cursor redrawを含むdeterministic fixtureを追加する。
- Rust実PTY harnessでinput ingress、writer enqueue、output read、model apply、publish、checkpoint、spawn phaseを採取する。
- built frontend + Chromium + real xterm + mocked deterministic Channelで、renderer parse、queue、drift、scroll、revisit、非Terminal UI操作を採取する。
- 実Tauri IPCを含む値はopt-in app harnessで採取し、mock transportの値をend-to-end key latencyとして報告しない。
- reportはpath、入力本文、AgentSession ID、Provider session IDを含めない。

### REFACTOR

- clock、sample collector、quantile、budget checker、formatterを分離する。
- product codeの通常起動ではprobeをno-opにし、performance harness専用分岐を通常UI state authorityにしない。

## Cycle P2: ordinary inputのresponse pacing除去

### RED

- 一つ目の`write_terminal_surface` Promiseを未解決にしたまま、IME commit、Enter、次keyのinvokeが順序通り開始されるfrontend testを追加する。
- preeditを送らず、commitをexactly once送る既存composition testを維持する。
- queue full、writer unavailable、Exitedを一回の入力errorとして通知するtestを追加する。
- owner Aとowner Bの入力が同じcoalesce単位へ混ざらないRust testを追加する。

### GREEN

- `useTerminal`の`inputWriteInFlight`によるresponse pacingを除去する。
- frontendは到着順にordinary inputをdispatchし、Promise完了を次入力の条件にしない。
- Rust側のowner別bounded input queueとwriter threadを順序の正本として使う。
- command失敗は入力dispatchと独立してerror callbackへ通知し、同じfailure streakを重複表示しない。

### REFACTOR

- `pendingInput`と再帰`flushInput`を削除する。
- imperative `writeToTerminal`とxterm `onData`が同じinput transportを使う。
- frontendにProvider、lifecycle、retry判断を追加しない。

## Cycle P3: backend output batchingとowner routing

### RED

- 4 KiB以下の連続断片が内容と順序を変えず、2 ms window、最大16 Ki charactersへまとまるdeterministic clock testを追加する。
- UTF-8がread境界を跨ぐ場合、結合後も欠落・replacement・順序逆転がないtestを追加する。
- input直後100 ms以内かつ1,024 characters以下のecho / redrawがbulk windowを待たないtestを追加する。
- exit前の保留outputが必ずflushされ、Output sequenceの次にExit sequenceが来るtestを追加する。
- owner Aのsubscriberがowner Bのoutputを受信しないtestを追加する。
- event lag時のsnapshot resyncが既存minimum sequence契約を維持するtestを追加する。

### GREEN

- PTY readerとmodel / event publisherの間にboundedなraw output transportを置く。
- output processorがdecode後のdataを2 ms / 最大16 Ki charactersでcoalesceし、model apply、sequence採番、publishを一回の順序操作にする。
- recent input時の小outputだけをbounded budgetで即時flushする。
- event subscriptionをsession key単位にし、無関係ownerのeventを各attachmentがfilterするglobal hot pathを廃止する。
- output processor完了後だけ`output_drained`を確定する。

### REFACTOR

- time / batch / channel mechanicsをInfrastructure、Domain event変換とowner routingをGatewayへ分離する。
- `TerminalSurfaceEventOrder`の一Surface内順序保証を維持し、別Surface間のglobal lockを作らない。

## Cycle P4: parse-clocked renderer scheduler

### RED

- fake xterm callbackで、parse完了前に次のactive writeを処理済みとしないtestを追加する。
- 一drainが8 writeまたは8 msを超えないtestを追加する。
- zero-delay継続が`setTimeout(0)`でなく`MessageChannel`を使うtestを追加する。
- queued code unitsのcurrent / peakが正確で、2,097,152上限を超えて保持しないtestを追加する。
- snapshot、resize、exitが保留outputを追い越さないtestを追加する。
- workspace TerminalとAgentSession Terminalが同じschedulerを使うtestを追加する。

### GREEN

- xterm専用の`TerminalOutputScheduler`をfrontend rendering infrastructureとして抽出する。
- chunkは最大16 Ki characters、active drainは最大8 write / 8 msとする。
- xterm callbackをparse clockにし、残作業は`MessageChannel`へyieldする。
- queue current / peak、write count、stallを計測する。

### REFACTOR

- `useTerminal`内の配列index、offset、timer、waiter closureをschedulerへ移し、hookはChannel itemを表示機構へ渡すだけにする。
- sequence判断はbackend attachmentに残し、frontend schedulerへ複製しない。

## Cycle P5: bounded queueとsnapshot resync

### RED

- queue cap超過時に中間deltaを無限保持せず、一つのresyncだけを要求するtestを追加する。
- resync中の追加overflowがresync stormを起こさないtestを追加する。
- 新snapshotより前のqueued outputを適用せず、snapshot後のbackend-ordered outputだけを適用するtestを追加する。
- unmount、reattach、renderer reloadでresync guardが次のattachmentへ漏れないtestを追加する。

### GREEN

- attachment単位のresync command / control pathをbackendへ追加し、現在のattachment stream自身がauthoritative snapshotを順序付きで返す。
- scheduler cap超過時はqueued deltaを破棄し、同じattachmentへ一回だけresyncを要求する。
- snapshot適用完了でguardを解除する。

### REFACTOR

- `get_terminal_surface`をfrontendから直接呼んでsequence gapを作る回避策を置かない。
- attach初期snapshot、lag recovery、renderer pressure recoveryを同じbackend resync操作へ集約する。

## Cycle P6: incremental durable checkpoint

### RED

- 通常dirty tickが`NativeTerminalEmulator::snapshot`を呼ばないtestを追加する。
- 同じdeltaを0行履歴と1,000行履歴へ適用し、通常保存bytesが履歴全量へ比例しないtestを追加する。
- OutputとResizeのmonotonic sequence、重複record、末尾partial record、再試行を含むrestore testを追加する。
- clean shutdown、強制終了相当、journal cap compaction後のvisible state、cursor、rows / cols、scrollbackを検証する。
- checkpoint workerがemulator mutex保持中にfile I/O callbackへ入らない構造testを追加する。

### GREEN

- current formatと別namespaceのversion 2 base checkpoint + append-only ordered journalを実装する。
- old version 1 fileは読取り、変換、削除せず、新namespaceからだけ復元する。
- new Surfaceは既知のinitial checkpointをbaseとし、通常250 ms tickはpending Output / Resize recordだけをappendして`sync_all`する。
- append成功後だけdurable sequenceをadvanceし、partial tailとduplicate sequenceをload時に安全に処理する。
- journal size capでfull snapshotを一回materializeし、atomic base replacement後にjournalを切り替える。
- clean stop / shutdownはpending deltaをflushしてからprocess lifecycle完了を返す。

### REFACTOR

- emulator、pending journal、file store、compaction policyを別責務にする。
- full replay materializationをattach / explicit snapshot / compactionに限定する。
- 250 ms cadenceは維持する。実測でincremental fsync自体が支配要因と確認されるまで5秒へ変更しない。

## Cycle P7: end-to-end flow control要否判定

### REDまたは不採用記録

- P2からP6後の10 MiB reportを実行する。
- queue peak、drop、resync、latencyのいずれかがgate違反なら、そのfailure reportをACK導入のREDとする。
- 全gateがGreenなら、ACK / producer pause-resumeを追加しない決定をreport hashと共に記録する。

### GREEN

- 必要な場合だけ、xterm parse後のcumulative ACK、per-owner in-flight上限、producer pause / resumeを追加する。
- wall-clock timeoutだけで未parse dataをconsumed扱いしない。

### REFACTOR

- ACKを追加した場合もsnapshot + sequenceをrecovery正本に維持し、Orcaのdaemon / remote / hidden pane state machineを移植しない。

## Cycle P8: Session起動phase計測と限定改善

### RED

- 全phaseを欠落なく記録し、子phase合計とtotalの関係を検証するtestを追加する。
- deterministic provider fixtureを30 warm runし、p50 / p95 / maxをJSONへ出す。
- frontend request、first byte、first parse、first paintの相関点が欠けるreportを拒否する。

### GREEN 1: 計測

- opt-in timingをcontroller、launch usecaseの手順境界、launch gateway、Terminal gateway、frontend parse / paintへ置く。
- user dataをattributeにせず、Provider種別とphase、success / failureだけを記録する。
- Codex / Claude実process値は観測値、deterministic fixture値は回帰値として分離する。

### GREEN 2: 計測された区間だけの改善

- launch fileが支配する場合、不変fileだけをapp初期化へ移し、Session固有capability / Hook fileは毎launch生成する。
- child envが支配する場合、app起動で不変alias envを一度解決し、同内容wrapperを再書込みしない現契約を維持する。
- checkpoint lookupが支配する場合、新規ownerではversion 2 file lookup / replayを省略し、restore ownerだけが読む。
- durable commitが支配しても省略またはspawn後へ移さず、transaction内の重複read / writeだけを計測して削減する。
- PTY spawnまたはProvider first byteが支配する場合、Releash transport時間とProvider-owned時間を分離して報告し、Provider CLI内部を変更しない。
- create acceptedとruntime readyを分ける必要が出た場合は外部契約変更になるため、このGoal内で無断実装しない。

### REFACTOR

- phase timingをbusiness decisionへ混ぜない。
- app-owned immutable cache以外を追加せず、Session lifecycle cacheをfrontendへ置かない。

## Cycle P9: production acceptanceと最終監査

### RED感度確認

- input pacing、batch、scheduler、journalの各production seamを一時的に無効化し、対応するperformance / correctness gateが失敗することを確認する。mutationは残さない。

### GREEN

- ATUI-010、011、012、030、performance report、IME、非Terminal UI responsivenessをすべて通す。
- Claude / Codexの実PTY release/manual gateで起動phase reportを取得する。

### REFACTOR

- measurement-only bypass、旧full checkpoint dirty path、旧`setTimeout(0)` drain、response-paced input、global event filtering、unused compatibility helperを削除する。
- Issue #1597本文、全コメント、Milestone 87、acceptance contract、#1597 Spec、performance Spec、全規約を再読し、全指摘を追加のRed-Green-Refactorで修正する。

## 実行順

1. P0 baseline correctness
2. P1 report / harness
3. P2 input
4. P3 backend batching / owner routing
5. P4 renderer scheduler
6. P5 queue / resync
7. P6 incremental checkpoint
8. P7 ACK要否判定
9. P8 Session起動
10. P9 acceptance / final audit

P1の実測でcheckpoint stallが入力・UI停止の支配区間と確認された場合だけ、証拠を記録してP6をP2直後へ繰り上げる。P2からP6のproduction変更は一cycleずつ完了させる。

## Cycleごとの検証command

focused commandは変更対象に合わせる。最低限次を使う。

```bash
pnpm exec vitest run src/hooks/useTerminal.test.ts
pnpm exec vitest run src/lib/terminalOutputScheduler.test.ts

cd src-tauri
cargo test --locked terminal_surface
cargo test --locked --test terminal_surface_acceptance
cargo test --locked --test agent_session_tui_acceptance
```

実時間performance gateは専用commandとして追加し、通常unit suiteのflake要因にしない。最終gateは次の全件とする。

```bash
cd src-tauri
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked --test agent_tui_harness
cargo test --locked --test terminal_surface_acceptance
cargo test --locked --test provider_lifecycle_acceptance_test
cargo test --locked --test provider_lifecycle_characterization_test -- --ignored --nocapture
cargo test --locked --test agent_session_tui_acceptance
cargo test --locked --test agent_session_tui_acceptance -- --ignored --nocapture
cargo test --locked
cargo deny --locked check
cargo build --locked

cd ..
pnpm lint
pnpm test
pnpm build
pnpm test:integration
qlty check --no-progress --all
```

## 実装中に変更しない外部仕様

- Standalone New SessionではProvider選択を必須とし、暗黙default、model、permission選択を追加しない。
- Workflow Nodeは設定済みProviderを使い、個別Archive / Delete UIを持たない。
- Hook異常でもAgentSessionとPTYを止めず、warningをアプリ単位で表示する。
- frontend disconnect、tab / workspace切替、renderer reloadでPTYを停止またはGCしない。
- Provider Stop、Node completion、Workflow completionでAgentSessionまたはPTYを停止しない。
- App process restartを跨ぐ同一PTY / process継続を保証しない。
- Provider transcript本文を読み込み、複製、保存しない。
- provider session ID不明でもOpen AgentSessionを利用できる。
- Archive / Restore / Delete / GC / history resumeの状態遷移を性能都合で変更しない。

## 完了判定

この文書のGoal、P0からP9、全performance budget、#1597回帰、最終quality gate、最終正本再監査がすべて完了した場合だけGoalをcompleteにする。時間不足、token不足、部分的な速度改善、unit testだけのGreen、mock harnessだけのGreenを完了理由にしない。

## 実装レビュー後の性能改善計画

### 原因

現在の性能改善が主に描画経路へ効いている一方、入力遅延の正本となる実Tauri経路を計測・保証できていない。現在の性能テストは`setupTauriMock`を使い、baselineも`transport: mocked-channel`である。42.25 MiB/s、key中央値15.55 msという値はrenderer性能としては有効だが、次の経路を通らない。

```text
xterm入力
→ Tauri IPC
→ async command
→ Rust usecase / gateway
→ Native PTY writer
→ PTY echo
→ backend event
→ Tauri Channel
→ xterm parse / paint
```

さらに現在の入力処理は複数の`invoke("write_terminal_surface")`を応答待ちせず開始するが、Rust commandは`#[tauri::command(async)]`であり別taskで実行されるため、Frontendでの呼出し順がGateway到達順になる保証はない。`NativePtyRuntime`が保証しているのは、そこへ到達した後の順序だけである。

### Cycle 1: 実Tauri性能計測を正本化する

RED:

- `mocked-channel`のreportがend-to-end key/IME budgetを通らないテストを追加する。
- 現在存在しない実Tauri IPC reportを必須にして失敗させる。
- first inputからfirst paintまでの相関点が欠けたreportを拒否する。

GREEN:

- 現在のPlaywrightテストはrenderer専用に限定する。
- performance専用buildで公式のWebdriverIO Tauri embedded driverを使い、macOS上でも実アプリを操作する。実Tauri binary・IPCを通す。
- deterministic PTY fixtureを実processとして起動し、入力をechoさせる。
- `onData`、command ingress、ordered ingress admission、native writer enqueue、PTY output read、model apply、event publish、Channel receive、xterm parsed、paintの区間を匿名sequenceだけで計測する。
- 10 MiB ANSI負荷中のUI heartbeatとWorkspace選択も、同じ実WebView上で計測する。

REFACTOR:

- renderer-only reportとTauri end-to-end reportを型として分離する。
- 現在のmock baselineは描画回帰用として維持し、入力性能の合否には使わない。

### Cycle 2: 入力を本当にorderedにする

RED:

- Rust側への到着を意図的に`2 → 1 → 3`へ入れ替えても、PTYには`1 → 2 → 3`で一度ずつ届くテストを作る。
- 同一sequenceの重複を二度書き込まない。
- 古いattachmentから遅れて届いた入力を、新しいattachmentへ混ぜない。
- IME preeditを送らず、commit、Enter、次keyをexactly onceで保存する。
- queue overflow、Exited、writer unavailableをout-of-bandで一度だけ通知する。

GREEN:

- `write_terminal_surface`へ`attachmentId`と単調増加するinput sequenceを付ける。
- owner/attachment単位のbounded ordered ingressをRustのGateway境界に置く。
- Controllerはprotocol変換とUsecase呼出しだけにする。
- Domainは現在どおりTerminal Surfaceのwritable/Exited判定を所有する。
- Infrastructureはraw byte queue、coalesce、PTY writeだけを所有する。
- Frontendのsequence採番は入力順を表すtransport metadataに限定し、受理、retry、lifecycle判断は持たせない。
- 通常入力はcommand responseを待たない。失敗は既存attachment streamのcontrol eventとして返す。

REFACTOR:

- 現在Frontendにある`inputFailureActive`の失敗系列判定をRustへ移す。
- `onData`とimperative writeを同一input transportへ集約する。
- 既存Native PTY queueとwriter threadは重複実装せず再利用する。

### Cycle 3: 入力IPC回数の最適化を実測で判断する

Cycle 1の実測で一入力一`invoke`が支配区間だった場合のみ実施する。

- 同一event-loop turn内の小入力を最大4,096 code unitsまで結合する。
- timer待ちは追加しない。
- owner、attachment、sequence境界を跨いで結合しない。
- IME commitとEnterのbyte orderを維持する。

Tauri IPCが支配的でなければ、この機構は追加しない。

### Cycle 4: ACK/producer flow controlの要否を決着させる

現在は性能Specが「必要時だけ追加」としているACK/producer pause-resumeまで実装されている。しかし、それを正当化する実Tauri負荷結果がない。実Tauri harnessで現在のACKあり/なしをA/B比較する。

- ACKなしでもqueue上限、drop 0、resync 0、stall 0を満たすなら、flow-control、ACK command、Frontend ACKを削除する。
- ACKなしでgate違反が再現する場合だけ、現在の仕組みを維持する。
- batching、16 KiB scheduler、MessageChannel、incremental checkpointは実測違反がない限り変更しない。

これは過剰な防御を残さず、必要な複雑性だけを維持するためのcycleである。

実Tauri 10 MiB A/Bの結果、ACKなしではthroughput 21.23 MiB/sまで上がる一方、queueが2,097,152 code unitsへ到達し、drop 3、snapshot resync 3となった。ACKありではthroughput 12.99 MiB/s、queue peak 261,540 code units、drop 0、resync 0、100 ms超stall 0で全budgetを満たした。このためproducer flow controlとparse後ACKは必要な複雑性として維持する。入力Tauri IPCは0〜1 msで支配区間ではなかったため、入力coalesceは追加しない。

### Cycle 5: Session起動を正しく計測する

現在の実測は不完全である。

- deterministic fixtureは30回だがfirst provider byteまで。
- Claude/Codexは各1回だけ。
- first xterm parse、first paintまで同一runとして相関していない。

Specの要求どおり、deterministic fixture、Claude、Codexを各30 warm runで取得する。

現状の単発観測では、Claudeはfirst provider byte 623 ms / visible 641 ms、Codexはfirst provider byte 383 ms / visible 503 ms、記録済みReleash-owned区間の合計は約4〜5 msである。したがって現段階でdurable commit、Hook file、checkpoint lookupを削る根拠はない。

30回計測後は次のように判断する。

- Releash-owned区間が支配的なら、その区間だけRed-Green-Refactorで修正する。
- first byte以降のparse/paintが支配的なら、attach、snapshot、scheduler経路を修正する。
- Provider first byteが支配的なら、Releash側を変更せずProvider-owned時間として報告する。
- create accepted/runtime readyの契約変更が必要なら、この実装では変更せず、別途合意を取る。

各30 warm runの結果、deterministic fixture / Claude / CodexのReleash-owned phase p50合計はそれぞれ5.01 / 6.20 / 4.80 msだった。Claude / CodexのProvider first byte p50は264.23 / 135.81 msで支配区間だったため、Releash側の起動契約・durability・Hook生成は変更しない。sequence 0の初期replayをProvider出力と誤認しないよう、first parse / paintはProvider出力を含むsequenceだけで開始する。

### Cycle 6: 最終acceptance

実Tauri reportで以下を通す。

- key中央値75 ms以下、worst 300 ms以下
- IME commit/Enterのexactly onceと順序
- UI heartbeat drift 150 ms以下
- Workspace選択worst 300 ms以下
- queue 2,097,152 code units以下
- drop 0、resync 0、100 ms超stall 0
- restore 1,000 ms以下
- 10 MiB ANSI workload
- deterministic/Claude/Codexの起動区間report

その後、`pnpm lint`、`pnpm test`、`pnpm build`、`pnpm test:integration`とRustの`cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test`、`cargo deny --locked check`、`cargo build --locked`を実施する。

前回レビューのうち、Provider exit後のSession UI同期、Codex historyのsubagent除外、ControllerでのAgentSession ID生成は性能Specの非スコープである。これらは隠さず、速度改善とは別のcorrectness / architecture cycleとして扱う。

この計画では、まず実際に遅い境界を測定可能にし、その後ordered ingressを修正する。xterm / avt置換、daemon化、durability緩和、Provider内部最適化は行わない。

## 実装結果とTDD証跡

すべてのproduction変更は、対応する失敗testまたはperformance gateを先に確認し、Green後に旧経路と重複を除去して同じtestを再通過させた。

| Cycle | RED | GREEN / REFACTOR |
| --- | --- | --- |
| P0 | ATUI-030でProvider root process exit後のwriteが成功する失敗を再現 | Exited判定をTerminal Surfaceのstate authorityへ固定し、`terminal_surface_acceptance` 9件を通過 |
| P1 | 欠落field、16 key未満、plain ASCII、user data、mock transportによるend-to-end判定をreport testで拒否 | renderer-only Playwrightと実Tauri / PTY / Channel / xtermのWebdriverIO harnessを分離し、report schemaとbudget checkerを通過 |
| P2 | 未解決の先行command、Rust到着順`2 → 1 → 3`、duplicate、stale attachment、queue / writer failureをfocused testで再現 | owner / attachment / sequence単位のbounded ordered ingressとout-of-band errorへ統一し、response-paced frontend queueを削除 |
| P3 | 4 KiB断片、UTF-8境界、2 ms expiry、16 Ki code units上限、interactive bypass、exit前flush、別owner混入をfocused testで再現 | Infrastructureのbatch mechanicsとGatewayのowner routingへ分離し、global attachment filteringをhot pathから除去 |
| P4 | parse callback前進禁止、8 write / 8 ms、MessageChannel、UTF-16 cap、surrogate境界をscheduler testで再現 | workspace TerminalとAgentSessionで同一schedulerを使用。surrogate境界での無限loopもREDで再現して修正 |
| P5 | overflow時のsingle resync、resync中のstorm防止、snapshot前delta破棄、reattach guard resetをfocused testで再現 | backend-owned snapshot / sequenceを正本とする一回のrecoveryへ集約 |
| P6 | dirty tickのfull snapshot呼出し、履歴量比例保存、partial tail、duplicate / non-contiguous sequence、resize / barrier、compactionをfocused testで再現 | version 2 base + ordered journalへ移行し、通常250 ms tickからfull replay / full JSONを除去。old formatは読まない |
| P7 | 実Tauri 10 MiB A/BでACKなしを実行し、queue cap到達、drop 3、resync 3を再現 | parse後ACKを維持。ACKありはqueue peak 261,540、drop 0、resync 0、100 ms超stall 0。入力IPCは0–1 msのためinput coalesceは不採用 |
| P8 | phase欠落、30 run未満、first byte / parse / paint非相関をreport testで拒否 | fixture / Claude / Codexを各30 warm runし、Releash-owned p50合計5.01 / 6.20 / 4.80 ms、Provider first byte p50 3.79 / 264.23 / 135.81 msを取得。Provider時間が支配的なため起動契約は変更せず |
| P9 | input pacing、scheduler、ACKのperformance seamが対応gateを失敗させることを確認 | #1597 acceptance、実Tauri performance、実Provider観測、全quality gateを再通過。旧response pacing、nested `setTimeout(0)` drain、dirty full checkpoint、global owner filteringは残していない |

正本artifactは次の3件である。

- `terminal-surface-performance-baseline.json`: SHA-256 `afbd451fb4b5aa4b104d5bd4aae56db37acdd828460a2b851c0d7c9d3c25909d`
- `terminal-launch-performance-baseline.json`: SHA-256 `048792888ee674c761667ae132505f950f8d8845280893d9d970810d98f3a5a7`
- `terminal-launch-provider-observations.json`: SHA-256 `f3549d776be44d2b98552f295a780acdc9a8c012fd27d2cf53bdbb40a731469b`

最終実行結果:

- `pnpm test`: 103 files / 1,500 tests Green
- `pnpm lint`: Green。既存`.pnpm-store`のbroken symlink warning 3件のみ
- `pnpm build`: Green
- `pnpm test:integration`: 最終full run 27件Green（2.1分）。直前runのSettings `page.goto("/")` navigation timeout 1件も単独再実行と最終full runの両方でGreen
- WebdriverIO実Tauri Terminal Surface: 3件Green
- fixture / Claude / Codex launch: 各30 warm run Green
- `cargo fmt --check`: Green
- `cargo clippy --locked -- -D warnings`: Green
- `cargo test --locked`: lib 3,696件Green、1件ignored。全integration test binary Green
- `cargo test --locked --test provider_lifecycle_characterization_test -- --ignored --nocapture`: 7件Green
- `cargo test --locked --test agent_session_tui_acceptance -- --ignored --nocapture`: 3件Green
- `cargo deny --locked check`: advisories / bans / licenses / sources Green
- `cargo build --locked`: Green
- `qlty check --no-progress --all`: Green
- `git diff --check`: Green

Issue #1597本文、採用コメント、Milestone 87、acceptance contract、#1597 Spec、性能Spec、root / Rust規約、architecture文書を最終再読した。frontendは入力・描画・計測点、Rust Domain / Usecase / Gateway / Infrastructureはそれぞれstate authority・手順・変換・外部mechanicsを所有し、非スコープの外部仕様変更はない。
