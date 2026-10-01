# Design

## 変える部分

- タスクの一覧の購読の対象: Repository・タイトルの絞り込み・ラベルの絞り込み・件数で決まる購読の対象を足し、絞り込みが無いときはその引数を持たず、ラベルの絞り込みは並べ替えて持つ。根拠: R-001「購読は Repository、タイトルの絞り込み、ラベルの絞り込み、件数で決まる」、R-003「その絞り込みを購読に渡さないことで表す」、R-004「ラベルの絞り込みの並び順にかかわらず、同じ購読の対象になる」。ルート: 引数の形は、件数を引数に含める Session の履歴の購読（`src-tauri/src/adaptor/presenter/state_subscription_target.rs:54-60`）に合わせる。目印の文字列で「絞り込み無し」を表さない。
- タスクの一覧の取得: daemon が Notion API の cursor をたどり、絞り込みに合うタスクを指定の件数まで取り、その先にまだタスクがあるかを値に含める。根拠: R-001「先頭からその件数まで届ける。届く値は、その件数の先にまだタスクがあるかを含む」、R-002「件数を 20 件増やした一覧」。ルート: 絞り込みは今の Notion API の filter（`src-tauri/src/adaptor/gateway/notion/service_impl.rs:134-147`）で行う。件数の刻みは今の 1 ページ 20 件（`:135`）。「まだタスクがあるか」は Session の履歴の `hasMore`（`src/components/workspace/WorkspaceList.tsx:840`）と同じ形にする。
- ラベルの選択肢の購読の対象: Repository で決まる購読の対象を足す。根拠: R-005「購読は Repository で決まる」。ルート: 委任
- 取り直しの契機と失敗の届け方: タスクの一覧とラベルの選択肢の購読を、Issue の一覧の購読と同じ外部の情報の取り直しの契機に載せ、Notion の設定の変化でも取り直す。最後に取れた値と直近の失敗を一緒に届ける。設定が揃っていないことによる失敗は、Notion からの取得の失敗と区別して届ける。根拠: R-006「Issue の一覧の購読と同じ契機で…加えて、その Repository の Notion の設定が保存・削除されたときも」、R-007「前に取れた値があれば、その値と失敗の両方が届く。Notion の設定が揃っていないことによる失敗は、Notion からの取得の失敗と区別して届く」。ルート: Issue の一覧の作り（`src-tauri/src/usecase/git_host/git_host_usecase.rs:69-97`、`src-tauri/src/usecase/state_subscription.rs:239-282`、`proto/client.proto:2851-2854`）と同じ形にする。
- 画面の hook: `src/hooks/useNotionTasks.ts` と `src/hooks/useNotionLabelOptions.ts` を購読で受け取る形に変え、取得の失敗を画面へ渡す。「Load more」で次の一覧が届くまで前の一覧を表示し続ける。使われていない `refresh` と `initialFilters` を消す。根拠: R-001、R-002「次の一覧が届くまで、画面は前の一覧を表示し続ける」、R-005、R-007。ルート: 委任
- 単発の呼び出しの削除: `QueryNotionTasks` と `FetchNotionLabelOptions` を proto と daemon の入口から消し、関係するテストと fixture を直す。根拠: R-008「単発の呼び出しで取る手段が無い」。ルート: 委任
- 設定が揃っているかの規則: 「token と database ID が前後の空白を除いて空でない」を domain の `NotionRepoConfig`（`src-tauri/src/domain/app_config/value_objects/mod.rs:30`）に置き、設定の検証（`src-tauri/src/usecase/notion/usecase.rs:162-176`）とタスク・ラベルの取得（`:180-189`）の両方がそれを使う。根拠: R-009「設定の検証とタスク・ラベルの取得は、この同じ規則で判定する」。ルート: 規則の置き場所は domain の `NotionRepoConfig`。

## 固定するルート

- 購読の引数の形は、Session の履歴の購読と同じく件数を引数に含め、「Load more」は件数を増やして購読し直す。Kubernetes の watch が絞り込み（label selector / field selector）を要求に含めることに合わせる。
- 絞り込みが無いことは、引数を渡さないことで表す。目印の文字列を使わない。目印の文字列は本物の入力と区別できないため。
- ラベルの絞り込みは並べ替えて購読の対象に持つ。並び順だけが違う購読が別の対象になり、同じものを二重に取りに行かないため。
- 件数は daemon が Notion API の cursor をたどって満たす。件数の刻みは 20 件。「まだタスクがあるか」は Session の履歴の `hasMore` と同じ形で値に含める。
- 取り直しの契機と、値と失敗の届け方は、Issue の一覧の作りに合わせる。
- 「設定が揃っているか」の規則は domain の `NotionRepoConfig` が持つ。状態を持たない業務の規則であるため。

## 変えないもの

- 画面の設定欄で、入力が空なら検証のボタンを押せない表示の制御（`src/components/panels/NotionSettingsSection.tsx:172`）。表示の制御であり、規則の判定ではないため。

## 未確定・リスク

なし
