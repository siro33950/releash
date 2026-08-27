# Design

terminal 化した workflow run の `WorkflowExecution` を `executions` map から即時解放し、無制限増大経路 C10（#1196）を是正するための実装設計。requirements.md / behavior.md に準拠する。

## 概要

workflow runtime（`WorkflowRuntimeService`）は `executions: Mutex<HashMap<String, WorkflowExecution>>` に run の本体を常駐させる。現状、terminal 化（Completed / Failed / Aborted）後も本体（`step_history` / `step_outputs` / `workflow_variables` / `workflow_definition`）は通常経路では削除されず、run の進行・蓄積に比例して常駐メモリが無制限に増大する。

本設計の方針は次の 2 点に集約される。

1. **即時解放**: terminal 化と同時に、当該 run を `executions` map から `remove` する。terminal 化の副作用（step session release・Session 逆引き cleanup・状態 broadcast）が完了し、broadcast 用 snapshot を取り終えた直後に削除する。削除は冪等とし、全 terminal sink に共通ヘルパとして適用する。
2. **読み取り経路の不変性保証**: 履歴問い合わせ・状態問い合わせが解放後も同一結果を返すことを保証する。履歴問い合わせは既に Event Log / State Projection（永続化）から供給されており map に依存しないため影響を受けない。Aborted run では terminal 解放後も中断 step の `session_id` 等を復元できるよう、`RunAborted` event に optional な中断 step snapshot を持たせ、projection 境界で read model に変換する。`get_state_by_run_id`（gateway）は run_id のみを受ける live active 用 API に留め、terminal run の状態取得は controller 側で `worktree_path` 認可を通る `get_workflow_run_state` に寄せる。

この方針により「terminal run の本体が常駐し続けない」かつ「履歴問い合わせ・active run 進行という外部観測可能な振る舞いが不変」を両立する。

## 変更対象

- `src-tauri/src/adaptor/gateway/workflow/runtime_engine_impl.rs`
  - terminal sink への即時解放の挿入（下記 3 経路）と共通ヘルパ追加。
  - abort 経路で中断 step の event snapshot を `RunAborted` に書き込む。
  - `get_state_by_run_id` を in-memory live state 参照専用に保つ回帰テスト追加。
  - `#[cfg(test)]` 検証用アクセサの追加。
- `src-tauri/src/adaptor/gateway/workflow/event.rs`
  - `WorkflowEvent::RunAborted` に後方互換の optional `aborted_step` を追加する。
  - `aborted_step` は Event Log 専用 snapshot であり、`StepHistoryEntry` の表示用 state までは永続化しない。
- `src-tauri/src/adaptor/gateway/workflow/runtime_events.rs`
  - `StepHistoryEntry` から `RunAborted` 用 snapshot へ変換する mapper を追加する。
- `src-tauri/src/adaptor/gateway/workflow/event_projection.rs`
  - `RunAborted.aborted_step` が存在する場合は snapshot を優先して `StepHistoryEntry` へ変換する。
  - `aborted_step` が存在しない旧 event は既存の復元経路で扱う。
- 上記以外のファイルは原則変更しない。
  - `usecase/workflow/query_service.rs` ほか履歴問い合わせ経路は Event Log / State Projection 依存で map を読まないため変更不要（後述 [調査結果]）。
  - `execution_registry.rs` の `find_by_worktree` / `find_by_worktree_mut` は `is_active()` フィルタ済みで terminal run を返さない設計のため変更不要。
  - Run Store / 永続化ファイル配置は変更しない。Event Log schema は `RunAborted.aborted_step` の optional 拡張のみ例外として扱う。

## アーキテクチャと責務分割

レイヤー境界（`infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller`）は維持し、本修正は gateway（`WorkflowRuntimeService`）内に閉じる。

- **解放の責務**: gateway の terminal 遷移 sink。terminal 化の確定（required event の append commit）と副作用（session release / refs cleanup / broadcast）の後段に「map からの本体解放」を追加する。解放は in-memory 表の管理であり gateway の責務に閉じる。
- **再構築の責務**: 既存の永続化読み取り（Event Log → `reconstruct_state_from_events` → projected state）を gateway 内で再利用する。`RunAborted.aborted_step` は Event Log から read model へ射影する責務を `event_projection.rs` に閉じ、usecase / controller の契約は変更しない。
- **履歴問い合わせの責務**: 既に usecase `query_service` が Event Log / State Projection から供給しており、本修正の前後で経路・契約とも不変。

### terminal sink の集約

terminal 化が最終的に通る sink は次の 3 経路で、現状いずれも `executions` からの削除を行っていない。

| sink | 位置（現状） | 扱う status | 呼び出し起点 |
|---|---|---|---|
| `set_execution_state_inner` の terminal branch | L2703-2742 付近 | Completed / Failed（Aborted は除外） | `set_execution_state` ← 各 `handle_*` / command handler |
| `finalize_after_commit` の terminal branch | L3316-3329 付近 | Completed / Failed / Aborted | `persist_release_and_broadcast` / `execute_outcome`（on_turn_complete 系） |
| `finalize_terminal_transition_after_required_append` | L1948-1966 付近 | Aborted（AbortRun post-commit） | `abort_workflow_by_run_id` |

これら 3 sink に対し、broadcast 完了後の末尾で共通ヘルパ `release_terminal_execution(run_id)` を呼ぶ。各 sink は broadcast 用の snapshot（`to_workflow_state()` の結果）を削除前に取得済みであるため、削除によって broadcast 内容は影響を受けない。

- **冪等性**: `HashMap::remove` は対象不在でも安全。Completed/Failed が複数 sink を通過し得る経路（例: `set_execution_state` 経由と `finalize_after_commit` 経由）でも、二重削除は無害。これにより「全 terminal sink を漏れなく覆う（漏れ＝メモリ残留）／重複は無害」という設計原則で安全側に倒す。
- **active run の保護**: ヘルパは呼び出し側が terminal 確定後にのみ呼ぶ。ヘルパ内でも防御的に「対象 run が `is_terminal()` のときのみ削除」を確認し、active run を誤って解放しない。

## データモデルまたは型

Run Store schema と永続化ファイル配置は変更しない。Event Log schema は Aborted run の復元精度を保つため、`WorkflowEvent::RunAborted` に optional `aborted_step` を追加する。

- `WorkflowExecution`（`runtime_state.rs`）: フィールド・`is_active()` / `is_terminal()` / `to_workflow_state()` は不変。
- `executions: Mutex<HashMap<String, WorkflowExecution>>`: 構造不変。ライフサイクルのみ変更（terminal 化で entry を除去）。
- `RunAbortedStepSnapshot` / `RunAbortedChildOutputSnapshot`（`event.rs`）: Event Log 専用の snapshot。中断された通常 step / parallel child の `session_id`・`result`・`structured_output`・`run_index` 等を保持するが、read model 表示用の `state` は projection 境界で補う。`aborted_step` は `#[serde(skip_serializing_if = "Option::is_none", default)]` とし、旧 event との deserialize 互換を保つ。
- `event_projection.rs`: `RunAborted.aborted_step` が存在する場合は snapshot から `StepHistoryEntry` を構築し、存在しない場合は従来の current step / active parallel snapshot 由来の復元にフォールバックする。
- 共通ヘルパ（新規・private）:

  ```rust
  /// terminal 化した run を executions map から解放する。
  /// 全 terminal sink から broadcast 完了後に呼ぶ。冪等。
  async fn release_terminal_execution(&self, run_id: &str) {
      let mut execs = self.executions.lock().await;
      if let Some(exec) = execs.get(run_id) {
          if exec.is_terminal() {
              execs.remove(run_id);
          }
      }
  }
  ```

- `get_state_by_run_id` の live state 境界（既存 method の責務明確化）:

  ```rust
  pub async fn get_state_by_run_id(&self, run_id: &str) -> Option<WorkflowState> {
      let execs = self.executions.lock().await;
      execs.get(run_id).map(|e| e.to_workflow_state())
  }
  ```

  `get_state_by_run_id` は `get_workflow_state` command 経由で run_id のみを受けるため、map miss 時に Run Store + Event Log から terminal history を返すと worktree 認可境界を迂回する。terminal run の状態取得は `worktree_path` を必須にする `get_workflow_run_state`（controller → `authorize_run_summary_for_worktree` → query_service）で行う。

## 処理フロー

### terminal 化時（解放）

1. step / abort / state 設定の各経路で terminal status を確定。
2. required event を append commit（既存）。Aborted run では、commit 前に中断 step の snapshot を作成し、`RunAborted.aborted_step` として append する。
3. step session release・Session 逆引き cleanup（既存）。
4. broadcast 用 snapshot を取得済みの状態で `broadcast_state`（既存）。
5. **（追加）`release_terminal_execution(run_id)` を呼び、map から本体を除去。**

これにより terminal 化直後に `executions` から当該 run が消える。broadcast は手順 4 で完了済みのため、UI への terminal 状態通知は従来どおり行われる。

### terminal run の状態・履歴問い合わせ（解放後）

- **`get_run_log` / `get_run_state` / `get_output` / `get_step_detail` / `list_runs`（usecase `query_service`）**: Event Log / State Projection / Run Store から供給。map を読まないため解放の影響を受けず、解放前と同一結果。Aborted run の中断 step は `RunAborted.aborted_step` があればそれを read model へ変換し、旧 event では既存復元経路を使う。
- **`get_state_by_run_id`（gateway, `get_workflow_state` command 経由）**: in-memory map に存在する live run の状態だけを返す。terminal 化で release 済みの run は `None` になり、terminal history は `get_workflow_run_state` 経由で参照する。
- **worktree 起点の active run 検索（`get_state` / `find_by_worktree`）**: `is_active()` フィルタ済みで terminal run を元来返さない。解放により map から消えても「active としては返らない」という従来の振る舞いと一致する。

### 競合・並列

- terminal 化と状態問い合わせがほぼ同時の場合: map 取得が削除前なら live 値、削除後なら再構築値を返す。いずれも同一の terminal 状態であり不整合は生じない（behavior: terminal 化直後の状態問い合わせで不整合が生じない）。
- 複数 run の並列 terminal 化: 削除は run_id 単位で独立。Session 逆引き cleanup も run_id 主語のため他 run の refs を巻き込まない。active run の進行は他 run の解放の影響を受けない。
- terminal run の後からの再開（CLI dispatch `ensure_execution_loaded_for_external`）: 当該経路は `validate_run_record_for_external_restore` で terminal run を `InvalidState("already terminal")` として拒否する（実装確認済み）。したがって即時解放した terminal run が CLI dispatch 経由で map へ恒久再挿入されることはなく、解放方針と整合する。

## エラー処理

- `release_terminal_execution`: I/O を伴わない map 操作のみ。失敗経路を持たない（戻り値なし）。terminal でない run に対しては no-op。
- `get_state_by_run_id`: in-memory map の読み取りのみを行い、対象不在は `None` を返す。永続履歴の読み取りエラーはこの経路に持ち込まず、認可付き履歴 API 側で既存どおり扱う。
- terminal 副作用（session release / broadcast）は既存どおり best-effort（post-commit 失敗は warn）であり、解放追加によってこの方針は変えない。解放は副作用の後段に置くため、副作用失敗時も解放は実行され、メモリ解放が副作用結果に依存しない。

## テスト方針

`#[cfg(test)]` の検証用アクセサ（例: `async fn contains_execution_for_test(&self, run_id: &str) -> bool` および `async fn executions_len_for_test(&self) -> usize`）を `WorkflowRuntimeService` に追加し、map の membership / len を検査可能にする。これは「常駐メモリが run 数・step output 量に比例して積み上がらない」ことの決定的な代理指標（terminal run が map に残らない＝本体が常駐しない）として用いる。

- **解放（Rust, gateway）**
  - 単一 run を Completed させた後、`contains_execution_for_test(run_id) == false`。
  - Failed / Aborted それぞれで terminal 化させ、いずれも map から除去される（behavior の Scenario Outline に対応）。
  - 複数 run（N 件）を順次完了させた後、terminal run 由来の entry 数が増加しない（`executions_len_for_test` が run 数に比例しない／terminal 分は 0）。
  - step output が大きい run を完了させても entry が残らない（output 量に比例した常駐がない）。
- **履歴問い合わせの不変性**
  - terminal run に対する `get_workflow_run_state` が、解放後も worktree 認可を通したうえで terminal 状態を返す。
  - `RunAborted.aborted_step` が Event Log 専用 snapshot として serialize され、`StepHistoryEntry` の表示用 `state` を直接永続化しない。
  - projection が `RunAborted.aborted_step` を優先して `StepHistoryEntry` に変換し、中断 step の `session_id` を復元できる。
  - released terminal run に対する `get_state_by_run_id` は `None` を返し、run_id-only command から履歴状態を露出しない。
  - `get_run_log` / `get_run_state` / `get_output` / `get_step_detail` / `list_runs`（usecase）が解放前後で同一結果（既存テストの維持で担保しつつ、terminal 後参照のケースを補強）。
  - 同一 terminal run への問い合わせを繰り返しても結果が一貫する。
- **active run の不変性**
  - active run（Running / WaitingApproval）は解放されず map に残る。
  - worktree 起点の active run 検索が従来どおり該当 run を返す。terminal 化後は active 検索から外れ、履歴問い合わせからは参照できる。
- **競合**
  - terminal 化直後の状態問い合わせで terminal 状態が返り、エラー / 不整合が生じない。
  - 並列 run の相次ぐ terminal 化で取りこぼし（履歴参照漏れ）が生じない。
- **品質ゲート**: `cargo test` / `pnpm test` / `cargo clippy -- -D warnings` / `pnpm lint` が green。フロント変更はないが回帰確認として `pnpm test` を実行する。

## リスクと代替案

- **terminal sink の網羅性**: 3 sink の列挙が漏れていると、その経路で terminal 化した run が map に残りメモリ解放が不完全になる。緩和: 削除を冪等にし全 sink へ配置、かつ「複数 run 完了後に terminal entry が 0」を検証するテストで網羅漏れを検出する。実装フェーズで terminal 遷移の全経路（特に Aborted の複数入口）を再確認する。
- **run_id-only public command の認可境界**: `get_state_by_run_id` に永続 fallback を持たせると、`worktree_path` を受けない `get_workflow_state` から terminal history を読める。緩和: `get_state_by_run_id` は live active 専用に留め、terminal 状態は `get_workflow_run_state` の `authorize_run_summary_for_worktree` 境界へ寄せる。
- **代替案 A（`get_workflow_state` に `worktree_path` 認可を追加する）**: 既存 command signature と active run UI 経路の契約変更が大きいため不採用。本 issue では terminal history を既存の履歴 API へ寄せる。
- **代替案 B（直近 N 件保持 / 遅延解放）**: 表示応答性を優先する保持戦略。requirements / behavior で即時解放に確定済みのため不採用。即時解放で体感応答性の劣化が観測された場合にのみ再検討する（requirements の仮定に準拠）。
- **再構築コストの応答性**: terminal 化直後の表示は broadcast（terminal snapshot）で供給される。後続の履歴表示は Event Log / State Projection を読む既存履歴 API 経由で行うため、live API に再構築コストを持ち込まない。
- **Event Log schema 拡張の互換性**: `RunAborted.aborted_step` は optional かつ default 付きで、旧 `RunAborted` event は従来復元経路で扱う。緩和: serialize 形と projection の回帰テストで、snapshot が存在する場合の復元精度と存在しない場合の互換性を検証する。

## 仮定

- [仮定] terminal run の解放方針は「即時解放」で確定（requirements / behavior の合意済み仮定に準拠）。直近 N 件保持・遅延解放は採用しない。
- [仮定] terminal run の履歴問い合わせは、全 terminal status・全履歴問い合わせ経路について Event Log / State Projection / Run Store から従来と同一結果を供給できる。query_service の public 関数は map 非依存であり、controller の履歴 API は `worktree_path` 認可を通る。
- [仮定] `RunAborted.aborted_step` は Aborted run の中断 step 復元精度を保つための optional Event Log 拡張として扱う。Run Store schema と永続化ファイル配置は変更せず、旧 `RunAborted` event は `aborted_step: None` として従来どおり復元する。
- [仮定] terminal 化直後の表示は broadcast 由来であり、以後の terminal history は履歴 API 経由で取得される。
- [仮定] terminal run は CLI dispatch の restore（`ensure_execution_loaded_for_external`）で拒否されるため、即時解放後に map へ恒久再挿入されることはない（`validate_run_record_for_external_restore` の terminal 拒否を確認済み）。
- [仮定] 「常駐メモリが比例して積み上がらない」ことの検証は、`#[cfg(test)]` アクセサによる map membership / len 検査を決定的代理指標として用いる（実バイト計測の常設プロファイラは Non-goals）。

## Open Questions

なし（解放方針は「即時解放」で確定。terminal history は認可付き履歴 API に寄せる）。
