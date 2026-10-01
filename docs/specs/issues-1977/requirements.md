# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1977
- 補助資料: マイルストーン #99「02. UI と daemon の間の通信の仕組みを一本化する」、#1956（購読の読み取りの失敗を画面へ伝える）、#1897（通信の共通化の確認）、マイルストーン #97（アーキテクチャを見直す）
- この要求は main `506f9a98` のコードで確かめた事実に基づく。
- マイルストーン #99 の通信の規則: daemon が持っている状態は daemon が配信し、client は購読する。単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけにする。確立した標準（Kubernetes の list + watch）に合わせる。
- 同じ外部の状態である Issue の一覧は、購読 `issues:[repoPath]` で届く。daemon は購読の開始時と 30 秒ごと（`src-tauri/src/domain/git_host/value_objects/cache.rs:7`）に外部から取り直し、最後に取れた値と直近の失敗を一緒に届ける（`src-tauri/src/usecase/state_subscription.rs:207-282`、`proto/client.proto:2851-2854`）。
- 件数を購読の引数に含め、もっと見るときは件数を増やして購読し直す前例が、Session の履歴にある（`src-tauri/src/adaptor/presenter/state_subscription_target.rs:54-60`、`src/components/workspace/WorkspaceList.tsx:830-843`）。
- 購読の引数は空文字を受け付けない（`src-tauri/src/adaptor/presenter/state_subscription_target.rs:26-31`）。
- 処理の重複の扱いは `docs/architecture/README.md:59`（同じ操作の実装は 1 つに集約する）に従う。

# Outcome

- 対象者: worktree を作る画面の Notion のタブで、Notion のタスクを選んで worktree を作る開発者。
- 今の問題: タスクの一覧とラベルの選択肢は、画面が開いたときと絞り込みを変えたときに単発で取るだけである。Notion 側でタスクが増減・変更されても、Notion の設定を保存・削除しても、画面は古い表示のままになる。同じ外部の状態を、Issue と Notion で別の届け方で扱っている。「Notion の設定が揃っているか」の判定が、設定の検証とタスク・ラベルの取得で食い違っている。
- 変更後の状態: タスクの一覧とラベルの選択肢が、Issue の一覧と同じく daemon からの購読で届き、Notion 側の変化と設定の変化が画面に反映される。取得の失敗も画面に届く。「Notion の設定が揃っているか」は 1 つの規則で判定される。

# Current Behavior

main `506f9a98` のコードを読んで確かめた挙動である。実行はしていない。

## タスクの一覧

- 画面は `QueryNotionTasks`（`proto/client.proto:2734`）を単発で呼び、結果を自分の state に入れて表示する（`src/hooks/useNotionTasks.ts:28-64`）。呼ぶのは、マウントしたとき（`:66-69`）、タイトル・ラベルの絞り込みを変えて 300 ms たったとき（`:79-90`）、「Load more」を押したとき（`:92-96`）だけである。
- 入力は Repository のパス、タイトルの絞り込み、ラベルの絞り込み（プロパティ名ごとの値の集合）、cursor である（`src/hooks/useNotionTasks.ts:37-44`）。daemon は保存された Notion の設定で Notion API の database query を引き（`src-tauri/src/adaptor/controller/client/notion/commands.rs:17-32`、`src-tauri/src/usecase/notion/usecase.rs:80-92`）、絞り込みを Notion API の filter に変換する（`src-tauri/src/adaptor/gateway/notion/service_impl.rs:134-147`）。1 回で 20 件取る（`:135`）。
- 「Load more」は返された cursor で次の 20 件を取り、画面が前の一覧の後ろにつなげる（`src/hooks/useNotionTasks.ts:45-51`）。取っている間も前の一覧は消えない。「Load more」を出すかは、返された `has_more` で決める（`:50`、`src/components/workspace/CreateWorktreeModal.tsx:968`）。
- 取得に失敗すると、画面は失敗の通知を出し、一覧を空にする（`src/hooks/useNotionTasks.ts:52-58`）。
- 画面（`src/components/workspace/CreateWorktreeModal.tsx:784-990`）は、ブランチ名が空のタスクと、既に worktree があるブランチのタスクを表示から外す（`:798-804`）。
- hook の `refresh` と `initialFilters` は、どこからも使われていない（`src/components/workspace/CreateWorktreeModal.tsx:796` は `tasks`・`loading`・`loadMore`・`hasMore`・`search` だけを使い、引数を渡さない）。

## ラベルの選択肢

- 画面は `FetchNotionLabelOptions`（`proto/client.proto:2719`）を、マウントしたときに 1 回だけ単発で呼ぶ（`src/hooks/useNotionLabelOptions.ts:10-25`）。購読も、取り直す手段も無い。
- 取得に失敗すると、画面は失敗の通知を出し、選択肢を空にする（`src/hooks/useNotionLabelOptions.ts:15-17`）。

## Notion の設定が揃っているかの判定

- タスクとラベルの取得は、token と database ID が前後の空白を除いて空でないときだけ Notion API を呼ぶ。そうでなければ「Notion設定が見つかりません」の失敗になる（`src-tauri/src/usecase/notion/usecase.rs:180-189`）。
- 設定の検証（`ValidateNotionConfig`）は、token と database ID が空文字のときだけ「未設定」を返す（`src-tauri/src/usecase/notion/usecase.rs:162-176`）。空白だけの token（例: `" "`）では Notion API を呼ぶ。
- domain の `NotionRepoConfig`（`src-tauri/src/domain/app_config/value_objects/mod.rs:30`）は、この規則を持たない。
- 画面の設定欄は、token か database ID が空なら検証のボタンを押せない（`src/components/panels/NotionSettingsSection.tsx:172`）。空白だけの入力では押せる。

## 比較: Issue の一覧

- Issue の一覧は購読で届き、daemon が外部から取ってキャッシュし、変化を購読へ知らせる（`src-tauri/src/usecase/git_host/git_host_usecase.rs:69-78`）。Notion はこの形になっていない。

# Scope / Non-goals

## Scope

- Notion のタスクの一覧の届け方を、単発の呼び出しから購読に変える。
- Notion のラベルの選択肢の届け方を、単発の呼び出しから購読に変える。
- タスクの一覧とラベルの選択肢を取る単発の呼び出し（`QueryNotionTasks`、`FetchNotionLabelOptions`）を無くす。
- 「Notion の設定が揃っているか」の判定を 1 つの規則にし、設定の検証とタスク・ラベルの取得の両方がそれを使う。
- 上の変更で使われなくなるコードを消す。

## Non-goals

- Notion の設定の保存の入力を presenter が domain の型に変えていること（マイルストーン #97 で扱う）。
- Notion の 429 の再送のループ（`src-tauri/src/adaptor/gateway/notion/service_impl.rs:72-99`。#1897 で扱う）。
- 画面からタスクの一覧・ラベルの選択肢を手動で取り直す操作。今の画面にも無い。
- 画面の設定欄で、入力が空なら検証のボタンを押せない表示の制御（`src/components/panels/NotionSettingsSection.tsx:172`）。

# Requirements

- R-001: Notion のタスクの一覧は、daemon からの購読で画面に届く。購読は Repository、タイトルの絞り込み、ラベルの絞り込み、件数で決まり、その絞り込みに合うタスクを先頭からその件数まで届ける。届く値は、その件数の先にまだタスクがあるかを含む。
- R-002: 画面の「Load more」は、件数を 20 件増やした一覧を購読で受け取る。次の一覧が届くまで、画面は前の一覧を表示し続ける。
- R-003: タイトルの絞り込みが無いこと、ラベルの絞り込みが無いことは、その絞り込みを購読に渡さないことで表す。
- R-004: 同じ Repository・同じ絞り込み・同じ件数の購読は、ラベルの絞り込みの並び順にかかわらず、同じ購読の対象になる。
- R-005: Notion のラベルの選択肢は、daemon からの購読で画面に届く。購読は Repository で決まる。
- R-006: タスクの一覧とラベルの選択肢の購読は、Issue の一覧の購読と同じ契機で、daemon が Notion から取り直して届ける。加えて、その Repository の Notion の設定が保存・削除されたときも取り直して届ける。
- R-007: タスクの一覧とラベルの選択肢の取得に失敗したとき、失敗が画面に届く。前に取れた値があれば、その値と失敗の両方が届く。Notion の設定が揃っていないことによる失敗は、Notion からの取得の失敗と区別して届く。
- R-008: タスクの一覧とラベルの選択肢を、単発の呼び出しで取る手段が無い。
- R-009: 「Notion の設定が揃っている」は、token と database ID が、前後の空白を除いて空でないことである。設定の検証とタスク・ラベルの取得は、この同じ規則で判定する。揃っていない設定では、どちらも Notion API を呼ばない。

# Assumptions

なし
