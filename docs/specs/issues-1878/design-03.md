# Design 03

## 開始状態

直前の Design は `docs/specs/issues-1878/design-02.md`。

- 差分の基準は base ブランチ `main` からの派生点 `1ad87359`（branch `feat/issues/1878`）。
- design-01 と design-02 が挙げた変える部分は実装済みであり、その未コミットの作業ツリー（`1ad87359` からの 108 ファイル、+3350 / -7586）がこの周の開始状態である。Spec 工程ではコードを変更していない。
- design-02 が挙げた 8 件は開始状態で満たされている。gateway の実 stream 経路のテスト（`src-tauri/src/adaptor/gateway/state_client_test.rs:69`）、usecase の並行 add/remove 順序のテスト（`src-tauri/src/usecase/repo_paths_usecase.rs:159-160`）、client 内の受信者の購読 id 単位での保持（`src-tauri/src/usecase/state_client.rs:25,44-47`）、UI の初回取得の単一契機化（`src/hooks/useWorkspaceList.ts:94-96`）、時刻 source の port 化（`src-tauri/src/usecase/state_subscription.rs:12-22` の `SubscriptionTimer`）、転送値の型名（`StateValue`）、外部 stream との対話 port の domain への移設（`src-tauri/src/domain/state_subscription/connection.rs:13,24`）、外向き通知の gateway からの `StateSubscriptionUsecase` 依存の除去（`src-tauri/src/adaptor/gateway/repository/notify.rs:8-11`）がいずれも反映されている。
- この周までに解消となった Thread は design-02 が扱った 8 件（`37a69c0c-7790-44a1-bdaf-03610ea95bc1`、`41122965-0a9d-409e-8bac-55df100fe585`、`dc763be7-1831-4883-aeea-c380bc8e6ef8`、`992a0d46-7ada-4f16-b9d4-9f9beb24ee7a`、`38dfa1f9-c7fc-48db-9f07-09de292ce6e0`、`8a2293e4-7405-4030-a178-9d24ffb501a7`、`836b0343-4daa-438c-9fe9-0fb7a56ffbe3`、`4515b38f-cea1-4cd2-9b1e-80eebcc77667`）と design-02 の開始時点で閉じていた `7c2221af-c5bc-49f1-9945-e3b0591d900f`。見送り（`[DEFERRED]`）・不成立（`[REJECTED]`）とした Thread は無い。

## 変える部分

- 購読状態の変更と待機 stream の起床の単一操作化: 購読の状態を変えたうえで待機中の stream を起こす手順を一つの操作として提供し、呼び出し側が片方だけを実行できない形にする。根拠: Thread `2f05086e-3acc-4f8a-90d1-df4aa3a81134`（`StateSubscriptionUsecase` が `state`（`Subscriptions<StateValue>`）と `changed`（`Notify`）を別々の `pub(crate)` フィールドで公開し（`src-tauri/src/usecase/state_subscription.rs:31-32`）、`start` が「集約の更新 → `notify_waiters`」を一つの手順として実装する（同:59-60）一方、`RepoPathsNotifyGateway` が同じ 2 つを保持して `publish` → `notify_waiters` を同じ順序で再実装し（`src-tauri/src/adaptor/gateway/repository/notify.rs:9-10,20-26`）、`src-tauri/src/adaptor/controller/daemon.rs:192-200` が本番経路として配線している。`notify_waiters` は `src-tauri/src` 全体で `state_subscription.rs:60` と `notify.rs:25` の 2 箇所にあり、`Subscriptions::publish` の本番呼び出しは `notify.rs:23` だけである。`docs/architecture/README.md:32`「同じ操作の実装は 1 つに集約する」、`docs/architecture/USECASE.md:19`「操作の順序制御も usecase が持つ」）。ルート: 委任。

Requirements（R-001〜R-021）と Behavior（B-001〜B-022）は、この周で追加・削除・統合・緩和のいずれも行われていない。要求由来の変える部分は無い。

## 固定するルート

- design-01 で固定した次の 3 つを今周も維持する。
  - 購読の仕組みを一般的な作り（Kubernetes API の list + watch、xDS の ADS）に合わせる。
  - 今は server streaming で作り、購読の開始と停止は単発の呼び出しで送る。
  - R-019（Repository の追加・削除が Workspaces の表示へ反映される）は UI 側（`src/App.tsx` と `src/hooks/useWorkspaceList.ts`）で起こす。
- design-02 で固定した版番号の生成方法を今周も維持する。対象ごとにメモリ上の連番を持ち、publish された値が現在の値と異なるときだけ 1 増やす。epoch は daemon の起動ごとに振る。別の起動の版および保持済み履歴より古い版を指定した場合は snapshot からやり直す。

今周の変える部分の修正方法（単一操作の置き場所、`StateSubscriptionUsecase` の `state` / `changed` の公開範囲の扱い、`RepoPathsNotifyGateway` が何を保持して何を呼ぶか、domain 側の `Subscriptions` と `Notify` の関係の表し方）は指定されていない。design-01 および design-02 が委任した購読の仕組みの内部設計（型、モジュール配置、proto の形、購読対象の識別方法）、購読ごとの送り待ちと溢れたときの再開の実装方法、印の間隔と client 側の無通信検出の判定、削除の手順と未使用コードの特定も、委任のまま維持する。

## 変えないもの

- `requirements.md` の R-001〜R-021 と `behavior.md` の B-001〜B-022 を変更しない。理由は、`2f05086e` の指摘が実装境界の是正だけで解決でき、外部から観測可能な結果を変えないためである。関係する B-017（Repository のパス一覧が購読で届き、変わるたびに変わった後の一覧が届く）、B-008（届く状態・変更・印に版番号が付く）、B-020（Repository の追加・削除が Workspaces の表示へ反映される）はいずれも変わらない。
- state stream の本数に上限を設けない。design-02 の「変えないもの」をそのまま維持する。
- R-013 / B-014 は daemon 側の購読の 1 件化を定めるものとして維持する。client 内部で受信者へ配る方法は R-013 / B-014 の範囲ではない。
- design-01 の「変えないもの」を維持する。`ListBranchesWithStatusSnapshot` は削除しない。デッドコードの削除について `behavior.md` に受入条件を追加しない。

## 未確定・リスク

なし。この周で自動判断した箇所、未決のまま残した要求、`[DEFERRED]` で人間へ渡した件は、いずれも無い。
