# Design 01

## 開始状態

初回。差分の基準は `main` の `9524a6fa` から派生した `feat/issues/1953`。開始状態は `docs/specs/issues-1953/requirements.md` の Current Behavior に記載した `9524a6fa` のコードである。この周までに閉じた Thread は無い。

## 変える部分

- 拒否の記録を解く: 拒否を記録した後、次に呼び出しが枠に受理されたときに 1 回だけ同じキーを解く。拒否が残っていないときは解かない。根拠: R-001「拒否を失敗の記録へ出したあと、次に呼び出しが枠に受理されたときに 1 回だけ同じキーの記録を解く。拒否が残っていないときは解かない」、B-001。ルート: 下記「固定するルート」
- 包みの本体を common へ移す: `PriorityInterceptor`（`src-tauri/src/adaptor/controller/api/client_priority.rs:36-73`）の中身を `src-tauri/src/common/` の包みへ移す。根拠: R-002、B-002。ルート: 下記「固定するルート」
- 拒否と受理を伝える口を作る: 包みが `usecase::failure` と `domain::failure` を参照せずに失敗の記録へ出せるようにする。根拠: R-002「common は他のどの層も参照しない」。ルート: 下記「固定するルート」
- 優先度の分類と枠の大きさを Main へ移す: `client_priority.rs:6-13` の `TOTAL_SEATS`・`SHARES`・`QUEUE_LENGTH`・`limits()` と、`:15-34` の `priority_level` を Main へ移し、包みの設定として包みへ渡す。根拠: R-003、B-003。ルート: 下記「固定するルート」
- 包みの組み立てを Main へ移す: 枠の実体を作る `ClientApiDeps::new`（`src-tauri/src/adaptor/controller/api/client.rs:38`）と、包みを組み立てる `router`（同 `:149-156`）から組み立てを外し、Main（`src-tauri/src/adaptor/controller/daemon.rs` の `compose`）が組み立てて controller へ渡す。根拠: R-004、B-004。ルート: 下記「固定するルート」
- 使われなくなるコードを削除する。根拠: R-007、B-008。ルート: 委任

## 固定するルート

- common に置く包みは転送（connectrpc）の型を名乗らない。失敗の型を generic にして転送非依存の包みとして common に置き、`connectrpc::Interceptor` の実装だけを adaptor/controller 側の薄い橋渡しとして残す。橋渡しが持つのは、common の包みへ渡す値（呼び出しの名前、期限）と、拒否から `ConnectError` への変換だけである。
- 拒否と受理を伝える口は common 側に定義し、実装は adaptor に置き、Main が注入する。トレイトにするか関数にするかは委任する。
- 組み立ての場所は Main、すなわち `src-tauri/src/adaptor/controller/daemon.rs` の `compose` とする。`common::retry::RetryLimiter` の実体を作っているのと同じ場所である（`daemon.rs:29`）。
- 優先度の分類（method の名前）と、枠の大きさ・待ち行列の長さは、包みの設定として Main が包みへ渡す。common に置くのは枠を掛ける仕組みだけであり、ClientService の method の名前を common に持たせない。Kubernetes の API Priority and Fairness が、要求の分類（FlowSchema）と段ごとの枠の大きさ（PriorityLevelConfiguration）を、枠を掛ける仕組みとは別の設定として置くのと同じ形にする。
- 拒否の記録を解くのは、拒否が残っているときの次の受理の 1 回だけとする。受理のたびに解くと、`WriteTerminalSurface` のようなキー入力ごとの呼び出しで、失敗の記録の走査と、その target を購読している画面の読み直しが毎回起きる。`src-tauri/src/adaptor/gateway/desktop_client.rs:221-223` が、直前まで失敗していたときだけ解いているのと同じ形にする。拒否が残っているかをどこに持つかは委任する。

## 変えないもの

- 優先度の分け方、それぞれの枠の大きさ、待ち行列の長さ、枠の対象外の呼び出し。#1894 で決まっており、この ISSUE の実装境界で変えないと定めている。
- 失敗の記録の仕組み。画面へ返す一覧が `active` で絞らないこと、要対応の分類、記録をまとめる条件は今のままにする。拒否の記録で正すのは `active` の意味だけであり、画面の見え方は変えない。

## 未確定・リスク

なし
