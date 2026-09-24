# Design 02

## 開始状態

直前の Design は `docs/specs/issues-1878/design-01.md`。

- 差分の基準は base ブランチ `main` からの派生点 `1ad87359`（branch `feat/issues/1878`）。
- design-01 が挙げた変える部分は実装済みであり、その未コミットの作業ツリー（`1ad87359` からの 106 ファイル、+2984 / -7574）がこの周の開始状態である。Spec 工程ではコードを変更していない。
- この周までに解消となった Thread は `7c2221af-c5bc-49f1-9945-e3b0591d900f`（state stream の本数に上限が無い）。「state stream の本数に上限を設けない」という決定により `resolved` で閉じている。見送り（`[DEFERRED]`）・不成立（`[REJECTED]`）とした Thread は無い。
- 版番号の生成は開始状態で確定した形を満たしている（`src-tauri/src/domain/state_subscription/mod.rs:202-210` が対象ごとのメモリ上の連番を値の変化時だけ 1 増やし、`src-tauri/src/usecase/state_subscription.rs:30` が daemon の起動ごとに epoch を振る）。

## 変える部分

- gateway の実 stream 経路のテスト追加: `StateClientGatewayImpl` が実 stream を開き、同じ接続の client_id で購読を開始・停止する経路を検証するテストを加える。根拠: Thread `37a69c0c-7790-44a1-bdaf-03610ea95bc1`（`StateClientGatewayImpl` は `src-tauri/src/desktop.rs:83` の本番配線でしか構築されず、`src-tauri/src/adaptor/gateway/state_client_test.rs` は `decode` だけを検証する。`docs/architecture/TEST.md:26` は adaptor/gateway のテストを必須とする）。ルート: 委任。
- usecase の並行 add/remove 順序のテスト追加: `RepoPathsUsecase` の mutation lock が保証する並行 add/remove の通知順序を検証するテストを加える。根拠: Thread `41122965-0a9d-409e-8bac-55df100fe585`（`src-tauri/src/usecase/repo_paths_usecase.rs:18,37-53` が変更・現在一覧の取得・通知を同じ Mutex で直列化する一方、同:109-158 のテストは逐次操作だけである。`docs/architecture/TEST.md:24` は usecase のテストを必須とする）。ルート: 委任。
- client 内の受信者の持ち方の修正: 同じ対象を別の購読 id で開始したときに、先行する受信者が置換されて以後の変更を受け取れなくなる状態を解消する。根拠: Thread `dc763be7-1831-4883-aeea-c380bc8e6ef8`（`src-tauri/src/usecase/state_client.rs:44,61-66` は subscriptions を target キーの HashMap とし、同じ target の start で id と receive を置換する。同:152-158 も target ごとに 1 受信者へだけ渡す。`src-tauri/src/adaptor/controller/command/client.rs:93-116` は呼び出しごとの id と Channel を受け、id 単位の停止を公開する）。ルート: 委任。
- UI の初回取得の重複解消: 起動時に `refresh_workspaces` が 2 回起こる状態を解消する。根拠: Thread `992a0d46-7ada-4f16-b9d4-9f9beb24ee7a`（`src/hooks/useWorkspaceList.ts:92-95` の repoPaths 到着による refresh と同:107-121 の listener 確立後の refresh は別の非同期契機であり、同:58-60 が実行開始時に pending を解除するため同一 pending にまとまらない）。ルート: 委任。
- usecase からの時刻 source の分離: 購読の usecase が時刻の source を直接所有しない形にする。根拠: Thread `38dfa1f9-c7fc-48db-9f07-09de292ce6e0`（`src-tauri/src/usecase/state_subscription.rs:76-95` が `tokio::time::interval` と `tick` を使い、`src-tauri/src/usecase/state_client.rs:98-115,164-169` が `tokio::time::sleep` / `timeout` を使う。`docs/architecture/USECASE.md:7` は時刻を usecase へ持ち込むことを認めない）。ルート: 委任。
- 購読の転送値の型名と所在の修正: QueryService の Response ではない購読の転送値から `Dto` の呼称を外す。根拠: Thread `8a2293e4-7405-4030-a178-9d24ffb501a7`（`src-tauri/src/usecase/state_subscription.rs:13-15` の `StateValueDto` は publish・受信・転送 payload に使われ、QueryService はこの型を返していない。`docs/architecture/USECASE.md:33,38` と `docs/architecture/DOMAIN.md:61` は DTO を QueryService が返す Response に限定する）。ルート: 委任。
- 外部 stream との対話 port の domain への移設: daemon の state stream との非永続な対話を表す port を domain 層に domain の言語で定義し、adaptor/gateway が実装する形にする。根拠: Thread `836b0343-4daa-438c-9fe9-0fb7a56ffbe3`（`src-tauri/src/usecase/state_client.rs:19-33` の `StateConnection` / `StateClientGateway` は connect・start・stop・receive で外部 daemon の stream を抽象化し、`src-tauri/src/adaptor/gateway/state_client.rs:21-120` が外部 RPC 接続として実装している。`docs/architecture/DOMAIN.md:96,110-112` は外部システムとの非永続な対話の port を domain 層へ置くと定め、同:104 の usecase 層へ置く port には当たらない）。ルート: 委任。
- 外向き通知の送信境界の修正: 外向き通知の gateway が別の usecase への単純転送になっている状態を解消する。根拠: Thread `4515b38f-cea1-4cd2-9b1e-80eebcc77667`（`src-tauri/src/adaptor/gateway/repository/notify.rs:4-17` は `StateSubscriptionUsecase` を保持し paths を無変換で `publish_repository_paths` へ渡す。`src-tauri/src/domain/repository/repository.rs:124-127` の port は送信手段の抽象のままである。`docs/architecture/GATEWAY.md:5` は変換しない処理を gateway に置かないこと、同:14-21 は外向き通知の Gateway 実装が infrastructure の送信実装を呼ぶことを定める）。ルート: 委任。

Requirements（R-001〜R-021）と Behavior（B-001〜B-022）は、この周で追加・削除・統合・緩和のいずれも行われていない。要求由来の変える部分は無い。版番号の生成方法は開始状態で確定した形を満たしているため、変える部分に入れない。

## 固定するルート

- design-01 で固定した次の 3 つを今周も維持する。
  - 購読の仕組みを一般的な作り（Kubernetes API の list + watch、xDS の ADS）に合わせる。
  - 今は server streaming で作り、購読の開始と停止は単発の呼び出しで送る。
  - R-019（Repository の追加・削除が Workspaces の表示へ反映される）は UI 側（`src/App.tsx` と `src/hooks/useWorkspaceList.ts`）で起こす。
- 版番号の生成方法を固定する。範囲は購読の版番号。粒度は次まで。対象ごとにメモリ上の連番を持ち、publish された値が現在の値と異なるときだけ 1 増やす。epoch は daemon の起動ごとに振る。別の起動の版および保持済み履歴より古い版を指定した場合は snapshot からやり直す。理由は、版を daemon が値の変化に対して振る形にすると、版の出所が対象の値の出所に依らなくなり、event store 由来の対象と git・ファイル由来の対象を同じ形で扱えるためである。design-01 の「版番号をどこから作るかは委任」をこの指定が置き換える。関係する要求は R-008、R-009、受入条件は B-008、B-009、B-010。

今周の変える部分 8 件の修正方法（テストの置き場所と構成、client 内の受信者の持ち方、UI の初回取得のまとめ方、時刻 source の抽象と所在、転送値の型名と所在、port の domain への移設方法、外向き通知の送信境界の作り方）は、いずれも指定されていない。design-01 が委任した購読の仕組みの内部設計（型、モジュール配置、proto の形、購読対象の識別方法）、購読ごとの送り待ちと溢れたときの再開の実装方法、印の間隔と client 側の無通信検出の判定、削除の手順と未使用コードの特定も、委任のまま維持する。

## 変えないもの

- state stream の本数に上限を設けない。理由は 3 点。攻撃条件が renderer での任意コード実行であり、その時点で同じ client token で状態を変える呼び出しも通せるため、stream の本数を絞っても塞がらない。正規に必要な本数を決める材料である client の識別が無く（`src-tauri/src/infrastructure/local_api/client_token.rs` の client token は単一の共有値）、識別は接続確立を扱う #1895 の範囲である。既存の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs:42`、`src-tauri/src/domain/terminal_surface/subscriptions.rs:3`）は 1 本の stream の中で持つ対象の数の上限であり、stream の本数の前例ではない。
- R-013 / B-014 は daemon 側の購読の 1 件化を定めるものとして維持する。client 内部で受信者へ配る方法は R-013 / B-014 の範囲ではない。理由は、`dc763be7-1831-4883-aeea-c380bc8e6ef8` の修正で R-013 / B-014 を読み替えないためである。
- R-016 / B-017、R-018 / B-019、R-019 / B-020 は維持する。理由は、`4515b38f`・`992a0d46`・`dc763be7` の修正がいずれもこれらの受入条件を変えずに行えるためである。
- design-01 の「変えないもの」を維持する。`ListBranchesWithStatusSnapshot` は削除しない。デッドコードの削除について `behavior.md` に受入条件を追加しない。

## 未確定・リスク

なし。この周で自動判断した箇所、未決のまま残した要求、`[DEFERRED]` で人間へ渡した件は、いずれも無い。
