## 要求

**種別**: バグ修正
**ゴール**: Workspace ナビのステータス表示が「現在タブが存在する（= オープン中の）Session のみ」を構築元として集約され、エラーになった Session をタブから閉じれば Workspace ステータスがその Session の Error 状態の影響を受けなくなる状態にする。
**背景**: 現状、`AgentStatusCenter` の `aggregate()` は worktree 配下の全 `SessionStatus` を集約対象にしており、`SessionState::Closed`（= AgentChatPanel でタブを閉じた Session）も含めてしまう。集約規則は Error > Waiting > Running > Done のため、過去にエラーになった Session が 1 つでも残っていると Workspace の `aggregated_state` が永続的に `Error` のまま固定され、ユーザーがエラー状態から復帰する手段がなくなってしまう。

### 現在の挙動
- worktree 配下のいずれかの Session で Error が発生する
- ユーザーが該当 Session のタブを閉じる（`close_session` で `SessionState::Closed` に遷移）
- `AgentStatusCenter.sessions` には Closed の `SessionStatus` が残り続け、`aggregate()` がそれも含めて集約するため、Workspace ナビのステータスが `Error` のままになる
- 他の Session を新規に開いたり既存の Session で正常に Done になっても、Workspace ステータスは `Error` から変化しない

### 期待する挙動
- `aggregate()` が `SessionState::Closed` の `SessionStatus` を集約対象から除外する
- ユーザーが Error になった Session のタブを閉じた瞬間に、その Session が Workspace の集約から外れ、残りのオープン中 Session の状態だけで `aggregated_state` が再計算される
- Closed の Session を `restore_session` で復帰させた場合は、再び集約対象に戻る
- worktree 配下のオープン中 Session（Closed を除外した Session）が 0 件のときは、Workspace ステータスは集約対象なしを示す

### 再現手順
1. ある worktree 配下の Session A でエラーを発生させる（`SessionState::Error`）
2. Workspace ナビのその worktree のステータスが Error 表示になることを確認する
3. AgentChatPanel で Session A のタブを閉じる
4. Workspace ナビのステータスが Error から変化せず、復帰できないことを確認する

### スコープ外
- `AgentStatusCenter.sessions` マップそのものから Closed Session を削除する変更は行わない（`SessionStatus` 自体は保持し続け、`aggregate()` のフィルタリングのみで対応する）
- `close_session` コマンド側からの `AgentStatusCenter` への明示的な削除呼び出しは行わない

## 振る舞い定義

```gherkin
Feature: Workspace ステータスのオープン中 Session 限定集約

  Rule: Workspace のステータスは現在オープン中の Session のみから決まる

    Scenario: エラーになった Session を閉じれば Workspace のエラー表示は解消する
      Given Workspace にエラー状態の Session と完了状態の Session がオープン中である
      And Workspace のステータスはエラーとして示されている
      When ユーザーがエラー状態の Session を閉じる
      Then Workspace のステータスは残るオープン中 Session の状態を反映するように変化する

    Scenario: 閉じた Session は Workspace のステータスにもう寄与しない
      Given Workspace に過去に閉じられた Session が残されている
      And 残るオープン中の Session はすべて完了状態にある
      When Workspace のステータスが評価される
      Then 閉じた Session の状態は Workspace のステータスに反映されない

    Scenario: 唯一のオープン中 Session を閉じれば Workspace は集約対象なしとして示される
      Given Workspace にオープン中の Session が 1 つだけ存在する
      And その Session はエラー状態にある
      When ユーザーがその Session を閉じる
      Then Workspace のステータスは集約対象なしとして示される

    Scenario: 同種のエラー Session が残っていれば Workspace のエラー表示は維持される
      Given Workspace にエラー状態の Session が複数オープン中である
      And Workspace のステータスはエラーとして示されている
      When ユーザーがそのうち 1 つの Session を閉じる
      Then Workspace のステータスはエラーのまま維持される

  Rule: 閉じた Session をユーザーが復帰させれば再び Workspace のステータスに寄与する

    Scenario: 閉じた Session を復帰させれば Workspace の集約対象に戻る
      Given Workspace に閉じられた Session が存在する
      And Workspace には他にオープン中の Session が存在する
      When ユーザーが閉じられた Session を復帰させる
      Then 復帰した Session はオープン中として扱われ Workspace のステータスへ再び寄与する
      And Workspace のステータスは復帰後のオープン中 Session 全体から再評価される

  Rule: オープン中 Session の状態は Error > Waiting > Running > Done の優先度で集約される

    Scenario: オープン中の Session にエラーが 1 つでも含まれていれば Workspace はエラーになる
      Given Workspace のオープン中 Session にエラー状態のものが含まれている
      When Workspace のステータスが評価される
      Then Workspace のステータスはエラーとして示される

    Scenario: オープン中の Session に待機と実行中のみが含まれていれば Workspace は待機を示す
      Given Workspace のオープン中 Session の状態は待機と実行中のみで構成されている
      When Workspace のステータスが評価される
      Then Workspace のステータスは待機として示される

    Scenario: オープン中の Session に実行中と完了のみが含まれていれば Workspace は実行中を示す
      Given Workspace のオープン中 Session の状態は実行中と完了のみで構成されている
      When Workspace のステータスが評価される
      Then Workspace のステータスは実行中として示される

    Scenario: オープン中の Session がすべて完了であれば Workspace は完了を示す
      Given Workspace のオープン中 Session の状態はすべて完了で構成されている
      When Workspace のステータスが評価される
      Then Workspace のステータスは完了として示される
```

## アーキテクチャ概要

### 責務配置
- `AgentStatusCenter`（`src-tauri/src/agent_status.rs`）: SessionStatus / WorkspaceStatus の中央管理と集約規則の適用、Tauri イベント・WS broadcast の発火を担当する。`SessionState` の真実値は決定しないが、SessionStore からの状態変更通知を受け取ったときは、自身が保持する `SessionStatus` スナップショットの `session_state` フィールドへその新値を反映する（=「最新化」する役割）。 / SessionStore の永続化、`SessionState` の値そのものを独自判断で書き換えること、タブ UI からの `close_session` / `restore_session` 呼び出しは担当しない。
- `SessionStore`（`src-tauri/src/session/store.rs` / `session/mod.rs`）: ChatSession の永続化と `SessionState` の確定を担当し、状態が変化した事実を観測可能なシグナルとして外部に公開する。 / 集約規則の適用、Workspace 状態の保持・emit は担当しない。
- `close_session` / `restore_session`（`src-tauri/src/session/mod.rs`）: SessionStore 上の `SessionState` を Closed / Idle に遷移させ、`SessionStore` の状態変更通知を経由して再集約をトリガすることだけを担当する。 / `AgentStatusCenter` の内部マップを直接操作したり、aggregate の引数を組み立てたりしない。
- フロントエンド（`src/hooks/useWorkspaceStatus.ts` 系・`useWorktreeList.ts`・`useWorkspaceNavigation.ts` 等）: Tauri イベント `workspace-status-changed` / `session-status-changed` を購読し UI に反映するだけ。 / 集約・フィルタリングのロジックは持たない。

### データ/通信フロー
- ユーザーが Error Session のタブを閉じる: AgentChatPanel → `close_session` Tauri command → SessionStore が `SessionState::Closed` を保存 → SessionStore の状態変更通知 → AgentStatusCenter（購読側）が該当 SessionStatus の `session_state` を最新化し、Closed を除外した上で当該 worktree を再集約 → `session-status-changed` / `workspace-status-changed` を emit → フロント Hook が反映 → Workspace ナビが新しい aggregated_state を表示。
- Closed Session を復帰させる: AgentChatPanel → `restore_session` Tauri command → SessionStore が `SessionState::Idle` を保存 → SessionStore の状態変更通知 → AgentStatusCenter が該当 SessionStatus の `session_state` を Idle に最新化し再集約 → `session-status-changed` と `workspace-status-changed` の両方を emit → ナビ反映。
- 通常のターン進行（既存経路）: バックエンド bridge → `notify_status_transition` → `AgentStatusCenter::update_session` → 集約 → emit。本フローは Closed フィルタの影響を受けるが構造は変えない。

### 状態Owner
- ChatSession の永続化済み `SessionState`（Active/Idle/Done/Error/Closed）: SessionStore（唯一の真実）。
- 実行時の SessionStatus（`turn_phase`, `agent_state`, `session_state` のスナップショット）と WorkspaceStatus（`aggregated_state` を含み、集約対象が 0 件のときは「集約対象なし」を表現する）: AgentStatusCenter（メモリ上）。
- Workspace ナビの表示状態: フロントエンド Hook（`AgentStatusCenter` から流れてくるイベントのキャッシュ）。
- 「どの Session タブが開いているか」: `SessionState` 自体（Closed か否か）が事実上のソース。タブ UI は SessionStore の状態に従う。
- SessionState と振る舞い定義の用語対応: Active → 実行中、Idle → 待機、Done → 完了、Error → エラー、Closed → 閉じた。集約規則「Error > Waiting > Running > Done」はそれぞれ SessionState の Error > Idle > Active > Done に対応する。

### 境界
- 集約規則（Error > Waiting > Running > Done）と「Closed を除外する」フィルタは `AgentStatusCenter::aggregate`（およびその呼び出し側のフィルタ責務）にのみ存在し、フロントエンドや SessionStore は知らない。Closed を除外した結果オープン中 Session が 0 件になった場合は、WorkspaceStatus の `aggregated_state` を「集約対象なし」として表現する責務も `AgentStatusCenter` 側にあり、フロントエンドはその値を表示するだけで独自に補正しない。
- SessionStore は `SessionState` の値を保持・公開するだけで、それが「集約対象か否か」の解釈は持たない。AgentStatusCenter 側が `SessionState::Closed` を集約対象から外す唯一の主体である。
- 永続化と中央管理の同期は SessionStore からの状態変更通知（一方向）に集約する。AgentStatusCenter から SessionStore への書き込みは行わない。
- フロントエンドは `workspace-status-changed` / `session-status-changed` の最新値だけを信頼し、独自に集約・補正しない。

### 実装に委ねること
- SessionStore の状態変更通知の具体的な仕組み（`tokio::sync::broadcast` / コールバックリスト / `parking_lot` で守るリスナー Vec など）と、その購読側（AgentStatusCenter）への配線位置。
- AgentStatusCenter が SessionStore からの通知を受けたときに、`update_session` を再利用するか、Closed 専用の内部経路を用意するか（外部 API シグネチャを変えない範囲での選択）。
- `aggregate` への Closed フィルタの実装位置（`aggregate` 呼び出し前の `Vec` 構築段階で除外するか、`aggregate` 内で除外するか）。
- 既存テストへの追記か新規テストファイル追加か、ヘルパー関数（`mk_session` 等）の再利用範囲。
- 通知の重複発火を防ぐ dedup の判断（既存 `is_session_state_equivalent` を流用するか専用判定を追加するか）。
- ログ出力の有無・粒度。
