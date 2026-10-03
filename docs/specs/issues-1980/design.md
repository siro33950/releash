# Design

## 変える部分
- 開始の要求: `StartStateSubscriptionRequest` に購読の識別子を足し、`terminal_input_id` を消して番号を reserved にする。根拠: R-001「開始の要求に識別子と対象の名前・引数を載せる」、R-009「開始の要求は、別に入力の識別子を持たない」。ルート: `client_id`・`target`・`args`・`version` は残す（`proto/client.proto:2855-2861`）。
- 停止の要求: `StopStateSubscriptionRequest` を識別子だけにし、`client_id`・`target`・`args` を消して番号を reserved にする。根拠: R-004「停止の要求は、識別子だけで購読を指す」。ルート: 委任
- 届く値: `StateSubscriptionEvent` に識別子を足し、`target`・`args` を消して番号を reserved にする。ready・stream の bookmark では識別子を空にする。根拠: R-002「その購読の識別子を付けて送る。対象の名前と引数は載せない。stream 全体の合図…は識別子を持たない」。ルート: 委任
- 処理済みの量の報告: `ReportTerminalProcessedRequest` を識別子と `units` だけにし、`client_id`・`args` を消して番号を reserved にする。根拠: R-010「識別子と量だけで購読を指す」。ルート: 委任
- 配信の土台の識別子の登録簿: 配信の土台が、client ごとの購読を「対象の文字列 → 送り待ち」から「識別子 → (対象の文字列, 送り待ち)」に持ち替え、識別子から client と対象を引けるようにする。開始での、全 client の有効な識別子との重なりの検査と登録を、配信の土台の 1 つの lock の中で行い、重なれば `ALREADY_EXISTS` にする。値は識別子ごとに送り待ちに積み、識別子を付けて流す。根拠: R-002、R-006「daemon 全体（全 client）の有効な購読と重なる識別子での開始は、`ALREADY_EXISTS` で拒まれる…同時に来た 2 つの開始が同じ識別子を持つとき、両方が受け付けられることはない」、R-007「値は識別子ごとに届く」。ルート: 置き場所は `src-tauri/src/infrastructure/state_subscription.rs` の `Subscriptions`。登録簿は識別子・client・対象の文字列だけを扱い、内側の層の型を使わない。
- 識別子の形の検査: 空でなく 128 バイト以下であることを、開始の要求で検査し、満たさなければ `SubscriptionError::InvalidId`（Connect の `INVALID_ARGUMENT`、`src-tauri/src/adaptor/presenter/connect.rs:285`）で拒む。根拠: R-005「満たさない開始は、stream の `client_id` の形の誤りと同じく `INVALID_ARGUMENT` で拒まれる」。ルート: stream の `client_id` の検査（`src-tauri/src/adaptor/controller/api/client_service.rs:22-26`）と同じ扱いにし、上限は同じ `SUBSCRIPTION_ID_MAX_BYTES`（`src-tauri/src/adaptor/controller/api/client_stream.rs:1-5`）を使う。形の検査はこの 1 か所だけにし、terminal の購読の usecase の入力の識別子の検査（空白だけを拒む規則を含む）と、そのための失敗 `StateReadFailure::InvalidTerminalInput` を消す。そのため、空白だけの識別子での開始は受け付けられるようになる（R-005 は空でなく 128 バイト以下だけを定め、stream の `client_id` の検査も空白だけを拒まない）。画面は UUID を付けるので、空白だけの識別子は届かない（requirements.md Assumptions）。
- controller の振り分け: 開始は今と同じく対象の種類で汎用か terminal の usecase に振り分ける。停止は、presenter を通して配信の土台に識別子から client と対象を引かせ、引いた対象の種類で今と同じく振り分ける。知らない識別子の停止は何もせず成功にする。処理済みの量の報告は terminal の購読だけのものなので、controller は識別子と量をそのまま terminal の usecase に渡す。terminal の usecase が自分の持つ入力の識別子（＝購読の識別子）から client と対象を引き、識別子が terminal の購読を指さないときは今の `TerminalSubscriptionEnded` を返す。この失敗は terminal の usecase の 1 か所でだけ作り、controller は作らない。根拠: R-004「daemon が知らない識別子…の停止は、何もせず成功する」、R-010「今の『terminal の購読が無い』ときと同じ失敗になる」。ルート: 振り分けは controller に残す（`src-tauri/src/adaptor/controller/api/client.rs:240-271`）。usecase に振り分けを移さない。
- 汎用の購読の usecase: client ごとの対象の集合を、(client, 対象) ごとの購読の数に置き換える。開始で 1 増やし、停止で 1 減らし、0 になった項目は消して、そのときだけ今の外す処理（worker・watch・外部の情報の保持の解放）をする。切断は数に関係なくその client の全部を外す。根拠: R-007「片方の識別子を止めても、他方には値が届き続ける」。ルート: usecase は識別子を持たない（`src-tauri/src/usecase/state_subscription.rs:96-98` の `clients` を `HashMap<対象, usize>` の形にする）。
- terminal の購読の usecase: client ごとの「対象 → 入力の識別子」を、(client, 対象) ごとの入力の識別子（＝購読の識別子）の集まりに置き換える。最後の 1 つが止まったときだけ出力の購読を外す。入力の宛先の有効・無効は今の規則のまま（後の開始が置き換え、止めた識別子が今の宛先と一致するときだけ無効）。入力の識別子が省略されたら `client_id` を使う規則を消す。根拠: R-007、R-009「terminal の入力の宛先は、その terminal の購読の識別子である…入力の受理・棄却の規則は変えない」。ルート: `src-tauri/src/usecase/terminal_surface/subscription.rs:37` の `clients` を持ち替える。
- #1977 の対応表の削除: presenter の `request_lock`・`requested_args`・`add_request`・`has_other_requests`・`remove_request`・`replay_request` と、届く値の別名ごとの複製（`wire_events`・`event_with_args`）、controller の Notion のタスクの一覧だけの分岐と `RequestStartPermit`、これらを確かめるテスト（`src-tauri/src/adaptor/controller/api/client_test.rs` の `test_notion購読_入力順をdaemonで共有し要求元へ値と失敗を届ける`、`src-tauri/src/adaptor/presenter/state_subscription_test.rs` の `test_購読入力の対応_解除は最後の入力まで共有しstream終了で破棄する`）を消す。根拠: R-002「対象の名前と引数は載せない」、R-007「値は識別子ごとに届く」。ルート: 委任
- 画面の購読: entry ごとに識別子を持ち、開始のたびに `crypto.randomUUID()` で新しく付ける。届いた値は識別子で entry を引き当てる。同じ `(kind, args)` の受け手で 1 つの購読を共有する仕組みは残す。根拠: R-001「開始するたびに新しい識別子を付け…再接続…terminal を開始し直すときも」、R-003「届いた値を識別子で自分の購読に引き当てる」。ルート: `src/lib/client.ts` で行う。並び順だけが違う `(kind, args)` は別の entry・別の識別子のままにし、画面で正規化しない。
- 画面の開始と停止の順序: `(kind, args)` ごとの直列を、識別子ごとに「停止はその開始の要求が終わってから送る」形に変える。残す理由は、daemon が開始前の識別子の停止を無視する（graphql-ws と同じ）ため。根拠: R-008「停止をその購読の開始の要求が終わってから送る」。ルート: `src/lib/client.ts:271-287` を持ち替える。
- 画面の terminal の入力の宛先と報告: `currentTerminalInputId` は terminal の entry の購読の識別子を返す。処理済みの量の報告は識別子と量を送り、terminal の args の作り直しをやめる。根拠: R-009、R-010。ルート: 委任

## 固定するルート
- 識別子の登録簿と全 client での重なりの検査は、配信の土台（`src-tauri/src/infrastructure/state_subscription.rs` の `Subscriptions`）の 1 か所に置く。理由: 購読の識別子は 1 本の stream の中で購読を見分ける転送の識別子で、送る仕組みの一部である（PRESENTER.md「送る仕組みそのもの…は infrastructure と common の包みを使う」）。配信の土台は既に汎用と terminal の両方の購読を持っている（`src-tauri/src/adaptor/presenter/terminal_subscription.rs:25-29`）。
- 汎用と terminal の振り分けは controller に残す（#1978 の形）。新しい Usecase を足さず、汎用の usecase に terminal を知らせない。
- 汎用の購読の usecase は識別子を持たず、(client, 対象) ごとの数だけを持つ。terminal の購読の usecase は入力の宛先として識別子を持つ。
- 汎用と terminal の usecase のそれぞれに、同じ client の 2 重の購読で、片方を止めても他方が外れないことを確かめるテストを 1 本ずつ足す。

## 変えないもの
- 対象の版（`version`）は対象ごとに 1 つのまま。再接続の後の再開は、今と同じく対象の版で行う。
- 停止の結果が `StreamEnded`（`NOT_FOUND`）から成功に変わっても、画面の振る舞いは変わらない。画面は停止の失敗のうち再接続の対象（`UNAVAILABLE`・`ABORTED`・`RESOURCE_EXHAUSTED`、`proto/client.proto:2670-2672`）だけで stream を開き直し、それ以外は debug のログだけを出す（`src/lib/client.ts:449-456`）。`NOT_FOUND` は再接続の対象ではない。
- terminal の購読の振る舞い（#1978）。入力の宛先の値の出どころが購読の識別子に変わるだけで、入力の受理・棄却と出力の流量制御（#1888）の規則は同じ。
- 処理済みの量の報告で、識別子が terminal 以外の購読を指すときも、新しい失敗を作らず今の `TerminalSubscriptionEnded` にする。今、terminal の購読が無いときの失敗と合わせるため。

## 未確定・リスク
なし
