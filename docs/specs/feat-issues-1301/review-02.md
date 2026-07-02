# 実装レビュー（第2回）— feat-issues-1301

- 対象: worktree `feat/issues/1301` の未コミット実装（182 ファイル、+14,855 / −40,692）。review-01.md の全指摘（A-1〜G-9、45 件）への修正後の再レビュー
- 検証方式: 指摘 ID ごとの fix 検証（5 グループ並列）+ 修正で導入された新規欠陥の退行検証（実行側 / backend / surface の 3 観点並列）+ 品質ゲート
- 判定基準: FIXED = 実装・テストとも解消 / PARTIAL = 実装済みだが残作業あり（テスト固定欠落含む） / NOT_FIXED = 未対応

## 総合判定: **FAIL**（ただし大幅前進）

第1回の「駆動系未配線」は解消された。turn 終端 → 状態 emit → status center → webhook → workflow 通知 → telemetry、stale watchdog、復帰計画（Reinject/Mismatch）、event log 記帳、coalescing、`infrastructure/process/`、4 イベント復旧が production 配線され、**報告されていた不具合（UI で streaming 状態が出ない・Stop/Interrupt 消失）の根本原因 A-1〜A-3 はいずれも実装レベルで解消**。品質ゲートも一括 allow なしで green。

FAIL の理由は次の 3 点:
1. **NOT_FIXED 3 件**（C-2 / F-6 / G-6）
2. **PARTIAL 26 件** — 大半はテスト固定の欠落だが、機能欠落を含むものが 10 件（C-4, C-6, D-2, D-7, E-1, E-3, F-3, F-5, G-2, G-4）
3. **修正が新たに導入した欠陥 25 件**（blocker 1 / major 13 / minor 11。→ N-1〜N-15）。特に並行性・状態機械まわり（二重 turn、complete_turn 非冪等、interrupt フラグ leak、turn 終端後のデータ喪失）は第1回に存在しなかった新規リスク

## 品質ゲート

| ゲート | 結果 |
|---|---|
| `cargo fmt --check` / `cargo clippy -- -D warnings` | PASS（一括 allow なし。残る `#[allow]` は 68 件全て理由 + 指摘 ID 付き item 単位） |
| `cargo test` | PASS 2,328 件（第1回 2,305 → +23。**HEAD 2,812 比 −484 のまま = G-3 が主因**） |
| `pnpm lint` / `pnpm test` | PASS（1,304 件、frontend テスト増なし） |

---

## 1. 修正検証結果（45 件: FIXED 16 / PARTIAL 26 / NOT_FIXED 3）

### A/B: UI 状態・turn 終端（FIXED 5 / PARTIAL 4）

| ID | 判定 | 残作業 |
|---|---|---|
| A-1 | PARTIAL | 実装済（両開始パスで Streaming emit、`usecase.rs:1107/2581`）。**queued drain パスの emit を固定する回帰テストが無い** |
| A-2 | FIXED | status center 実配線（`usecase.rs:3142-3222`、全遷移点から呼出、テストあり） |
| A-3 | PARTIAL | payload 形状は listener と一致（rename 除去済）。**presenter 側の serde 形状固定テスト未追加**（rename 再付与の退行を検出不能） |
| B-1 | PARTIAL | message_id 退避・最終 persist・buffer 解放実装済（`usecase.rs:2207-2281`）。**TurnCompleted 注入の終端処理テスト 0 件**（同型バグ再発を検出不能） |
| B-2 | FIXED | Completed→Done/Failed→Error/Abort→Idle/Timeout→124（`usecase.rs:3058-3104`）、webhook 発火経路・テスト確認 |
| B-3 | FIXED | workflow turn-complete 通知の production 配線（lib.rs:397 → runtime_driver → WorkflowRuntimeUsecase）+ 受信側テスト 6 件 |
| B-4 | FIXED | typed payload で exit_code/interrupted/session_state を搭載 |
| B-5 | PARTIAL | 記帳は全種実装済。**保存→再起動復元の round-trip テストが SessionClosed のみ** |
| B-6 | FIXED | `record_agent_turn_duration` を complete_turn で backend 非依存に記録 |

### C: permission（FIXED 1 / PARTIAL 5 / NOT_FIXED 1）

| ID | 判定 | 残作業 |
|---|---|---|
| C-1 | PARTIAL | §8.2 の順序（送信成功後 patch→永続化→記帳→flush→通知、失敗時 patch なし）を完全実装。**usecase の permission 遷移テスト 0 件** |
| C-2 | **NOT_FIXED** | answers が依然 allow response の兄弟キー（`claude/permission.rs:182-195`）。§6.3 の「updatedInput へ {questions, answers} 合成」が無く、**Question flow では updatedInput={} 送信で元 input も失われる** |
| C-3 | FIXED | Deny の method 別分岐（decline result / error）復元。テストも旧意味（decline 期待）に復元済（HEAD と照合、帳尻合わせの逆戻りなし） |
| C-4 | PARTIAL | server request 一本化・id 相関は完了。**未知 id の `unwrap_or_default()` → 既定 accept 送信が残存**（`codex/session.rs:210-216`。review 明示のセキュリティ項目） |
| C-5 | PARTIAL | resync 実装済（`usecase.rs:2964-3003`）。テスト 0 件 |
| C-6 | PARTIAL | TurnCompleted での掃除は実装。**「Prompt 昇格分のみ保存」が未実装**（auto-allow 分の input 全量が turn 中蓄積） |
| C-7 | PARTIAL | id 照合実装済（不一致エラー）。テスト 0 件 |

### D: 復旧・プロセス管理（FIXED 4 / PARTIAL 7）

| ID | 判定 | 残作業 |
|---|---|---|
| D-1 | PARTIAL | watchdog・Timeout 終端・SessionSpec 配線・テストあり。**drain 起動 turn に watchdog なし / WaitingPermission で監視消滅**（→ N-4） |
| D-2 | PARTIAL | Plan 決定・Mismatch 処理・テスト実装済。**`ContextCarryState::Failed` の書き込みが全コード 0 箇所**（BackendSessionCleared で carry が Resumed のまま残留・通知なし） |
| D-3 | PARTIAL | Fatal 後 drain + 再 open 実装。**Fatal 注入テスト 0 件・drain 失敗/stale 後の再試行契機なし**（→ N-5） |
| D-4 | PARTIAL | Fatal で close() 実装済。テスト 0 件 |
| D-5 | FIXED | `infrastructure/process/{pid_registry,child_env,child_process}` 実装・両 backend 配線・起動時 cleanup（ただし owner 検証欠落 → N-12） |
| D-6 | PARTIAL | rollback_started_turn 実装済。直接テストなし |
| D-7 | PARTIAL | startup retry・resume 判定・EOF Crash 実装。**残: 新規 thread/start エラーが Fatal にならない（→ N-10）、turn_id 未クリア（→ N-9）、clamp 未移設・既定 30s→15s 変化、テストなし** |
| D-8 | FIXED | 非 JSON 行 skip + CLI バージョンチェック（>= 2.0.0） |
| D-9 | PARTIAL | interrupt 相関・10 秒合成・rollback 実装。**Completed が aborting を消費しない等フラグ leak 3 箇所（→ N-8）、旧 bridge-utils テスト未移植** |
| D-10 | FIXED | stderr reader 常駐（両 backend） |
| D-11 | FIXED | 段階 shutdown（SIGTERM→SIGKILL、pgid sweep）を共有 utility 化 |

### E/F: streaming・surface（FIXED 4 / PARTIAL 4 / NOT_FIXED 1）

| ID | 判定 | 残作業 |
|---|---|---|
| E-1 | PARTIAL | live buffer は domain `merge_part` に一本化・全規則テストあり。**projector の push_or_update_* 群と session DTO 側の完全コピーが残存**（委譲も逸脱記録も無し。live と保存正典の merge 規則が乖離し得る） |
| E-2 | FIXED | assistant からは tool_use のみ変換（二重表示解消） |
| E-3 | PARTIAL | coalescing・1 秒 persist・retry 実装 + 純関数テスト 5 件。**旧 93 テストの移植が実質 6 件・§8.2 の turn 外 post-turn 契約が未実装（→ N-6 のデータ喪失）** |
| F-1 | FIXED | 4 イベント全て presenter から emit・payload は frontend 型と一致 |
| F-2 | FIXED | Codex archive/unarchive/fork/skills/fuzzy をワンショット client で実装・frontend 分岐撤去（ただしタイムアウト時リーク → N-11） |
| F-3 | PARTIAL | can_change_backend は Rust 供給・frontend 判定撤去。**default model フォールバックが §9.4-5 削除指定に反し残存**（`BoundSessionChat.tsx:194-203`）+ スナップショット非更新（→ N-15j） |
| F-4 | FIXED | TS 再実装削除・Rust 供給（ただし 50 件探索制限の退行 → N-14） |
| F-5 | PARTIAL | 説明付き parser・emit 実装済。**initialize control_response を処理しないため実運用で説明が届かない**（`convert.rs:70-79` が control_response を捨てる） |
| F-6 | **NOT_FIXED** | compact_boundary は grep 0 件。system subtype 対応表（現行 listener が表示していた集合の移植 + テスト固定）も未実装 |

### G: 品質・規約・テスト（FIXED 2 / PARTIAL 6 / NOT_FIXED 1）

| ID | 判定 | 残作業 |
|---|---|---|
| G-1 | FIXED | 一括 allow 全除去（`rg '#!\[allow'` 0 件）・列挙デッドコード全て配線 or 削除。残 68 件は理由付き item 単位 |
| G-2 | PARTIAL | 暗黙フォールバック撤去（欠損は Err）。**meta 必須化（Option のまま）・読込時 invalid 隔離・欠損許容テスト 2 件の撤去が未実施** |
| G-3 | PARTIAL | テスト +23 のみ（HEAD 比 −484）。**§15 必須 8 シナリオ中 5 つ欠落**（Fatal 後 queue 保全 / permission 遷移順序 / post-turn 更新 / persist-first / TurnCompleted 終端注入）。**§6.2 変換表の行単位固定は 5/18 行のまま**。旧 stream_emit 93→5、context_restore 10→4 |
| G-4 | PARTIAL | 分割ファイル新設・system prompt/setsid/shutdown 共通化は完了。**usecase.rs が 3,335 行に肥大（第1回 1,840 行より悪化）、event_apply.rs の名実不一致、scan_dir/RuntimeSessionPhase/JSONL framing の重複温存** |
| G-5 | FIXED | tempfile を通常 [dependencies] へ |
| G-6 | **NOT_FIXED** | registry の個別 manage（`lib.rs:113-117`）と try_state（application_lifecycle / workflow gateway 3 ファイル）が温存。design への逸脱追記も無し |
| G-7 | PARTIAL | stale doc 修正済。**空モジュール `adaptor/controller/agent_session/mod.rs` 残存** |
| G-8 | PARTIAL | 新設 usecase テスト群は日本語規約準拠。**merge_part 4 件・convert 系 12 件が英語名のまま** |
| G-9 | PARTIAL | parse 失敗 Err 化実装済。失敗経路テスト 0 件 |

---

## 2. 新規指摘（修正が導入した欠陥。統合済み: blocker 1 / major 13 / minor 多数）

### N-1（blocker）send_message の busy 判定と turn 開始が非原子 → 二重 turn・二重プロセス spawn
- `usecase.rs:267`（is_turn_busy）/ `:992-1094`（start_turn_for_session）/ `:1456-1463`（runtime 上書きで 1 個目リーク）/ `:2829`（next_turn_id 非原子）
- `ensure_runtime` の spawn 等の await 中は phase=Idle のままで、同一 session への並行 send（UI 二重送信・workflow との競合）が二重 turn・二重 spawn・turn_id 重複を起こす。UI 経路（`session.rs:1184`）は `acquire_session_lock` を取らないため workflow 側の排他保証も破れる。
- 修正: send_message / drain でも session lock を取得し、busy 判定 + phase 予約 + turn_id 採番を同一臨界区間で実施。並行 send の回帰テスト追加。

### N-2（major）complete_turn が非冪等（turn/世代相関ガードなし）→ 二重終端
- `usecase.rs:2177-2332` / stale watchdog `:1266-1296`
- stale Timeout 終端（Error/124）後、backend の interrupt 応答（遅延 `TurnCompleted(Abort)`）で complete_turn が再実行され **Error が Idle に上書き**。10 秒 grace 中に新 turn が始まっていた場合は旧 runtime の終端イベントが新 turn を誤終端する。
- 修正: complete_turn 冒頭で generation/turn_id 一致と phase!=Idle を検証し不一致は no-op。watchdog の interrupt/close も generation 再検証後に実行。回帰テスト追加。

### N-3（major）respond_permission が無条件 phase=Streaming 代入 → 恒久 busy
- `usecase.rs:418-427`
- await 中に turn が終端（Deny 直後の即終端等）すると Idle を Streaming に巻き戻し、以後 drain 契機なく全 send が enqueue。
- 修正: lock 区間内で phase==WaitingPermission かつ pending.id 一致の場合のみ遷移。

### N-4（major）stale 監視の穴 3 点
- (a) drain 起動 turn（queued/Mismatch 再開/Fatal 後）に watchdog がスポーンされない（`usecase.rs:2580-2597`）
- (b) watchdog が WaitingPermission 観測で終了し、respond 後の再アームも無い → permission を経た turn は監視対象外（`:1219-1245`）
- (c) permission 待ち時間を無進捗として計上 → 長考後に許可した健全な turn を誤 Timeout（respond が last_progress_at を更新しない）
- 修正: watchdog スポーンを共通ヘルパ化して drain 成功パスにも適用、WaitingPermission では再スリープ継続、respond 成功時に last_progress_at 更新。境界値テスト追加。

### N-5（major）drain 失敗時の恒久スタック + TurnStarted 記帳位置
- `usecase.rs:2460-2512` / `:267`
- drain の起動契機が pump イベントのみのため、再 open 失敗等で return すると phase=Idle・queue 非空のまま再試行契機なし（以後の send は全て enqueue）。また TurnStarted 記帳が queue pop 確定前で、並行 cancel 時に実行されない TurnStarted が保存正典に残り projection が Active 固定。
- 修正: send_message 冒頭で「live runtime なし && phase=Idle && queue 非空」なら drain 起動（or 失敗時の遅延再試行）。TurnStarted 記帳を pop 確定後へ移動。

### N-6（major）turn 終端後の trailing PartsMerged が確定 message を破壊（データ喪失）
- `usecase.rs:1906`（apply_parts に phase 分岐なし）+ `:2274-2280`（buffer clear 後も last_agent_message_id 残存）
- 終端後 delta が空 buffer に merge → seq=0 snapshot emit で **UI の確定 parts を単一 part に置換**、1 秒 persist 通過時は **保存済み最終 parts を全量上書き**。design §8.2「turn 外は post-turn 更新として適用し即時永続化」が未実装（E-3 残と同根）。stale 先行終端後に backend が生きて stream し続けるケースで現実に発生。
- 修正: apply_parts で phase=Idle を分岐し、保存済み parts を base にロード → merge → 即時 persist（snapshot 単独 part emit はしない）。旧 post_turn reseed テストの意味を移植。

### N-7（major）event log が O(n²) I/O の full-recompute 経路に
- `event_store.rs:25`（毎回全読込→pretty 全書換）+ `usecase.rs:2841-2872`（delta 毎に load_session_events 全 parse、durable 1 件毎に全書換 + 全 projection）
- FinalPartsRecorded（turn 全文）や ImageRecorded が蓄積するため長 session で streaming が滞り、pump 遅延が stale 誤判定と複合。CLAUDE.md「full-retention / full-recompute 経路を増やさない」抵触。
- 修正: durable 対象を含まない delta は load をスキップ。JSONL append 化（または live session の event log メモリ保持 + batch append）、projection は終端・状態遷移時のみ。

### N-8（major）Claude interrupt 状態機械のフラグ leak（3 箇所）
- `claude/session.rs:405-414, 449-457, 171-189`
- (a) aborting 中の `TurnCompleted(Completed)` が Abort に変換されず aborting も未消費（§6.2 wasAborted 優先違反）→ 残留 aborting で**次 turn の Failed/Crash が Abort に誤変換・backend_session_id 誤 rollback**。(b) idle interrupt の timer が aborting を戻さない。(c) synthetic_abort_pending が turn を跨いで残留し、次 turn の本物の TurnCompleted を swallow → Streaming 固着。
- 修正: Completed arm でも aborting を消費して Abort 化、timer early-return でフラグ復帰、start_turn 冒頭でフラグリセット + turn 世代相関。normalize_runtime_event を関数テストで固定（旧 buildResultTurnCompletion/rollback テストの移植）。

### N-9（major）Codex turn_id ライフサイクル欠陥
- `codex/convert.rs:92-97`（turn/completed でクリアせず）+ `codex/session.rs:323-360`（turn/started で同期されない）
- idle EOF で偽の `TurnCompleted(Crash)`（完了済み session が Error 化。host 側 complete_turn の idle ガード欠如 = N-2 と複合）、turn 開始直後の Stop が旧 turn_id を送って no-op になる窓。
- 修正: turn/completed 変換で turn_id=None、read_loop の同期を毎メッセージ Option ごと反映。3 シナリオをテスト固定。

### N-10（major）Codex の新規 thread/start エラーが Fatal にならない
- `codex/convert.rs:52-60`（`requested_resume_id.is_some()` ゲート）
- 実エラーが Error part のみで startup_error が立たず、open() が 15 秒 × リトライ回数を空転して実エラーを隠した StartupTimeout になる（design §7.2 違反）。
- 修正: startup request の error 応答は resume 有無に関わらず Fatal（resume 時のみ BackendSessionCleared 先行）とし、open() が即 Err で返る経路へ。

### N-11（major）Codex ワンショット client のタイムアウトでプロセスリーク
- `codex/models.rs:201-221`（timeout 時に `?` が shutdown より先に return）
- skill_catalog / fuzzy_file_search は高頻度呼び出しで setsid 済みプロセス + PID ファイルが累積。
- 修正: timeout 結果を変数に受け、先に `process.shutdown().await` してから `?` 評価。

### N-12（major）pid_registry の owner 検証欠落 → 多重起動時に他インスタンスの agent を誤 kill
- `infrastructure/process/pid_registry.rs:14-22, 96-101, 225-239`
- 旧実装（issue #1024 対応）の owner_app_pid / owner_start_time 検証が落ち、起動時 cleanup が登録済み pgid を無条件 SIGTERM→SIGKILL。
- 修正: PidFileV1 に owner フィールドを復元し、owner 生存（start_time 一致）は skip・検証不能は保守的 skip の旧規則 + テストを移植。

### N-13（major）Claude replace_process 失敗で zombie runtime
- `claude/session.rs:101-113`
- 旧プロセス kill 後の spawn 失敗で Fatal を出さず、死んだ handle が残って以後の start_turn が恒久失敗（host は Fatal 契機でしか runtime を破棄しない）。
- 修正: spawn 失敗時に events_tx へ Fatal 送出（または新 spawn 成功後に旧プロセスを shutdown する順序へ）。

### N-14（major）permission presentation の 50 件探索制限（F-4 修正の退行）
- `usecase.rs:611-639`（find_permission_request が live + 最新ページ 50 件のみ）
- 51 件超の session で過去へスクロールすると解決済み plan/質問/回答が縮退表示になり、スクロールのたびに失敗 invoke が発生。
- 修正: cursor で全ページ逆順走査（発見時打ち切り）or Permission part への presentation 同梱。not-found 経路の回帰テスト（Rust + TS）追加。

### N-15（minor 群）
| # | 場所 | 内容 |
|---|---|---|
| a | `codex/wire.rs:67` | -32001 採用理由のコードコメント欠落（design §7.3 の明示要求） |
| b | `usecase.rs:446-452` | resolved patch の配信が retry 機構をバイパス（emit 失敗時 pending 表示残留） |
| c | `codex/session.rs:307-309` | pending_methods が全 server request を蓄積・掃除は respond 時のみ（turn 終端で破棄すべき） |
| d | `telemetry/attributes.rs:62` | AgentTurnMetric の bridge 系 stale variant 温存（自己参照 allow） |
| e | `codex/wire.rs:32-36` 他 | 使用済み item への stale な #[allow(dead_code)] 残存 / `usecase.rs:2685` の `let _ = registry;` |
| f | `codex/convert.rs:61-64` | untracked resp error（thread/name/set 等）まで Error part 化（契約 9 違反、リネーム失敗が transcript に露出） |
| g | `claude/permission.rs:37` | tool_name 欠落の can_use_tool が無応答 → CLI 無期限待ち（deny 応答を返すべき） |
| h | `claude/session.rs:173-189,244-263` | permission_mode/plan_mode の楽観更新（送信失敗で CLI と乖離、pre-turn 同期が誤判定） |
| i | `codex/session.rs:304-327` | read_loop に closed ガードなし（teardown 中の stale イベントで終端処理が走る） |
| j | `useAgentChat.ts:780-909` | canChangeBackend が get_session 時点のスナップショットで送信後も true のまま（silent no-op 化） |
| k | `PermissionDialog.tsx:354-361` | Rust が受け取らない request 引数を毎描画 IPC 送信 + テスト mock がそれ依存で not-found 経路を隠蔽 |
| l | `adaptor/controller/command/agent_session/image.rs` | 画像添付の I/O・変換本体が controller 層に残存（CONTROLLER.md 不整合）+ 旧テスト 3 件未移植 |
| m | `pid_registry.rs:56-60,126-131` | CleanupGate の lost-wakeup パターン / save_pgid と cleanup の data_dir 不一致（RELEASH_DATA_DIR 明示指定時に orphan 回収不能） |
| n | `claude/process.rs:148-165,231-257` | stdout 行長 1MB 上限未実装 / CLI バージョンチェックが同期実行で worker ブロック |
| o | `codex/session.rs:52` | 旧 timeouts.rs の clamp（300s/10 回）未移設・startup 既定 30s→15s 変化 |

---

## 3. 修正の優先順位（次ラウンド）

1. **P0（新規 blocker/データ喪失・状態破壊）**: N-1, N-2, N-3, N-6, N-8, N-12
2. **P1（機能欠落の完遂）**: C-2, F-6, N-4, N-5, N-7, N-9, N-10, N-11, N-13, N-14, D-2 残（Failed carry）, C-4 残（未知 id 拒否）, E-3 残（post-turn 契約 = N-6 と同一）, F-5 残（initialize 応答処理）, G-2 残（meta 必須化・invalid 隔離）
3. **P2（テスト固定・規約）**: G-3（§15 必須 5 シナリオ + 変換表行単位）、A-1/A-3/B-1/B-5/C-1/C-5/C-7/D-3/D-4/D-6/D-7/D-9/G-9 の残テスト、E-1 残（projector 委譲 or 逸脱記録）、F-3 残（default model fallback）、G-4/G-6（or design 逸脱追記）/G-7/G-8、C-6 残、N-15 群

**注意**: N-1〜N-6 は互いに絡む並行性の問題であり、個別パッチではなく「session lock の臨界区間設計（busy 判定・phase 遷移・turn_id 採番・complete_turn の冪等化・watchdog 世代）」として一体で設計・修正すること。テスト（G-3）は修正の検出器なので先行または同時に整備すること。
