# Context

- 正本: [#1953 `[10] 同時実行の枠の拒否の記録が解けず、包みの本体と組み立てが controller にある`](https://github.com/siro33950/releash/issues/1953)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 10。
- 依存: [#1894 `[08] 同時実行の枠を優先度で分ける`](https://github.com/siro33950/releash/issues/1894)（closed）。優先度の分け方、それぞれの枠の大きさ、待ち行列の長さ、枠の対象外の呼び出しは #1894 で決まっている。
- 標準の参照先: [Kubernetes API Priority and Fairness](https://kubernetes.io/docs/concepts/cluster-administration/flow-control/)。#1894 が枠の設計の基準にした標準であり、要求の分類（FlowSchema）と段ごとの枠の大きさ（PriorityLevelConfiguration）を、枠を掛ける仕組みそのものとは別の設定として置く。
- 規約の正本は `docs/architecture/`。この変更が根拠にする規定は次のとおり。
  - `README.md`「部品の一覧」: common に置ける部品は「横断的関心事の包み」だけで、役割は「処理を外から包んで振る舞いを足す」。Main の部品は「配線（composition root）」で、役割は「全ての部品を組み立てる。最も外側」。
  - `README.md`「依存方向」: 「common はどの層にも依存しない」。「DI 配線（composition root）は Main の責務とし、gateway や controller へ配線責務を漏らさない」。
  - `README.md`「横断的な設計原則」: 「横断的関心事は、処理の中に書かず、包みとして掛ける。包みの定義は common に置く。掛ける位置は、受け手の側（controller の手前）、Usecase の入口、出ていく側（adaptor/gateway と infrastructure）の 3 つ」。
  - `CONTROLLER.md`「受け手の側の横断的関心事」: 「期限・取り消し・同時実行の制限・優先度・ログと計測は、common の包みを入口の手前に掛けて足す。handler の中に書かない」。
- この変更が前提にする、既に入っている変更。
  - #1928: 横断的関心事の包みの定義は `src-tauri/src/common/` にあり、common は他のどの層も参照しない（`docs/specs/issues-1928/requirements.md` の R-001）。この ISSUE の時点で `src-tauri/src/common/` の非テストコードに `crate::domain` / `crate::usecase` / `crate::adaptor` の参照は無い。
  - #1930: 失敗の記録は `FailureKey`（処理の種類と対象）に対して `observed` と `resolved` を出す形で扱う。
- 調査基準は `main` の `9524a6fa`。正本が「確認済みの事実」として挙げる `636bb2f0` から `9524a6fa` まで、`src-tauri/src/` に差分は無く、正本の記載はこの commit でも成立する（Current Behavior に記載）。
- Main は `src-tauri/src/adaptor/controller/daemon.rs` の `compose` である。`src-tauri/src/lib.rs:69` から呼ばれ、`common::retry::RetryLimiter` の実体もここで 1 つ作る（`daemon.rs:29`）。

# Outcome

対象者は、daemon を実装・保守する開発者である。Releash の UI を使う利用者は、この変更の前後で同じ振る舞いを見る。

現在、同時実行の枠に入れなかった呼び出しを失敗の記録へ出す（`observed`）が、枠に空きができても同じキーを解かない（`resolved` を出さない）。成功したときに同じキーで `resolved` を出す、という失敗の記録の約束が、この 1 か所だけ守られていない。加えて、枠を掛ける包みの本体、優先度の分類、枠の大きさ、包みの組み立てが controller にあり、「包みの定義は common に置く」「配線は Main の責務」という規約と食い違う。そのため #1897 で確かめる「枠は受け手の側の包みとして掛かっている」の確認の対象がずれる。

変更後は、拒否の記録が、枠に空きができて呼び出しが受理された時点で解かれる。枠を掛ける包みの定義と、優先度の分類と枠の大きさは common にあり、包みの組み立ては Main が行う。controller は組み立て済みの包みを入口の手前に掛けるだけになる。利用者から見た振る舞い（枠の対象、拒否時の結果、待ち方、画面が受け取る失敗の記録と一覧の見え方）は変わらない。

# Current Behavior

`9524a6fa` のコードで確認した挙動である。`636bb2f0..9524a6fa` の差分は `proto/` と `src/` だけで、`src-tauri/src/` には無い。

## 拒否の記録が解かれない

- 枠に入れなかった呼び出しを、`FailureKey::new("client_request_limit", "daemon")` と `Failure::Technical(TechnicalFailureNature::Transient)` で失敗の記録へ出す（`src-tauri/src/adaptor/controller/api/client_priority.rs:52-69`）。メッセージは「呼び出しの path」と「拒否の理由」を連ねたものである。
- 同じキーで `resolved` を呼ぶ箇所は無い。`resolved` の非テストの呼び出しは `src-tauri/src/usecase/retry.rs:40,71`（やり直しが成功したとき）と `src-tauri/src/adaptor/gateway/desktop_client.rs:221-223`（生存が戻ったとき）の 3 箇所で、いずれも `observed` と同じキーを解く。`desktop_client.rs` は、直前まで失敗していたときだけ解く。`DaemonLiveness::succeeded` が連続失敗数を 0 に戻しつつ、それが 0 より大きかったかを返す（`src-tauri/src/domain/daemon_supervision.rs:14-16`）。
- `FailureOutput::resolved` は、そのキーの記録の `active` を false にする（`src-tauri/src/adaptor/presenter/failure.rs:51-54`、`src-tauri/src/adaptor/gateway/failure_records.rs:70-78`）。
- `FailurePresenter` は `observed` と `resolved` のどちらでも、毎回 `StateChangeSource::Failures(target)` を購読へ出す（`src-tauri/src/adaptor/presenter/failure.rs:36`）。要対応の有無が変わったときだけ出すのは workspace の一覧への変更であり、処理の種類が `workflow_` で始まる場合に限る（同 `:33-35`）。したがって `resolved` を呼ぶたびに、失敗の記録の走査と、その target を購読している画面の読み直しが起きる。
- 画面の見え方は変わらない。要対応の分類は `Business(Other)`・`Technical(TimedOut)`・`Technical(Other)` の 3 つで、`Transient` は含まない（`src-tauri/src/usecase/failure.rs:140-146`）。画面へ送る失敗の記録が持つのは `requires_attention`（`active` かつ要対応の分類）だけで、`active` そのものは送らない（`src-tauri/src/adaptor/presenter/state_subscription_wire.rs:10-35`、`proto/client.proto:2918-2932`）。既存テストも `requires_attention` が false であることを期待している（`src-tauri/src/adaptor/controller/api/client_test.rs:1451`）。
- 画面へ返す一覧は `active` で絞らないため、解けた記録も一覧に残る（`src-tauri/src/adaptor/gateway/failure_records.rs:126-150`）。画面は記録が 1 件でもあれば「処理の失敗」を出す（`src/components/workflow/BackgroundFailures.tsx:13-22`）。target が `"*"` の一覧に daemon の記録も入る（`src/components/workspace/WorkspaceList.tsx:1676`）。
- 同じ失敗の前の記録を探す条件も `active` かつ要対応であるため、`Transient` の記録の `active` は、記録をまとめる処理にも効かない（`src-tauri/src/adaptor/gateway/failure_records.rs:31-37`）。

## 包みの本体、優先度の分類、枠の大きさ、組み立てが controller にある

- 枠を掛ける包み `PriorityInterceptor` は `src-tauri/src/adaptor/controller/api/client_priority.rs:36-73` にある。`connectrpc::Interceptor` の `intercept_unary` だけを実装し、枠を取り、取れなければ失敗の記録へ出して `ConnectError` を返す。
- 包みは releash の 3 つの層に依存している。`usecase::failure`（`FailureOutput`・`FailureKey`・`WorkFailure`・`Failure`）、`domain::failure::TechnicalFailureNature`、`adaptor::presenter::connect::request_rejected`（`src-tauri/src/adaptor/presenter/connect.rs:643-654`）である。
- 優先度の分類（`priority_level`）は同ファイル `:15-34`、枠の大きさと待ち行列の長さ（`TOTAL_SEATS`・`SHARES`・`QUEUE_LENGTH`・`limits()`）は同ファイル `:6-13` にある。
- 枠そのもの（段ごとの席と待ち行列、受理と拒否の判定、待ち時間の決め方）は common にある（`src-tauri/src/common/concurrency.rs`）。`common/` の非テストコードに releash の層への参照は無い。
- 包みの組み立ては controller の中にある。枠の実体は `ClientApiDeps::new` が作り（`src-tauri/src/adaptor/controller/api/client.rs:38`）、包みは `router` が組み立てて入口へ掛ける（同 `:149-156`）。Main は `ClientApiDeps` に失敗の記録の出し先を渡すだけである（`src-tauri/src/adaptor/controller/daemon.rs:532-545`）。
- 枠の大きさと待ち行列の長さの値は、テストの中にも同じ値の写しがある（`src-tauri/src/common/concurrency_test.rs:3-9`）。
- `connectrpc::Interceptor` の `intercept_unary` の戻りは `Result<UnaryResponse, ConnectError>` に固定されている。Interceptor を実装する側は転送の型を名乗る。

# Scope / Non-goals

今回変更する対象。

- 拒否の記録の扱い（`src-tauri/src/adaptor/controller/api/client_priority.rs`）。枠に空きができて呼び出しが受理されたときに、同じキーを解くこと。
- 枠を掛ける包みの本体の置き場所。`adaptor/controller/api/client_priority.rs` から `src-tauri/src/common/` へ移すこと。移すために必要な、拒否と受理を伝える口の定義と、その口を通した失敗の記録への接続を含む。
- 優先度の分類と、枠の大きさ・待ち行列の長さの置き場所。`adaptor/controller/api/client_priority.rs` から Main へ移し、包みの設定として Main が包みへ渡すこと。common は枠を掛ける仕組みだけを持ち、ClientService の method の名前を持たないこと。
- 包みの組み立ての場所。`adaptor/controller/api/` から Main（`adaptor/controller/daemon.rs` の `compose`）へ移すこと。
- 上記に伴い使われなくなるコードの削除。

今回変更しない対象。

- 優先度の分け方、それぞれの枠の大きさ、待ち行列の長さ。#1894 で決まっている。
- 枠の対象外の呼び出し（接続の確立、送る量の制御のための呼び出し）。#1894 で決まっている。
- 購読の stream に枠を掛けないこと。#1878 で決まっている。
- 拒否時に返すステータスコードとエラーコード、拒否の理由の文字列、拒否の分類（`Transient`）。
- 失敗の記録の仕組み。画面へ返す一覧が `active` で絞らないこと、要対応の分類、記録をまとめる条件、画面の失敗の一覧の見え方は今のままにする。
- 期限と取り消しを受け取る仕組みと、既定の期限の値。#1883 が扱った。
- 画面（`src/`）のコード。この milestone の「層の整理」の対象はサーバのコードだけである。
- 規約（`docs/architecture/`）の記述。

# Requirements

- R-001: 拒否を失敗の記録へ出したあと、次に呼び出しが枠に受理されたときに 1 回だけ同じキーの記録を解く。拒否が残っていないときは解かない。
- R-002: 同時実行の枠を掛ける包みの定義は `src-tauri/src/common/` にあり、common は他のどの層も参照しない。
- R-003: 優先度の分類と、枠の大きさ・待ち行列の長さは、包みの設定として Main が包みへ渡す。common は枠を掛ける仕組みだけを持ち、ClientService の method の名前を持たない。
- R-004: 包みの組み立ては Main が行う。controller は組み立て済みの包みを入口の手前に掛けるだけであり、枠の実体を作らない。
- R-005: 枠の対象、優先度の分け方、枠の大きさ、待ち行列の長さ、枠が埋まったときの待ち方、拒否時の結果は変わらない。
- R-006: 画面が受け取る失敗の記録の内容と、画面の失敗の一覧の見え方は変わらない。
- R-007: この変更で使われなくなるコードは残らない。

# Assumptions

人間が明示的に受け入れた仮定は無い。
