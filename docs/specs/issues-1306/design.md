# Design

関連: #1306 / requirements.md / behavior.md / milestone [12] / Blocks: #878

## 概要

terminal pane tree の state ownership を、UI layout（frontend 所有）と PTY session lifecycle（Rust が source of truth）に分離するリファクタリングの設計を定める。外部から観測可能な terminal の振る舞いは原則不変とする。

現状、`src/hooks/useTerminalPanes.ts` が以下 2 つの PTY lifecycle 正典判定を frontend 側で再計算している。

1. **stale 判定**: `list_pty_sessions` の結果から live session 集合を組み立て、pane の `sessionKey` がそこに含まれないものを「stale」と判定して layout から除去する（`removeStalePanes` / `removePaneBySessionKeyFromState`）。
2. **eviction 後の cache 整合**: `pty-evicted` event を受けて、表示中 state だけでなく `tabStateCache`（layout + session を同居させた module-global cache）も独自 reconcile で書き換える（`ensurePtyEvictionMirror` / `removeEvictedSessionFromTabStateCache`）。

本設計の方針は次の通り。

- **stale / live の判定（reconciliation 判定）を Rust の usecase に移す。** frontend は「自分の pane が参照している session_key の集合」を Rust に渡し、Rust が「利用不能（unavailable）な session_key」を返す。frontend はその結果を pure layout 操作として適用するだけにする。set-difference という判定計算を frontend から撤去する（要求 #2 / #3 / #5）。
- **eviction は `pty-evicted` event（backend 判定済み）を pure layout 操作として適用するだけにする。** frontend 独自の cache mirror（`tabStateCache` の lifecycle 書き換え）を撤去する（要求 #3 / #4）。
- **layout cache（`tabStateCache`）を PTY 生存判定で書き換えない構造にする。** unmount 中の cached layout に残った stale pane は、再 mount 時の reconcile reflection で除去する（要求 #4）。
- `src/lib/paneTree.ts` は pure UI layout helper のまま維持する（要求 #1）。

## 変更対象

### Rust（backend）

- `src-tauri/src/usecase/pty_session/query_service.rs`（または同 module 内の新規ファイル）
  - PTY session availability reconciliation の pure 関数を追加する。
- `src-tauri/src/usecase/pty_session/dto.rs`
  - reconciliation の入出力 DTO を追加する（必要に応じて）。
- `src-tauri/src/adaptor/controller/command/pty_session/commands.rs` および `mod.rs`
  - reconciliation を呼ぶ Tauri command を 1 つ追加し、`invoke_handler` / allowlist に登録する。

### Frontend

- `src/hooks/useTerminalPanes.ts`
  - `removeStalePanes` / `removePaneBySessionKeyFromState` の stale 算出ロジックを撤去し、Rust reconciliation の結果を適用する反映処理に置き換える。
  - `ensurePtyEvictionMirror` / `removeEvictedSessionFromTabStateCache` および module-global eviction listener / token 管理（`ptyEvictionMirrorPromise` / `ptyEvictionMirrorUnlisten` / `ptyEvictionMirrorToken`）を撤去する。
  - `pty-evicted` event handler を「該当 session_key の pane を layout から除去する」pure layout 適用に限定する。
- `src/types/terminal-pane.ts`
  - 型整理（後述）。binding field は leaf に残すが、layout / binding の意味づけをコメントまたは型分割で明確化する。
- `src/hooks/useTerminalPanes.test.ts` / `src/lib/paneTree.test.ts`
  - test 方針（後述）に合わせて追加・更新する。

### 非対象（不変）

- `src/lib/paneTree.ts`（pure helper のまま）。
- `src/hooks/useTerminal.ts` の PTY spawn / IO ロジック。`pty-evicted` を自身の `ptyId` 判定に使う既存経路は本変更の reconciliation 対象外であり、変更しない。
- PTY registry / eviction policy / buffered output 等の backend 挙動。

## アーキテクチャと責務分割

```
[Rust: source of truth]                         [Frontend: layout 所有 + 反映]
PtySessionRuntimeGateway (registry/snapshot)
   │ list_snapshots()
   ▼
usecase::pty_session::query_service
   - list()                ── availability read model (既存)
   - reconcile_unavailable() ── NEW: 判定（referenced − live = unavailable）
   │
   ▼ (command)
reconcile_pty_sessions(session_keys) -> Vec<String(unavailable)>
   ▲                                              │ invoke
   │                                              ▼
pty-evicted event (backend 判定済み) ───────────► useTerminalPanes
                                                  - referenced keys を集めて invoke
                                                  - unavailable / evicted session_key を
                                                    paneTree helper で layout から除去（pure apply）
                                                  - tabStateCache は layout だけを保持
```

責務境界:

- **判定（Rust）**: 「どの session が live か」「frontend が参照している session_key のうちどれが unavailable か」を Rust が決定する。eviction の判定は `pty-evicted` event の発行（backend）で既に Rust 側にある。
- **適用（Frontend）**: Rust が返した unavailable 集合 / evicted session_key を、`paneTree` の pure layout helper（`closePane` / `getAllLeaves` 等）で layout に反映するだけ。どの pane を消すかの「判定」は持たず、backend が示した session_key への「対応付け（layout 上の除去）」だけを行う。

## データモデルまたは型

### Rust DTO

reconciliation command の入出力は最小限にする。判定対象は session_key の集合で十分なため、live snapshot の全 field（`pty_id` / `worktree_path` / `label` / `kind`）は不要。

```rust
// usecase/pty_session/dto.rs（新規追加）
#[derive(Clone, serde::Serialize)]
pub struct PtySessionAvailability {
    /// frontend が参照している session_key のうち、現在 live でないもの。
    pub unavailable_session_keys: Vec<String>,
}
```

判定関数（pure、Rust test 対象）:

```rust
// usecase/pty_session/query_service.rs（新規追加）
pub fn reconcile_unavailable(
    manager: &impl PtySessionReadGateway,
    referenced_session_keys: &[String],
) -> PtySessionAvailability {
    let live: HashSet<String> = manager
        .list_snapshots()
        .into_iter()
        .map(|s| s.session_key)
        .collect();
    let unavailable = referenced_session_keys
        .iter()
        .filter(|key| !live.contains(*key))
        .cloned()
        .collect();
    PtySessionAvailability { unavailable_session_keys: unavailable }
}
```

> 仮定: 判定は `session_key` の live 集合への所属で行う。現状 `removeStalePanes` も `liveSessionKeys.has(sessionKey)` と同等の判定であり、挙動を変えない。exited / kind 等で live 集合を絞る必要があるかは Open Questions 参照。

command:

```rust
// adaptor/controller/command/pty_session/commands.rs（新規追加）
use crate::adaptor::controller::state::AppState;
use crate::usecase::pty_session::dto::PtySessionAvailability;
use tauri::State;

#[tauri::command]
pub fn reconcile_pty_sessions(
    state: State<'_, AppState>,
    session_keys: Vec<String>,
) -> PtySessionAvailability {
    state
        .pty_session_read_usecase
        .reconcile_unavailable(&session_keys)
}
```

`PtySessionReadUsecase` は `lib.rs`（composition root）で read gateway を注入して `AppState` に保持する。controller は `docs/architecture/CONTROLLER.md` の規約どおり usecase 呼び出しに限定し、`query_service` / gateway を直呼びしない。

`list_pty_sessions`（既存）は残す。availability read model として他用途で再利用可能であり、本変更では撤去しない（frontend の唯一の consumer は `useTerminalPanes` だが、reconcile に置き換わるため呼び出しは消える）。

### Frontend 型（`terminal-pane.ts`）

`PaneLeaf` の binding field（`ptyId` / `sessionKey` / `pendingKill`）は、A3 の通り「layout-adjacent な参照」として leaf に残す。これらは pane が PTY を参照するための binding であって、live/stale の判定値ではない。型としては現状維持しつつ、binding field であることをコメントで明示する。

```ts
export interface PaneLeaf {
	type: "leaf";
	id: string;
	label: string;
	// --- PTY binding（layout-adjacent な参照。live/stale 判定値ではない） ---
	ptyId: number | null;
	sessionKey: string | null;
	pendingKill?: boolean;
}
```

> 仮定: binding を別 map（`Map<paneId, PtyBinding>`）へ分離する案も検討したが、(a) 既存の `updatePaneSessionKey` / `killPanePty` / `markPendingPaneKill` が leaf 上の binding に密結合しており、分離は本変更のスコープ（reconciliation 撤去）を超える、(b) 要求 #4 が禁ずるのは「PTY 生存判定が layout cache を書き換えること」であり、binding を leaf に持つこと自体は禁じていない、ため leaf 保持を採用する。binding を別 state へ分離するのは #878 以降の dead-code sweep に委ねる。

### layout cache（`tabStateCache`）

`CachedTabState` は現状のまま layout（`tabs` / `activeTabId` / counter）のみを保持する。本変更で `tabStateCache` を lifecycle（eviction）から書き換える経路（`removeEvictedSessionFromTabStateCache`）を撤去するため、cache は「layout だけが入る」構造として明確化される。session 生存状態を表す独立 state は frontend には持たない（source of truth は Rust）。

## 処理フロー

### 1. mount 時 / availability reflection

```
useTerminalPanes mount（cacheKey あり）
  → 現在の tabs から参照中 session_key を列挙（getAllLeaves → sessionKey 非 null）
  → invoke("reconcile_pty_sessions", { sessionKeys })
  → 返り値 unavailable_session_keys を、順に paneTree helper で layout から除去
      - 各 session_key について該当 leaf を特定 → closePane（または最後の 1 pane なら clear）
      - focus / activeTab を layout 規則に従って整える
  → tabStateCache は除去後の layout を反映（layout 操作の結果としてのみ更新）
```

旧 `removeStalePanes` の while ループ（stale を 1 件ずつ探して除去）は、Rust が返した unavailable 集合を反復適用する形に置き換える。frontend 側に残るのは「session_key → 該当 pane の layout 除去」という適用のみで、stale の算出は行わない。

### 2. eviction reflection（`pty-evicted`）

```
pty-evicted(session_key) 受信
  → 表示中 tabs から該当 session_key の pane を paneTree helper で除去（pure layout apply）
  → tabStateCache は除去後 layout を反映（useEffect 経由の既存 cache 更新で十分）
```

旧経路の module-global mirror（unmount 中の cached worktree に対する書き換え）は撤去する。unmount 中に evict された session が cached layout に残っても、その worktree を再 mount した時点で「1. availability reflection」が走り、unavailable として除去される。これにより「layout cache を lifecycle が直接書き換えない」（要求 #4）と「evict 後の結果整合」（要求 #7）を両立する。

> 挙動差分の確認: 旧 mirror は unmount 中でも即座に cached layout を更新していた。新設計では再 mount 時まで cached layout の除去が遅延する。ただし cached layout は画面に出ていない（unmount 中）ため、外部から観測可能な差分は「再 mount 後の最終 layout」に限られ、そこは reconcile により同一に収束する。behavior.md「evict された session の pane が layout から除去される」「tab 切り替えで layout が復元される」は、再 mount 時 reconcile で満たされる。この遅延を許容する前提を仮定として置く（requirements A5）。

### 3. 最後の 1 pane のクリア

`reconcile_pty_sessions` が最後の 1 pane の session_key を unavailable として返した場合、frontend は該当 pane を「除去」ではなく「clear（`ptyId` / `sessionKey` を null 化して空 pane 化）」する。これは現行 `removePaneBySessionKeyFromState` の単一 pane 分岐と同じ layout 規則であり、pure layout 適用として維持する（behavior.md「最後の 1 pane の session が利用不能になった場合はクリアされる」）。

### 4. pane close 時の PTY 終了（不変）

`closeSpecificPane` / `closeTab` / `killPanePty` / `killPaneTreePtys` の `kill_pty` invoke は現状維持する。これは layout 操作に伴う観測可能な副作用であり、reconciliation とは独立（behavior.md「pane close 時に対応する PTY が終了される」）。

## エラー処理

- `invoke("reconcile_pty_sessions")` 失敗時は、現行 `list_pty_sessions` 失敗時と同様に `console.error` でログし、layout を変更しない（reconcile しない）。reconcile は「stale を消す」最適化であり、失敗しても layout の一貫性は保たれる（pane は残るが、後続の mount / eviction event で収束する）。
- `pty-evicted` listen の登録失敗時も現行同様 `console.error` のみ。
- Rust command はエラーを返さない（live 集合との比較は失敗しない）。`Result` を返さず値を返す（既存 `list_pty_sessions` と同形）。
- module ごとの専用 error type 規約に従い、新規 usecase 関数はエラーを返さない pure 関数のため独自 error type を増やさない。

## テスト方針

### Rust（要求 #5）

`src-tauri/src/usecase/pty_session/query_service.rs` の `#[cfg(test)]` に、`reconcile_unavailable` の reconciliation を検証する test を追加する。既存の `MockGateway`（`list_snapshots` を制御可能）を再利用する。

- referenced のうち live 集合に無い key だけが unavailable として返る。
- referenced 全てが live なら unavailable は空。
- referenced が空なら unavailable は空。
- live 集合に有るが referenced に無い key は返らない（frontend が参照しない session には干渉しない）。

eviction reconciliation は `lifecycle_usecase::evict` の既存 test（`emit_evicted` 呼び出し・snapshot 除去）で backend 判定を担保する。reconcile 後に evicted session が live 集合から外れること（= 次の reconcile で unavailable になる）を `list_snapshots` 連動で確認する。

### Frontend（要求 #6）

`src/hooks/useTerminalPanes.test.ts`:

- (a) layout interaction: add/close tab、split/close/move pane、focus 移動、上限（`MAX_TABS` / `MAX_PANES_PER_TAB`）— 既存 test を維持。
- (b) backend event reflection:
  - `reconcile_pty_sessions` が返した `unavailable_session_keys` の pane が layout から除去される（mock invoke で unavailable を返す）。
  - reconcile 結果が空なら layout 不変。
  - `pty-evicted` event の session_key を参照する pane のみが除去され、他 pane は維持される。
  - 最後の 1 pane が unavailable のときは clear される。
  - tab 切り替えで cache から layout が復元される（layout cache が reconcile/eviction で壊れない）。
- 撤去に伴い、`list_pty_sessions` を前提とする既存 test（行 761 周辺）、global mirror の既存 test（行 713 / 641 周辺）は、新 API（`reconcile_pty_sessions`）と新挙動（mirror 撤去・再 mount 時収束）に合わせて書き換える。

`src/lib/paneTree.test.ts`:

- 既存の pure helper test を維持し、paneTree に session 判定が入っていないことを担保する（要求 #1 / behavior.md「paneTree helper は session 判定を行わない」）。helper の入出力が layout tree のみであることを確認する test を必要に応じて補強する。

### コマンド（要求 #8）

`pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を通す。

## リスクと代替案

### R1. reconcile 命令の形（採用案 vs 代替案）

- **採用案**: frontend が参照 session_key を渡し、Rust が unavailable を返す。判定（set-difference）が Rust に閉じ、要求 #2 / #3 / #5 を満たす。Rust test で reconciliation を直接検証できる。
- **代替案 A**: 既存 `list_pty_sessions`（live 集合）を返し、frontend が set-membership で適用。この場合 set-difference の「判定」が frontend に残り、要求 #5 の「Rust 側に reconciliation の test」が形骸化する。よって不採用。
- **代替案 B**: backend が pane 構成まで知り、除去すべき pane id を返す。pane layout は frontend 所有（要求の境界）であり、backend が layout を知るのは責務逆転。よって不採用。

### R2. eviction 反映の遅延（unmount 中 cache）

旧 mirror は unmount 中の cached layout も即時更新していた。新設計は再 mount 時 reconcile に委ねるため、cached layout の除去が遅延する。観測可能なのは再 mount 後の最終 layout のみで、そこは収束するため許容する（処理フロー 2 の注記）。万一「unmount 中も cache を即時整合させる」要件が判明した場合は、Rust が evict 時に「どの cacheKey の layout から外すか」を判定して frontend に通知する形へ拡張する（layout 所有を侵さないため、frontend が cache→referenced keys を渡して Rust が unavailable を返す再 reconcile を全 cacheKey に対して行う案）。本変更では採らない。

### R3. reconcile のタイミング多重化

mount 時の reconcile と `pty-evicted` event 反映が近接して走る可能性がある。両者とも pure layout 適用（冪等：既に除去された session_key は no-op）であり、二重適用しても layout は壊れない。`removeStalePanes` の while ループに相当する反復適用は冪等性で安全。

### R4. 既存 test の大幅書き換え

mirror 撤去・command 変更により既存 frontend test の一部を書き換える。挙動契約（behavior.md）を基準に、内部関数ではなく観測可能な結果（layout への反映）を検証する test へ寄せることで、リファクタ耐性を上げる。

## 仮定

- A1. reconciliation 判定は `session_key` の live 集合所属で行う（現行 `removeStalePanes` と同等）。live 集合は `list_snapshots()` 全件とし、exited / kind による追加の絞り込みは行わない（現行と挙動を変えない）。
- A2. `list_pty_sessions` command は残置する（availability read model として再利用余地があるため）。frontend からの呼び出しは reconcile へ置き換わり消える。
- A3. PaneLeaf の binding field（`ptyId` / `sessionKey` / `pendingKill`）は leaf に保持する。binding の別 state 分離は本変更スコープ外（#878 系へ委譲）。
- A4. unmount 中 cached layout の stale pane は再 mount 時 reconcile で除去する（即時 mirror は撤去）。観測可能な最終 layout は収束するため許容する。
- A5. `reconcile_pty_sessions` は `Result` を返さず値を返す（既存 `list_pty_sessions` と同形）。reconcile 失敗時は frontend が layout を変更しない。
- A6. 検証は CI と同じコマンドで行う（requirements A6）。

## Open Questions

なし。
