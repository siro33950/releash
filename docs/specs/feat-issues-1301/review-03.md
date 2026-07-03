# 実装レビュー（第3回）— feat-issues-1301

- 対象: worktree `feat/issues/1301` の未コミット実装（197 ファイル、+18,073 / −40,614）。review-02.md の全指摘への修正後の再レビュー
- 検証方式: 修正検証 5 グループ（並行性 P0 / Claude / Codex / 実行側 / 品質・規約）+ 退行検証 2 観点（新規欠陥 / FIXED 16 件の退行）+ 品質ゲート

## 総合判定: **FAIL**（収束は大きく前進、残るは deadlock 1 件を頂点とする局所修正とテスト固定）

**前進**: review-02 指摘の大半が解消（判定合計: FIXED 52 / PARTIAL 26 / NOT_FIXED 10 / REGRESSED 5）。並行性一体設計の中核（N-2 complete_turn 冪等化 / N-3 respond 条件付き遷移 / N-6 post-turn 契約 / N-7 event log I/O）は実装・テストとも FIXED。**第1回からの FIXED 16 件は全て退行なし**を個別確認。品質ゲートも green（clippy 警告ゼロ / cargo test 2,358 / pnpm 1,305）。

**FAIL の理由**:
1. **新規 blocker 1 件（R3-1）**: 新設 lock 設計自体が deadlock を導入。output_contract 付き workflow step で**確実に発生**する
2. lock 境界・状態機械の残欠陥（R3-2〜R3-8。多くは review-02 指摘の「条件付き残存」または修正の過剰適用）
3. 機能欠落の残り（R3-9〜R3-13: C-2 実経路 / task id 対応表 / F-5 / open_tabs hydration / telemetry 配線）
4. テスト固定の未了（G-3: §15 必須 8 シナリオ中 3 欠落、変換表 claude 9/18・codex 9-10/21 行、各 ID の残テスト多数）

## 品質ゲート

| ゲート | 結果 |
|---|---|
| `cargo fmt --check` / `cargo clippy -- -D warnings` | PASS（一括 allow 0 維持。item allow 66 件・理由付き） |
| `cargo test` | PASS 2,358 件（前回 +30。HEAD 比 −454） |
| `pnpm lint` / `pnpm test` | PASS（1,305 件） |

## review-02 指摘の解消状況（要点）

| グループ | 結果 |
|---|---|
| 並行性 P0（N-1〜N-6） | N-2/N-3/N-6 **FIXED**（テスト込み）。N-1/N-4/N-5 PARTIAL（→ R3-3/R3-4/R3-5）。ただし lock 設計が R3-1/R3-2 を新規導入 |
| Claude（C-2, F-6, N-8, N-13, N-15g/h/n 等） | F-6/N-13/N-15g/N-15h/N-15n **FIXED**。C-2 PARTIAL（→ R3-9）、N-8 PARTIAL（→ R3-8）、F-5 NOT_FIXED（→ R3-11）、G-9 テスト残 |
| Codex（N-9〜N-11, C-4, N-15a/c/f/i/o, D-7） | N-10/N-11/C-4/N-15a/c/i/o **FIXED**。C-3/F-2 退行なし。N-9 PARTIAL（→ R3-7）、N-15f **REGRESSED**（→ R3-6）、D-7 テスト残 |
| 実行側（N-7, N-14, D-2, E-1, E-3, F-3, N-15b/j/k/l, C-1/5/6/7, A-3, B-5） | N-7/D-2/E-1（逸脱記録済）/F-3/N-15k/C-6 **FIXED**。N-14/E-3/N-15b/j/l/C-1 PARTIAL（テストのみ残）。C-5/C-7/A-3/B-5 NOT_FIXED（テスト 0 件のまま） |
| 品質・規約（N-12, N-15m, G-2〜G-8, N-15d/e） | G-2/G-6/G-7 **FIXED**。N-12/N-15m PARTIAL（テスト移植残 + owner 検証の方向差 → R3-16h）。G-3 PARTIAL、G-4 **NOT_FIXED**（→ R3-15）、G-8 残 3 件、N-15d/e 残 |
| FIXED 16 件の退行 | **全 16 件退行なし**（A-2, B-2, B-3, B-4, B-6, C-3, D-5, D-8, D-10, D-11, E-2, F-1, F-2, F-4, G-1, G-5） |

---

## 残指摘（R3-1〜R3-16）

### P0: lock 境界と状態機械（即修正必須）

#### R3-1（blocker）event pump が session lock 保持のまま workflow 通知を同期 await → 確定 deadlock
- 経路: `usecase.rs:1656`（pump が lock 保持）→ `:2810-2816`（complete_turn 内で workflow turn-complete 通知を await）→ `runtime_driver.rs:31` → `usecase/workflow/turn_complete.rs:33-58` → `runtime_engine_impl.rs:1370 → 3389/2394/1822/1159` → `:2935` の `handle_missing_required_output` が**同一 session の `acquire_session_lock` を再取得**
- tokio Mutex は非再入のため pump task が永久停止し、当該 session の send / respond / workflow run が全凍結。**output_contract 付き step で agent が SubmitOutput せず turn 完了すると 1 回目から確実に発生**（auto-evaluate / auto-approve / 並列子 / pending 検証の 4 経路）。`step_lifecycle.rs:131-132` の自書 invariant にも違反。既存 workflow テストは lock 非保持で直接呼ぶため検出不能
- 修正: complete_turn は notification の**生成まで**とし、dispatch は pump ループの guard drop 後（または `spawner.spawn` で非同期化）に行う。mock backend + contract step の missing-output 完了注入で「repair turn が開始される」ことを timeout 付きで検証する回帰テストを追加

#### R3-2（major）watchdog / Fatal / mismatch が lock 保持のまま interrupt → 10 秒 grace → close を await（最大 ~20 秒凍結）
- `usecase.rs:1500-1543`（watchdog。STALE_CLOSE_GRACE=10s は本番値）、同型: `:2053-2061`（Fatal）、`:1708-1710`（mismatch）
- 修正: `state.runtime.take()` で切り離した後は guard 依存の不変条件がないため、状態遷移（complete_turn）完了時点で guard を drop し、interrupt / grace / close は lock 外で実行。テストの grace 短縮（10ms）で隠れないよう、lock 非保持を assert する形の回帰テストを工夫する

#### R3-3（major・N-1 残）start_session が session lock を取らない → 二重 open・runtime リーク
- `usecase.rs:395-418`。並行 send と競合すると後着が `state.runtime` を上書き（`:1611-1613`）し先着プロセスがリーク、その turn のイベントは epoch 不一致で全破棄。UI は「タブを開いて即送信」で競合し得る
- 修正: start_session 冒頭で `acquire_session_lock` を取得し ensure_runtime 完了まで保持

#### R3-4（major・N-4b 残）permission 待ちが stale_timeout を超えると watchdog が消滅
- `usecase.rs:1477-1497` + `stale.rs:59-66`: WaitingPermission 中は last_progress_at が進まず、経過 >= timeout で remaining=0 → loop break → 非 Streaming で return。respond 後の再 arm なし。**既定 180 秒を超えて許可判断した turn は以後監視されない**
- 修正: phase==WaitingPermission の間は固定間隔で再スリープ継続（remaining ではなく）。「WaitingPermission 超過 → respond → stale 検出が生きている」統合テストを追加

#### R3-5（major・N-5 残）drain 自己回復の `runtime.is_none()` 条件で恒久スタック経路が残存
- `usecase.rs:542-551`。drain 中の start_turn 失敗 / next_turn_id 失敗 / queued_agent_message 失敗（`:2947/:2964/:3057-3092`）で「phase=Idle・runtime 生存・queue 非空」になると、以後の send は enqueue のみで drain 契機なし
- 修正: recover 条件から `runtime.is_none()` を外す（phase==Idle && queue 非空で drain。start_next_queued_turn は live runtime を再利用できる）。drain 失敗 → 次 send で回復の回帰テスト追加

#### R3-6（major・N-15f の REGRESSED）Codex turn/start の error 応答が無音破棄され実エラーが隠蔽される
- `codex/convert.rs:63` + `codex/session.rs:159-173`: untracked error の握り潰し解消の過剰適用で、design §7.2 が tracked と明記する turn/start の error まで `Vec::new()` で無視。turn/start が拒否されると（invalid params・thread 消滅・model 不受理等）Error part も TurnCompleted も出ず、**180 秒の stale timeout まで Streaming 固着 → 無関係な「応答停止」エラーに化ける**
- 修正: start_turn の request id を convert 側へ登録し、当該 error 応答を `PartsMerged([Error]) + TurnCompleted(Failed{error})` へ変換。その他の error 応答も log::warn は残す。テスト固定

#### R3-7（major・N-9 残）Codex turn_id 同期の実装位置誤りで Stop 不能窓が残存
- `codex/session.rs:337`: `state.turn_id = convert_state.turn_id` の同期が変換イベントの for ループ**内**にあり、イベント 0 件の turn/started（`convert.rs:95-100`）では同期されない。turn 開始〜最初の delta（初回 reasoning 数十秒）間の interrupt() が turn_id=None で no-op、同窓の EOF は idle 誤分類
- 修正: 同期を `convert_jsonrpc_message` 呼び出し直後（ループ外）へ移動。turn/started 直後 Stop のテスト固定

#### R3-8（major・N-8 残）synthetic abort 後の旧 turn 遅延 result が次 turn を誤終端
- `claude/session.rs:431-438`: suppression が同一 turn_generation 限定のため、synthetic Abort → 同一 runtime で次 turn 開始（フラグリセット + gen+1）後に CLI が復活して旧 turn の result を emit すると素通りし、host の complete_turn（expected_generation=None）が新 turn を旧結果で誤終端（trait 契約 1「破棄済み turn の stale イベントは backend が破棄」違反）
- 修正: synthetic abort を出した世代を「破棄済み世代集合」として保持し、start_turn 後もその世代宛て TurnCompleted を破棄する（または synthetic abort 後は Fatal で runtime を捨て再 open に倒す）。回帰テスト追加

### P1: 機能欠落の残り

#### R3-9（major・C-2 残）Question 実経路で updatedInput.questions が空になる
- `claude/permission.rs:199-209` + `controller/permission.rs:325-338`: {questions, answers} 合成は実装されたが、frontend onAnswer は {answers} のみ送信 → controller が answers 除去後の**空 object を Some("{}")** で渡すため original_input フォールバック（None 時のみ）が働かず、CLI へ `questions=[]` が送られる（design §6.3 違反）。既存テストは updated_input=None の非実経路のみ
- 修正: questions の取得優先を「updated_input.questions → original_input.questions（pending_inputs）→ []」に変更（または controller で空 object を None に正規化）。実経路形（updated_input="{}"）のテスト + Deny fallback テストを追加

#### R3-10（major）task id 対応表（`agentId:` / `with ID:`）が実装ごと欠落
- `claude/convert.rs:279-335`: design §6.2 の user tool_result 行が明示する「tool_result 本文からの task id 対応表の convert 内維持」が未実装。background task の TaskStatus が起動元 ToolUse に相関せず orphan 化（サブタスク進捗のネスト表示退行）
- 修正: ClaudeConvertState に task_id→tool_use_id map を実装し、旧 extract_agent_id テストの意味を移植

#### R3-11（major・F-5 残）initialize control_response が未処理で説明付き slash commands が届かない
- `claude/convert.rs:71-80` が TYPE_CONTROL_RESPONSE を捨てる。system/init の名前配列のみが供給源のまま
- 修正: control_response（request_id=releash-initialize）の response.commands を SlashCommandsUpdated へ変換しテスト固定

#### R3-12（major）init_sessions が open_tabs を破棄 → workflow step tab hydration（issues-1023）の退行
- `usecase.rs:839`（`let _ = open_tabs;`）: HEAD が init 時に実行していた `hydrate_open_workflow_step_tabs` 相当が production 全経路で未配線（allow 付き温存・呼出はテストのみ）。再起動後に step tab の keep-open / close 判定が「閉じている」前提で動く。併せて `InitSessionsResponse.permission_mode/plan_mode` が Edit/false 固定（`:851-852`）に変化
- 修正: init_sessions で hydration を実行し allow を除去。permission_mode/plan_mode を active session 由来に復元（or design へ逸脱記録）。再起動後 step tab の回帰テスト追加

#### R3-13（minor→P1 扱い）telemetry の design §8.2 要求分が未配線
- `telemetry/attributes.rs:62`: BackendSpawn/QueryInit/FirstBackendEvent/PermissionWait が未配線 dead（rename 済みだが記録実績なし）。permission wait 記録と query_init 近似は design 要求
- 修正: PermissionWait（要求→respond）と FirstBackendEvent（turn 開始→最初の runtime イベント）を event pump から記録。使わない variant は削除

### P2: テスト固定・構造・minor

#### R3-14（テスト群。G-3 残 = §15 必須の完遂）
1. **Fatal 後 queue 保全**: `controller.emit(Fatal)` 注入テスト 0 件（D-3/D-4 残と同根。close 呼び出し・queue 保全・再 open drain を固定）
2. **permission 遷移順序**: 送信失敗時に patch されないこと（mock の respond_permission が常に Ok で注入不能 → 失敗注入可能なテストダブルへ拡張）+ 成功時の patch→persist→記帳→flush→通知の順序（C-1/C-7 残含む: id 不一致経路）
3. **persist-first**: set_model / set_permission_mode の usecase テスト 0 件（C-5 resync 含む）
4. 変換表の行単位固定の残り: claude §6.2 残 9 行（thinking_delta / TodoWrite / user tool_result+task id / permission_denied / status permissionMode / result success / EOF 2 行 / keep_alive）、codex §7.2 残 ~11 行（reasoning / mcpToolCall / webSearch / dynamicToolCall / outputDelta 累積 / compacted / turn failed・interrupted / requestApproval 2 種 / EOF 2 行）
5. 個別残: A-1（queued drain の Streaming emit）/ A-3（presenter serde 形状）/ B-5（turn イベントの fresh-store round-trip）/ D-1（drain watchdog）/ D-6（rollback 直接）/ D-7（resume 判定・EOF・retry）/ D-9・N-8（旧 bridge-utils の 5 テスト意味移植 + 同語反復テスト解消）/ E-3（retry 系・byte 上限・1 秒 persist・終端 flush 順序）/ N-14（not-found 経路 Rust+TS）/ N-15b/j/l（retry 経路・canChangeBackend 更新・旧画像テスト 3 件）/ N-12・N-15m（旧 pid suite 残 + CleanupGate 3 件）/ G-9（parse 失敗）/ G-8 残 3 件（codex/permission.rs の英語名）

#### R3-15（構造。G-4 = NOT_FIXED）
- `usecase.rs` が 4,962 行へさらに肥大（review-02 比 +1,600）。event_apply.rs の名実不一致、scan_dir / RuntimeSessionPhase / JSONL 行読み loop の重複も温存。design §2.1 の分割か、design への逸脱記録（分割を後続 Issue 化）のどちらかを必ず行う

#### R3-16（minor 群）
| # | 場所 | 内容 |
|---|---|---|
| a | `usecase.rs:1184-1194` | ensure_runtime 失敗時に TurnInterrupted 記帳・state 通知なし（終端なし TurnStarted が残り保存 state が Active 固定） |
| b | `usecase.rs:2069-2085` | turn 中 Fatal の二重終端 emit + `let _ = set_session_state` の握り潰し |
| c | `usecase.rs:3377-3387` | ToolUse 含む delta の event log 全読込（retry 判定）。fileChange/patchUpdated 再 emit で偽 ToolCallRetried 記帳 |
| d | `claude/process.rs:185-190` | 1MB 行超過が Err→Crash+Fatal（メッセージ 1 件の skip に留めるべき。D-8 の skip と非対称） |
| e | `usecase.rs:1162` | `required_backend_id(...).unwrap()` の panic 経路 |
| f | `codex/models.rs:188-199` | ワンショット handshake write 失敗時に shutdown を通らず PID ファイル残留 |
| g | `codex/convert.rs:655-681` | `todo_items_from_value` の dead 残骸（§7.5 判断済みなら削除） |
| h | `pid_registry.rs:291-294` | owner 生存だが start_time 検証不能 → Stale(kill) に倒す（旧規則は保守的 skip）。`:160-163` env 未設定時の silent スキップ |
| i | `event_store.rs:106-107` | 破損末尾のエラー文言が実態と不一致 + edge case テストなし |
| j | `codex/wire.rs:32-47` / `protocol/agent.rs:23-32` | 使用済み定数への stale allow 5 件 / context-carry payload 型の二重定義 |
| k | `claude/session.rs:700-719` | start_turn リセットの同語反復テスト（実装経由に改修） |

---

## 修正の優先順位（次ラウンド）

1. **P0**: R3-1（deadlock。最優先）、R3-2、R3-3、R3-4、R3-5、R3-6、R3-7、R3-8
2. **P1**: R3-9、R3-10、R3-11、R3-12、R3-13
3. **P2**: R3-14（テスト群）、R3-15（構造 or 逸脱記録）、R3-16（minor 群）

**設計上の注意**: R3-1/R3-2 は「lock 臨界区間の出口設計」の問題。complete_turn を「lock 内では状態遷移と記帳まで」「通知・interrupt・close・workflow dispatch は guard drop 後」に再構成すること（部分パッチで通知だけ外すと R3-2 の同型が残る）。R3-6/R3-7/R3-8 は backend 状態機械への「世代・request id 相関」の追加であり、それぞれ回帰テストを同時に固定すること。
