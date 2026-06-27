# Behavior

関連: #1306 / requirements.md / milestone [12] / Blocks: #878

本変更は terminal pane tree の state ownership を「UI layout（frontend 所有）」と「PTY session lifecycle（Rust が source of truth）」へ分離するリファクタリングである。外部から観測可能な terminal の振る舞いは原則不変である。したがって本書は次の二種類の振る舞いを定義する。

1. **不変として維持される観測可能な振る舞い**（リグレッション防止の契約）。tab/pane の操作と上限、evict/消失 pane の最終的な layout 整合、close 時の PTY 終了、cache 復元の整合。
2. **新たに保証される責務境界の観測可能な不変条件**。session の live/stale/evicted 判定は Rust が source of truth として決定し、frontend はその判定結果を layout へ反映するだけである（frontend は独自に再計算しない）。

責務境界の不変条件は内部関数・cache 構造・IPC 形を behavior の対象とせず、外部から観測可能な性質（「frontend は backend が決定した unavailable set / eviction を反映するのみ」「layout cache が PTY 生存判定で書き換わらない」）として表現する。具体的な API 形・型整理・cache 分離の実装は design.md で確定する。

## 仮定

- A1. 本書のシナリオは terminal pane tree の UI layout 操作（add/close tab、split/close/move pane、focus 移動）と、PTY session availability の反映（backend が返す `unavailable_session_keys` の反映、`pty-evicted` event の反映）を対象とする。
- A2. backend は既に PTY session の source of truth を所有し、session availability（参照中の session_key のうち現在利用不能なもの）と eviction を read model / event として公開している（requirements A2）。
- A3. pane が自分に紐づく `sessionKey` / `ptyId` を「参照」として保持すること自体は layout-adjacent な binding として許容する。禁ずるのは、その binding の live/stale/evicted 判定を frontend が再計算することである（requirements A3）。
- A4. eviction / session 消失時に「stale pane を layout から除去する」最終的な layout 反映は frontend の layout 操作として残る。Rust が持つのは「参照中のどの session が unavailable / evicted か（reconciliation 判定）」であり、frontend はその判定結果を layout に適用するだけである（requirements A4）。
- A5. 上限は現行のまま `MAX_TABS = 8` / `MAX_PANES_PER_TAB = 4` とする。pane label の命名規則・表示フォーマットは現行のまま不変とする（requirements 非スコープ）。
- A6. 本変更は terminal の外部観測可能な振る舞いを不変に保つことを優先する。挙動差分が避けられない箇所が見つかった場合は本書 / design.md で明示して合意する（requirements A5）。

## Feature: terminal pane tree の layout 操作と PTY session availability 反映の観測可能な振る舞い

### Background

```gherkin
Background:
  Given terminal pane を持つ worktree が開かれている
  And backend が PTY session の source of truth を所有している
  And backend は参照中 session の unavailable set と eviction を read model / event として公開している
```

---

### Rule: tab の追加とクローズ（layout 操作）

tab は最大 `MAX_TABS = 8` まで追加できる。tab をクローズしても layout は一貫した状態を保つ。

```gherkin
Scenario: tab を追加できる
  Given 現在の tab 数が MAX_TABS 未満である
  When 利用者が tab を追加する
  Then 新しい tab が 1 つの pane を持って追加される
  And 追加した tab が focus される

Scenario: tab 数の上限を超えて追加できない
  Given 現在の tab 数が MAX_TABS（8）である
  When 利用者が tab を追加する
  Then tab は追加されず、tab 数は MAX_TABS のままである

Scenario: tab をクローズできる
  Given 複数の tab が存在する
  When 利用者がある tab をクローズする
  Then その tab が layout から除去される
  And 残りの tab のいずれかが focus される
```

---

### Rule: pane の分割・移動・クローズ・focus 移動（layout 操作）

1 つの tab 内で pane は最大 `MAX_PANES_PER_TAB = 4` まで分割できる。分割・移動・クローズ・focus 移動は純粋な UI layout 操作として振る舞う。

```gherkin
Scenario: pane を分割できる
  Given ある tab の pane 数が MAX_PANES_PER_TAB 未満である
  When 利用者が focus 中の pane を分割する
  Then 同じ tab 内に新しい pane が追加される

Scenario: tab あたりの pane 数の上限を超えて分割できない
  Given ある tab の pane 数が MAX_PANES_PER_TAB（4）である
  When 利用者が pane を分割する
  Then pane は追加されず、pane 数は MAX_PANES_PER_TAB のままである

Scenario: pane を移動できる
  Given ある tab に複数の pane が存在する
  When 利用者が pane を別の位置へ移動する
  Then layout tree 上で pane の配置が更新される
  And pane の総数は変わらない

Scenario: pane をクローズできる
  Given ある tab に複数の pane が存在する
  When 利用者がある pane をクローズする
  Then その pane が layout から除去される
  And 残りの pane のいずれかが focus される

Scenario: focus を移動できる
  Given ある tab に複数の pane が存在する
  When 利用者が隣接 pane へ focus を移動する
  Then focus 中の pane が移動先の pane に変わる
```

---

### Rule: pane close 時に対応する PTY が終了される

pane をクローズすると、その pane に紐づく PTY が終了される（`kill_pty`）。これは pane close に伴う観測可能な副作用として維持される。

```gherkin
Scenario: pane クローズで PTY が終了される
  Given PTY が紐づく pane が存在する
  When 利用者がその pane をクローズする
  Then 対応する PTY の終了が backend に要求される

Scenario: pane を含む tab クローズで配下の PTY が終了される
  Given 複数の PTY が紐づく pane を持つ tab が存在する
  When 利用者がその tab をクローズする
  Then その tab 配下の全 pane に対応する PTY の終了が backend に要求される
```

---

### Rule: session availability は Rust が決定し、frontend は unavailable set を反映する

どの session が live / stale / evicted かの判定は Rust が source of truth として決定する。frontend は `list_pty_sessions` の結果や `pty-evicted` event を独自に reconcile（stale 判定・cache mirror）せず、backend が決定した availability を layout へ反映するだけである。availability read model は live session set ではなく、frontend が参照している session_key のうち backend が利用不能と判定した `unavailable_session_keys` を表す。

```gherkin
Scenario: backend が unavailable と示した session の pane は layout から除去される
  Given ある pane が session_key を参照している
  And backend が提供する unavailable_session_keys にその session_key が含まれる
  When frontend が backend の session availability を反映する
  Then その pane が layout から除去される
  And どの pane を除去するかの判定は frontend ではなく backend の unavailable_session_keys に基づく

Scenario: backend が unavailable と示していない session の pane は維持される
  Given ある pane が session_key を参照している
  And backend が提供する unavailable_session_keys にその session_key が含まれない
  When frontend が backend の session availability を反映する
  Then その pane は layout に維持される

Scenario: 最後の 1 pane の session が利用不能になった場合はクリアされる
  Given tab に pane が 1 つだけ存在する
  And その pane の session_key が backend の unavailable_session_keys に含まれる
  When frontend が backend の session availability を反映する
  Then その pane はクリア（除去または空状態化）され、結果整合が保たれる
```

---

### Rule: eviction event を layout へ反映する

`pty-evicted` event を受け取ると、backend が決定した eviction を pane layout へ反映する。frontend は eviction 後の整合を独自計算しない。

```gherkin
Scenario: evict された session の pane が layout から除去される
  Given ある pane が session_key を参照している
  When backend が当該 session の pty-evicted event を発行する
  Then その session を参照する pane が layout から除去される

Scenario: evict 対象でない pane は影響を受けない
  Given session_key の異なる複数の pane が存在する
  When backend がそのうち 1 つの session の pty-evicted event を発行する
  Then 当該 session を参照する pane のみが除去される
  And 他の pane は layout に維持される
```

---

### Rule: layout cache は PTY 生存判定で書き換わらない

pane layout cache（tab/pane 構成・focus）と PTY lifecycle（session 生存）の state は分離されている。layout cache は PTY 生存判定によって暗黙に書き換えられない。tab を切り替えた際は cache から layout が一貫して復元される。

```gherkin
Scenario: tab 切り替えで layout が復元される
  Given 複数の tab があり、それぞれ固有の pane 構成と focus を持つ
  When 利用者が別の tab へ切り替え、元の tab に戻る
  Then 元の tab の pane 構成と focus が cache から復元される

Scenario: PTY 生存判定が layout cache を直接書き換えない
  Given layout cache に tab/pane 構成が保持されている
  When backend の session availability 反映が行われる
  Then layout cache の tab/pane 構成は、availability 反映による layout 操作の結果としてのみ変化する
  And PTY 生存判定が layout cache 構造を直接書き換えることはない
```

---

### Rule: paneTree helper は pure UI layout helper として維持される

`paneTree.ts` は layout helper（pane の探索・分割・クローズ・leaf 列挙・隣接取得等）のみを提供し、PTY lifecycle / session 判定ロジックを持たない。

```gherkin
Scenario: paneTree helper は session 判定を行わない
  Given paneTree helper が layout 操作（分割・クローズ・探索・隣接取得）を提供する
  When これらの helper が呼び出される
  Then helper は layout tree のみを入出力とする
  And helper は session の live/stale/evicted 判定を行わない
```

---

## Open Questions

なし。
