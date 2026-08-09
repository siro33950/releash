# Terminal Surface 実利用性能検証 Spec（Phase 1: 計測基盤と kill-switch A/B）

関連: #1597 / `issue-1597-agent-session-vertical-slice.md` / `acceptance-contract.md` / `terminal-surface-performance-spec.md`（既存 spec。本 spec は改訂しない）

## 要求

**種別**: 改善（性能検証基盤）

**ゴール**: AgentSession TUI の性能問題を実利用条件で再現・計測でき、実装済み性能対策の個別寄与を A/B 比較で判定できる状態にする。具体的には以下が満たされること。

- 実 WorkbenchApp（全 UI ツリーがマウントされた通常画面）を production-equivalent build で起動した状態で性能計測を実行できる。専用画面（TerminalPerformanceScreen）だけの計測を実利用の合否根拠にしない。
- 負荷源として実 Codex/Claude と、実 TUI 形状の fixture（カーソル上移動＋ erase 型の小フレームを持続的高頻度で送るもの）の両方を使える。単発 burst だけを負荷とみなさない。
- **TUI 出力負荷中の**連続キー入力レイテンシを計測できる。1 文字ずつ paint 完了を待つ方式に加え、複数入力が同時に in-flight になる連続タイプを再現する。キー注入に依存しない入力パイプ遅延の代理指標（DSR 往復プローブ）も持つ。
- 実 composition イベント（compositionstart/update/end）を通る IME 計測ができる。確定済み文字の直接投入を IME 計測と称さない。
- AgentSession 起動の計測終点を「provider が入力を受け付ける時点」まで延長し、Releash-owned / provider-owned / hook-owned の各区間を分離して記録できる。
- TUI 負荷中の実 UI 操作（実 Workspace 選択、実リスト再取得を伴うもの）の応答を計測できる。ローカル state 反転ボタンを他 UI 応答の根拠にしない。
- main-thread 帰属を計測できる: longtask、event-loop drift、および層別区間（PTY read → batch → IPC 配送 → scheduler → xterm parse → paint）の各 duration。
- 実装済み性能対策（ACK/producer flow-control、interactive bypass、renderer scheduler、incremental journal 等）を個別に無効化する kill-switch が存在し、paired A/B で各対策の寄与・逆効果を判定できる。
- 計測結果は JSON artifact と人間可読サマリで保存され、同一 build・同一マシン条件で baseline 比較できる。

**スコープ外**:

- 性能対策そのものの追加・変更（Phase 2。本計測の結果を根拠に項目ごとに別途判断する）
- reattach 時の input sequence 恒久欠番バグの修正（Phase 3 として別途）
- 既存 `terminal-surface-performance-spec.md` の改訂・置換
- 合否 budget 閾値の新設・変更（Phase 1 の目的は支配区間の帰属確定と baseline 取得。gate 数値の確定は計測後に別途判断）
- daemon 化・transport 変更等のアーキテクチャ変更

**現状温存**:

- 既存 harness（TerminalPerformanceScreen、wdio 実 IPC 計測、playwright mocked-channel 計測）と既存 budget gate は削除・変更しない。整理は Phase 1 完了後に別途判断する。
- 本番経路のロジックは kill-switch 追加以外変更しない。kill-switch は全て default（env 未設定）で現行挙動と完全一致する。
- IME composition correctness テストは変更しない。
- 既存の計測 probe（terminalPerformanceProbe 等）の本番 no-op 性（フラグ未注入時に不活性）を維持する。

**背景**: #1597 の TUI 移行で、性能対策実装後も Codex/Claude TUI の実利用体感（通常入力・IME 確定・他 UI 応答・起動から操作可能まで）がほぼ改善していない。調査（2026-08-07、実コード・Tauri/wry/xterm 実ソース・orca 現行実装と履歴の読解＋敵対的検証）の結果、次が確認された。

1. 既存性能テストは実利用を再現していない: 実 WorkbenchApp 不使用、Workspace 選択はローカル state 反転ボタン、入力対象は shell echo、16 文字を 1 文字ずつ paint 待ち直列投入、IME は composition 非経由、起動計測は first byte / first paint 止まり。さらに **TUI 出力中に打鍵する計測が harness 全体に存在しない**。
2. 実装済み対策は parse 以前の搬送路最適化に閉じており、コード上確定した支配候補（xterm DOM レンダラ描画、Tauri Channel 終端のメインスレッド eval ＋ 8KB 超 fetch 往復、snapshot 全量再構築の乱発、起動直列チェーン）に触れていない。
3. 一部対策（per-message ACK、1-write 直列 scheduler、interactive 経路のメッセージ増殖、credit reserve のロック保持ブロック）は実利用で逆向きに働きうる構造を持つ。

どの構造が支配的かは未計測であり、対策の追加・除去を判断するには、実利用を再現する計測と対策別 A/B が先に必要である。

**制約**:

- production-equivalent build を性能判定の正本にする（既存 spec 9.1 の共通条件を踏襲）。
- A/B は同一マシン・同一電源状態・quiet machine・paired 比較で行う。
- path・入力本文・Session ID 等の user data を telemetry / artifact に含めない。
- kill-switch は runtime env 変数でゲートし、未設定時はコンパイル済み binary でも現行挙動と完全一致させる（計測専用 build を作らない。ビルド分岐は production-equivalence を壊すため）。

## 振る舞い定義

外部観測可能な振る舞いのみを定義する。計測値の合否閾値は定義しない（baseline 取得が目的）。

```gherkin
Feature: 実利用条件での性能計測

  Scenario: 実 WorkbenchApp での計測実行
    Given performance 計測モードで実 App 指定が有効
    When 計測 build を起動する
    Then TerminalPerformanceScreen ではなく通常の WorkbenchApp 画面が表示される
    And 計測 collector が結果を収集できる

  Scenario: TUI 出力負荷中の連続入力計測
    Given terminal が持続的な agent-TUI 形状出力を受信し続けている
    When 連続キー入力（前打鍵の paint 完了を待たない）を送る
    Then 各打鍵の key-to-paint レイテンシと DSR 応答の到着 cadence 乖離が report に記録される

  Scenario: 実 composition 経路の IME 計測
    Given terminal がフォーカスされている
    When compositionstart → compositionupdate → compositionend の実イベント列で日本語文字列を確定する
    Then 確定文字列が exactly once で PTY に送信される
    And 確定から画面反映までのレイテンシが report に記録される

  Scenario: 負荷中の実 UI 操作計測
    Given TUI 出力負荷が継続している
    When 実 Workspace リストの選択操作を行う
    Then 実際のリスト再取得・画面切替を含む応答時間が report に記録される

  Scenario: 起動の interactive-ready 計測
    When AgentSession を新規作成する
    Then 作成操作から provider が入力を受け付けるまでの時間が report に記録される
    And Releash-owned / provider-owned / hook-owned の各区間が分離して記録される

  Scenario: kill-switch による A/B
    Given ある性能対策の kill-switch を有効にして計測 build を起動する
    When 同一 fixture で計測を実行する
    Then 当該対策だけが無効化された状態の report が得られる
    And switch 未設定の起動では現行挙動と完全に一致する

  Scenario: kill-switch 未設定時の無影響
    Given kill-switch 用 env が一切設定されていない
    When 既存の全テストと既存性能計測を実行する
    Then 全て現行と同じ結果になる
```

## 実装仕様

アーキテクチャ・技術選定レベルの方針。詳細はコードに従う。

### 1. kill-switch（Rust 正本）

- `src-tauri/src/other/performance_switches.rs` にモジュールを置き、プロセス起動時に一度だけ env を読む（`OnceLock`）。未設定時のホットパス追加コストは boolean 読み取りのみ。
- switch 一覧:
  - `RELEASH_PERF_DISABLE_OUTPUT_FLOW_CONTROL`: producer flow-control の `reserve` を no-op 化（ACK は受理するが credit 計算を行わない）
  - ~~`RELEASH_PERF_DISABLE_INTERACTIVE_BYPASS`~~: 2026-08-08 削除。interactive bypass 機構自体を A/B 実測（DOM レンダラで有害・WebGL で中立・idle 代償最大 2ms）に基づき廃止し、出力は常に 2ms/16Ki バッチとした
  - `RELEASH_PERF_DISABLE_TERMINAL_JOURNAL`: 通常出力時の journal record / dirty tick を no-op 化（exit 時の最終 flush は維持。durable 保存が消えることは計測時 opt-in の前提とする）
  - ~~`RELEASH_PERF_DISABLE_GLOBAL_OUTPUT_BROADCAST`~~: 2026-08-08 削除。global broadcast 自体を Exit 系専用へ変更し（常駐 global 購読者は exit observer のみで Output を破棄していた）、Output の二重送信と出力バッチ毎の clone を機構ごと廃止したため switch も不要になった
  - `RELEASH_PERF_DISABLE_RENDERER_WRITE_SERIALIZATION`: frontend renderer scheduler 向けフラグ（backend は値の保持と公開のみ）
  - `RELEASH_PERF_DISABLE_WEBGL_RENDERER`: WebGL レンダラを無効化し DOM レンダラへ戻す（A/B・escape hatch 用。backend は値の保持と公開のみ）
- 真値は `1` または `true`（大文字小文字不問）。それ以外・未設定は無効。
- frontend が switch 状態を照会する query command `get_terminal_performance_switches` を追加する。ロジックは Rust が所有し、frontend は表示・分岐のみ。

### 2. kill-switch（frontend 連動）

- attach 前に switch 状態を照会し、次を切り替える:
  - per-message ACK 送出（backend flow-control 無効時は ACK invoke を送らない）
  - renderer scheduler の直列度（1-write in-flight ↔ 複数 write パイプライン）
  - レンダラ選択（既定は `@xterm/addon-webgl` をロード。disable switch 時、ロード失敗時、WebGL context loss 時は DOM レンダラへ fallback）
- 判定ロジックは持たない。backend の switch 値に従うだけにする。

WebGL レンダラは 2026-08-08 の A/B 計測（light/heavy 両負荷・n=2・canvas 検出・スクリーンショット目視）で drift 約 8 倍改善・100ms 超 stall 全消滅を確認して既定化した。数値の正本は `specs/milestone-87-agent-tui-cutover/performance-baselines/` 配下の baseline artifact（commit 対象）。計測の実行出力は `performance-results/tauri-performance/`（gitignored、実行毎に上書き）に書かれ、baseline として残す結果はそこから specs 配下へコピーする（注意: 旧 `test-results/` 配下は playwright が実行時に削除するため `performance-results/` へ移設した。移設前の A/B 数値は本 spec 記載値が記録）。既知の制約: addon は `0.20.0-beta.286`（orca と同版、xterm 6.0.0 との peer 宣言不一致を許容）、context loss 時は再取得せず DOM へ fallback する。

### 3. 実 WorkbenchApp 計測モード

- `src/main.tsx` の performance 分岐に実 App mount を追加する。選択は **runtime env** `RELEASH_PERF_REAL_APP`（backend command `get_performance_real_app_mode` 経由で照会）で行う。同一の performance build が両モードを提供し、build 分岐で production-equivalence を壊さない。既存 TerminalPerformanceScreen モードは温存する。
- collector（`installPerformanceCollector`）は `src/test/performance/performanceCollector.ts` へ独立させ、両モードから注入する。
- wdio 実行は `RELEASH_PERFORMANCE_REAL_APP=1`（npm script `test:performance:real-app`）で実 App spec（`tests/tauri-performance/terminal-real-app-load.spec.ts`）を選択する。runner 環境の `RELEASH_PERF_DISABLE_*` は app プロセスへ透過され、A/B をコマンドラインから切り替えられる。

### 4. 負荷 fixture

- 持続的 agent-TUI 形状 generator（`ESC[nA` ＋ `ESC[0J` 型の小フレームを指定レート・指定時間送出する script fixture）を追加する。既存の 10MiB burst fixture は残す。
- フレームサイズ・レートは runner env（`RELEASH_PERFORMANCE_LOAD_FRAME_REPEAT` / `RELEASH_PERFORMANCE_LOAD_FRAME_INTERVAL_MS`）で可変。
- 画面内容の検証は DOM rows でなく xterm buffer の probe 読み取り（`__RELEASH_TERMINAL_BUFFER_READERS__`、常時登録・読み取り時のみ実行・wrapped 行は論理行へ連結）で行い、レンダラ非依存にする。terminal 領域のスクリーンショットと WebGL canvas 検出結果を artifact に含める。
- harness の PTY shell は `SHELL=/bin/bash` に固定する（ユーザーの interactive shell 設定・共有履歴による echo の非決定性を排除）。
- Chromium で走る playwright mock テストは tauri-mock が `disableWebglRenderer: true` を返し DOM レンダラで実行する（DOM span/CSS の projection assert を維持。WebGL 既定の実機経路は wdio harness が担う）。

### 5. 計測テスト（wdio 実 IPC harness に追加）

- 持続出力中の連続タイプ計測: paint 待ちなしの連続 key 投入、in-flight 複数、key-to-paint と backend 区間 sample の相関。打鍵間隔は runner env `RELEASH_PERFORMANCE_TYPED_KEY_INTERVAL_MS`（default 300ms）で可変。echo sampler は複数 pending marker を配列で保持し（marker ごとに armedAt 時刻窓で backend sample と join、arm 上書きによる喪失なし）、間隔を echo レイテンシより短くすると複数入力の同時 in-flight を再現できる。default 300ms は従来どおり 1 打鍵ずつ echo が返る逐次計測。使用した間隔は artifact に記録される。
- DSR プローブ: 負荷 fixture に埋め込んだ `ESC[6n` の CPR 応答について、到着間隔の期待周期からの乖離（cadence deviation の p50/p95/max）を記録する。往復レイテンシの直接計測は echo sampler の arm 時刻窓 join と衝突するため、cadence を入力パイプ健全性の代理指標とする。
- 実 composition 列 IME 計測: xterm の textarea に対する合成 composition イベント列（start/update/end ＋確定）で計測。WebDriver からのネイティブ IME 駆動は不可能なため、合成イベント列を正とし、その制約を report に明記する。確定文字列の PTY 二重送信を assert で検出する（累積 echo 行に 1 回以上 echo され、かつどの echo 行内にも 2 回以上現れないこと。redraw / scrollback 由来の行重複は送信の exactly-once と矛盾しないため行単位で観測する）。
- 負荷中の実 UI 操作: 実 App モードで実 Workspace リスト選択を実行し応答時間を計測。
- main-thread 帰属: PerformanceObserver（longtask）と event-loop drift を計測中常時記録する。
- report スキーマを拡張し、負荷条件（fixture 種別・レート）・switch 状態・transport（switch からの導出値。WS 接続失敗時の Channel fallback は区別されない）・実行条件（build 種別・マシン・電源状態）・longtask 集計（WebKit が longtask 未対応の場合は unsupported を記録）を artifact に含める。

### 6. 起動 interactive-ready 計測

- TUI 的な deterministic fixture を **新規追加**する（`tests/fixtures/terminal-launch-tui-fixture`: alt-screen 移行 → 初期描画 → 初期化遅延 → READY マーカー出力 → 入力 echo ループ）。既存の printf+sleep fixture は温存する。`RELEASH_PERFORMANCE_LAUNCH_PROVIDER=tui-fixture`（npm script `test:performance:launch:tui-fixture`）で選択する。
- tui-fixture では READY マーカー paint までを interactive-ready、入力行送信から echo paint までを echo roundtrip として計測する。
- 実 Claude/Codex は echo-probe（probe 文字列を 500ms 間隔で打鍵し、TUI が echo した時点を検出。Enter は送らない）で interactive-ready を観測として記録する。
- 既存の launch report / budget は変更せず、interactive-ready は別 artifact（`terminal-launch-<provider>-interactive.json`）へ出力する。計測意味（probe 方式・粒度）を artifact 自身に記録する。
- hook-owned 区間は provider hook（SessionStart）の backend 到達〜処理完了を `terminal.launch.hook_ingress` phase として記録する。計測境界: hook CLI のプロセス起動と HTTP 往復のオーバーヘッドは backend から観測できず provider-owned 側に含まれる。fixture 起動では hook が発火しないため、この phase は実 provider 起動でのみ現れる。

### 6.5 実装済みの実利用改善（2026-08-08）

計測結果に基づき、次を production 挙動として実装済み。

- **interactive bypass 廃止**: 出力は常に 2ms/16Ki バッチ。`last_input_at` 追跡と bypass 分岐を削除（A/B で DOM レンダラ時有害・WebGL 時中立・idle 代償最大 2ms を確認済み）。
- **reattach 入力順序の修正**: 入力の宛先 attachment/sequence は `attach_terminal_surface` 完了後にのみ切り替える。attach 完了前の打鍵が新 attachment の sequence 0..N を先食いして恒久欠番となる無音バグを除去。
- **起動前入力のバッファ**: 初回 snapshot 適用前の打鍵は破棄せず 1KiB まで順序保持し、適用後に送出する。
- **summary 読み取り経路**: presence 判定・Tauri `get_terminal_surface`・runtime generation 照合は registry summary のみで答え、scrollback 全量 replay の再構築（emulator/registry ロック保持）を伴わない。full materialization は attach と WS の snapshot 配信だけに残る。
- **起動の体感改善**: AgentSession 作成はクリック直後に中央 pane を `provider_agent_session_launching` 表示へ切り替えてから backend create を実行する。失敗は同 pane にエラー表示。
- **二重描画の排除**: replay が空の snapshot では xterm を snapshot 寸法（80x24）へ縮めず fit 済み実サイズを維持し、即時 `resize_terminal_surface` で PTY 寸法を確定する（provider 初回描画前に実サイズが届く）。replay がある場合は記録寸法で描画する従来動作を維持。
- **workspace-status-changed の恒等更新**: 変化のないイベントでルートから全再 render しない。
- **keep-mounted panes（2026-08-08 実装）**: 一度開いた worktree の WorktreeContent は `WorktreePane` として mount を維持し、選択切替は visibility 反転のみ（LRU 上限 5 pane）。復帰時の remount（AgentChat 初期化・Review 全量再取得・terminal 再 attach・replay 再 parse）を構造的に排除した。隠し pane は attach を維持する（WebGL parse は安価。将来コストが計測されたら配送 gate を別途検討）。
- **terminal stream の WebSocket transport（2026-08-08 実装）**: attach/write/ack を local_api `/v1/terminal` の WS で行い、Tauri Channel の per-message メインスレッド eval を経由しない。認証は `Sec-WebSocket-Protocol: releash-bearer.<token>`（server が echo。subprotocol 経由の bearer は `Upgrade: websocket` を持つ実 WS handshake でのみ受理）。renderer へ公開する token は terminal route 専用の scoped token であり、local API 全体を認証する master token は renderer JS へ露出しない（discovery file にも書かない）。endpoint は `get_terminal_stream_endpoint`（`RELEASH_PERF_DISABLE_TERMINAL_WEBSOCKET` 時と local API 不在時は null → Channel へ自動 fallback。予期しない切断も Channel へ fallback して単発 resync。切断 recovery は epoch 単位で管理し、snapshot 到達前の切断でも再入して回復する）。**注**: 修正後の計測では現負荷域で WS と Channel の差は検出されなかった（下記・計測正誤参照）。採用継続は server-client 構想との整合、撤去は簡素化が根拠となる。
- **global broadcast の Exit 系専用化**: 全 surface 共有 global broadcast へは Exit のみ送り、Output 等は owner stream へ move で渡す（出力バッチ毎の clone を排除）。常駐 global 購読者は exit observer のみで Output を破棄していたため、失われる機能はない。
- **StaleAttachment write の自動 resync**: 古い attachment への write 失敗時に `input_unavailable` を publish し、frontend の単発 resync を誘発する（従来はエラー表示のみが続いた）。
- **journal switch の構築時注入**: journal 有効判定は gateway 構築時に注入した bool を読み、出力 hot path での OnceLock 参照を排除（テストからは on/off を注入可能）。
- **hook_ingress phase**: provider hook（SessionStart）の backend 到達〜処理完了を launch phase として記録（§6 の計測境界を参照）。

### 6.6 計測の正誤記録（2026-08-08）

実利用計測の反復で、harness 自身の欠陥を 3 件特定し修正した。修正後の確定値が正である。

1. **fixture の O_NONBLOCK 汚染**: pty 子プロセスは fd0/1 が file description を共有するため、stdin への `O_NONBLOCK` が stdout も非 blocking 化しフレーム書込を破壊していた。`select` ポーリング + raw read へ修正（fixture 単体の echo は 0.6〜3.5ms）。
2. **入力レイテンシのペアリング汚染**: DSR の CPR 応答が inputPoints へ混入し、echo 結果との index 対応がずれて「616ms」という虚偽の値を生んでいた。arm 時刻窓 join へ修正。**確定値: 負荷中 typed key median 43〜77ms / max ≤85ms**（backend 11-14ms + 配送/parse 32-62ms）。pipeline は WebGL 化後、現負荷域で健全。
3. **選択計測の対象不備**: harness workspace は実質 1 worktree（複数行は同一 rootPath へ解決）で、2 回目クリックは状態無変化の no-op となり「8 秒停滞」に見えていた。成立した選択のみ記録する形へ修正（初回選択は 46〜71ms）。worktree「切替」の計測には複数 worktree 環境が必要（未整備）。
4. **DSR 指標の定義**: 初期の「往復レイテンシ（送信→応答の p50/p95/max）」定義は、CPR 応答が入力計測のペアリングを汚染する（上記 2）ため計測手法と両立しない。埋め込み CPR の到着 cadence 乖離を正式定義とし、§5 と振る舞い定義を改訂した。

### 7. A/B 実行手順

- 同一 spec を switch env の有無 2 条件で実行し、artifact を並べて比較する。実行条件（build 種別・マシン・電源状態）を artifact に記録する。
- 判定規律: packaged または production-equivalent build、quiet machine、paired 比較、warm-up 後計測。
- artifact の正本は `specs/milestone-87-agent-tui-cutover/performance-baselines/`（commit 対象）。実行出力は `performance-results/`（gitignored）へ都度上書きされるため、baseline として残す結果は specs 配下へコピーして commit する。

### 8. テスト方針

- Rust: switch モジュールの unit test（default で off、env で on）、switch on/off での batcher / flow-control / journal の挙動 test。
- frontend: scheduler の直列度切替と ACK 抑制の unit test。
- 既存テストが switch 未設定で全て通ることを regression の根拠とする。
