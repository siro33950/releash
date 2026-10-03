# Context

- 入力文書: https://github.com/siro33950/releash/issues/1980
- 補助資料: https://github.com/siro33950/releash/issues/1977、https://github.com/siro33950/releash/issues/1978、`proto/client.proto`、`src/lib/client.ts`、`src-tauri/src/infrastructure/state_subscription.rs`、`docs/architecture/CONTROLLER.md`、`docs/architecture/PRESENTER.md`、`docs/architecture/USECASE.md`、`docs/architecture/INFRASTRUCTURE.md`
- 購読の多重化は、graphql-ws の protocol の形に合わせる。graphql-ws では、client が付けた id と購読の内容を Subscribe で送り、その後は Next・Error・Complete のどれも id だけで購読を指す。Releash の開始・停止・処理済みの量の報告は、値が流れる stream とは別の unary の要求である（`proto/client.proto:2676-2678`）。
- Releash は、購読の識別子を terminal の入力の宛先にも使う。入力の受理は、terminal ごとに有効な宛先を 1 つ持ち、宛先の値の一致だけで照合する（`src-tauri/src/domain/terminal_surface/entities/terminal_surface_input_ingress.rs:40-78`）。そのため、識別子が重ならない範囲を、graphql-ws の「接続の中」から「daemon 全体」に広げる。
- daemon は 127.0.0.1 だけに bind し、client token を持つ同じ利用者の画面だけを相手にする（`AGENTS.md`）。
- ISSUE の本文の位置は main `506f9a98` のものである。その後 #1977（`35b83a80`）と #1978 が入った。以下の位置は main `5b1c71fb` で確認したもの。

# Outcome

- 対象者: daemon と画面の開発者。
- 現在の問題: 画面が購読を見分ける識別子と、daemon が複数の購読で共有する対象の識別子が、同じ「対象の名前＋引数」を兼ねている。daemon が引数を正規化して同じ対象にまとめると、画面は届いた値を自分の購読に結び付けられない。これを補うため、#1977 で、画面が送った引数と正規化した対象の対応表を controller と presenter に持たせた。これは規約（CONTROLLER.md・PRESENTER.md・USECASE.md）に反する。
- 変更後の状態: 画面は購読ごとに識別子を付けて開始し、daemon はその識別子を付けて値を返す。開始の後は、停止・届く値・terminal の処理済みの量の報告のどれも、識別子だけで購読を指す。共有する対象は daemon だけが正規化して持ち、同じ対象を複数の購読が指しても取得は 1 つにまとまる。画面も daemon も、画面が送った引数と正規化した対象の対応を持たない。

# Current Behavior

調査は main `5b1c71fb` のコードを読んで行った。

1. 購読の指し方（proto）
   - 開始の要求は `client_id`・`target`・`version`・`args`・`terminal_input_id` を持つ（`proto/client.proto:2855-2861`）。購読ごとの識別子は無い。
   - 停止の要求は `client_id`・`target`・`args` で購読を指す（`proto/client.proto:2862`）。
   - 届く値は `target`・`args`・`version` と、`ready`・`snapshot`・`change`・`bookmark`・`failure` のどれかを持つ（`proto/client.proto:2912-2923`）。stream 全体の合図 `ready`・`bookmark` は `target`・`args` を空で送る（`src-tauri/src/adaptor/presenter/state_subscription_wire.rs:232-243`）。
   - terminal の処理済みの量の報告は `client_id`・`args`・`units` で購読を指す（`proto/client.proto:2925-2929`）。
2. 画面（`src/lib/client.ts`）
   - `(kind, args)` を JSON にした文字列ごとに entry を 1 つ持ち、同じ文字列の受け手は 1 つの購読を共有する（:229-231, :407-461）。
   - 届いた値は `target` と `args` から作った文字列で entry を引き当てる（:352）。
   - 同じ文字列の開始と停止を、送った順に直列にする（:271-287）。理由は「開始済みの対象への開始は無視されるため、後から届いた停止で購読が消える」。
   - terminal の開始のたびに UUID を作って `terminal_input_id` に入れ、受理されたら入力の宛先として使う（:290-311, :493-498、`src/hooks/useTerminal.ts:570-580,712-716`）。
   - 処理済みの量の報告は、owner から terminal の args を作り直して送る（:500-527）。
3. daemon
   - 配信の土台は、client ごとに「対象の文字列 → 送り待ち」を持つ（`src-tauri/src/infrastructure/state_subscription.rs:276-287`）。値は対象の文字列を付けて流す（:8-12, :506-536）。開始済みの対象への開始は何もせず成功する（:376-378）。汎用と terminal の presenter は同じ配信の土台を共有する（`src-tauri/src/adaptor/presenter/terminal_subscription.rs:25-29`）。
   - 汎用の購読の usecase は、client ごとに対象の集合を持つ（`src-tauri/src/usecase/state_subscription.rs:96-98`）。同じ client の 2 つ目の開始は集合に既にあるので何も増えず、1 つ目の停止で消える（:475-505）。
   - terminal の購読の usecase は、client ごとに「対象 → 入力の識別子」を持つ（`src-tauri/src/usecase/terminal_surface/subscription.rs:37`）。入力の識別子が省略されたら `client_id` を使う（:141）。停止は (terminal, client) の単位で出力の購読を外す（:115-131）。
   - controller は対象の種類で、汎用か terminal の usecase に開始と停止を振り分ける（`src-tauri/src/adaptor/controller/api/client.rs:240-271`）。
   - 停止は、stream が閉じた後だと `StreamEnded`（Connect の `NOT_FOUND`）を返す（`src-tauri/src/infrastructure/state_subscription.rs:370-379`、`src-tauri/src/usecase/state_subscription.rs:494-499`、`src-tauri/src/adaptor/presenter/connect.rs:282-292`）。
4. #1977 の対応表
   - presenter が、client ごと・対象ごとに画面が送った args の集合を持つ（`src-tauri/src/adaptor/presenter/state_subscription.rs:50-165` の `request_lock`・`requested_args`・`add_request`・`has_other_requests`・`remove_request`・`replay_request`）。
   - 届く値を、対象の args を画面が送った args に差し替えて、別名ごとに複製して送る（`src-tauri/src/adaptor/presenter/state_subscription.rs:150-181`、`src-tauri/src/adaptor/presenter/state_subscription_wire.rs:227-280`）。
   - controller が、Notion のタスクの一覧の対象だけ、この対応表への登録・解除を分岐で行う（`src-tauri/src/adaptor/controller/api/client_service.rs:34-121`、`src-tauri/src/adaptor/controller/api/client.rs:150-165`）。
   - 再現: 同じ client が、同じ Repository・件数で、ラベルの絞り込みの並び順だけが違う 2 つの `notion-tasks` を開始する。daemon は 1 つの対象として取得し、届く値を画面が送った 2 つの args に差し替えて 2 回送る。片方を止めても、もう片方の args が残っている間は daemon の購読を外さない。

# Scope / Non-goals

## Scope

- 購読の開始・停止・届く値・terminal の処理済みの量の報告での、購読の指し方（`proto/client.proto`）。
- 画面の購読の持ち方と、届いた値の引き当て、開始と停止を送る順序、terminal の入力の宛先と処理済みの量の報告の送り方（`src/lib/client.ts`、`src/hooks/useTerminal.ts`）。
- daemon の、client ごとの購読の持ち方と、値に付ける識別子（配信の土台、presenter、controller、汎用と terminal の購読の usecase）。
- #1977 で入った、画面が送った引数と正規化した対象の対応表と、それを使う controller の分岐、それを確かめるテストの削除。

## Non-goals

- 対象ごとの値の中身と、取り直し・差分・版（`version`）の規則。
- terminal の購読の振る舞い（#1978）。terminal の入力の受理・棄却の規則と、出力の流量制御の規則（#1888）を含む。
- controller が対象の種類で汎用か terminal の usecase に振り分ける形（#1978 で決めたもの）。
- 画面が同じ `(kind, args)` の受け手で 1 つの購読を共有する仕組み。
- 画面で引数を正規化すること（並び順をそろえる等）。

# Requirements

- R-001: 画面は、購読を開始するたびに新しい識別子を付け、開始の要求に識別子と対象の名前・引数を載せる。再接続で開始し直すときと、terminal を開始し直すときも新しい識別子を付ける。
- R-002: daemon は、購読ごとに届く値（snapshot・change・failure・購読ごとの bookmark）に、その購読の識別子を付けて送る。対象の名前と引数は載せない。stream 全体の合図（ready・stream の bookmark）は識別子を持たない。
- R-003: 画面は、届いた値を識別子で自分の購読に引き当てる。
- R-004: 停止の要求は、識別子だけで購読を指す。daemon が知らない識別子（停止済み、stream が閉じた後、一度も開始していない）の停止は、何もせず成功する。
- R-005: 識別子は、空でなく 128 バイト以下である。満たさない開始は、stream の `client_id` の形の誤りと同じく `INVALID_ARGUMENT` で拒まれる。
- R-006: daemon 全体（全 client）の有効な購読と重なる識別子での開始は、`ALREADY_EXISTS` で拒まれる。このとき stream は閉じず、既存の購読は続く。停止・切断・stream の終了で無効になった識別子は、再び使える。同時に来た 2 つの開始が同じ識別子を持つとき、両方が受け付けられることはない。
- R-007: 同じ正規化した対象を複数の識別子が指すとき（同じ client の中の場合を含む）、daemon の取得は 1 つにまとまり、値は識別子ごとに届く。片方の識別子を止めても、他方には値が届き続ける。ラベルの絞り込みの並び順だけが違う Notion のタスクの一覧の購読（#1977 R-004）はこれに当たる。
- R-008: 画面は、停止をその購読の開始の要求が終わってから送る。開始の要求が終わる前に受け手がいなくなっても、daemon に購読が残らない。
- R-009: terminal の入力の宛先は、その terminal の購読の識別子である。開始の要求は、別に入力の識別子を持たない。入力の受理・棄却の規則は変えない。
- R-010: terminal の処理済みの量の報告は、識別子と量だけで購読を指す。識別子が有効な terminal の購読を指さないときは、今の「terminal の購読が無い」ときと同じ失敗になる。量を足す単位（terminal と client の組）と流量制御の規則は変えない。

# Assumptions

- 画面は識別子に UUID を使う。再接続では新しい stream で新しい UUID を付けて開始し直すため、古い stream の後始末より先に新しい開始が届いても識別子は重ならない。
