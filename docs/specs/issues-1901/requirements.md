# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1901 （[04] CLI と hook が使う入口を Connect に揃え、method ごとの scope を proto に書く）
- 補助資料: マイルストーン #100（https://github.com/siro33950/releash/milestone/100 ）、#1902（[05] CLI を Connect に載せて HTTP を削除する。この ISSUE の入口の使い手）、#1908（[02] `daemon` ドメイン）、#1853（[03] クレート分離）、Google AIP-180（https://google.aip.dev/180 ）、buf breaking（https://buf.build/docs/breaking/overview ）
- 本文の「今の作り」の file:line と数は main `4533af2f` 時点のもの。この文書は main `998a6e59`（#1852・#1908・#1853・#2018 の後）で読み直した事実に基づく。
- マイルストーン #100 の条件: どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。CLI と hook を Connect に移し HTTP `/v1` を削除するのは #1902 で行う。この ISSUE では CLI と hook のプロセス側は HTTP を使い続ける。
- AGENTS.md「通信の原則」: サーバは状態を配信し、client は購読する。単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけ。#1902 の CLI は、読み取りを購読して最初の状態を受け取ったら閉じる。
- 画面の Connect は binary で通信する（`src/lib/client.ts:92`）。proto を使う側（Rust・TS、#78 以降の Swift）は同じリポジトリの proto から生成する。
- 本文と違う形にした点（本文の書き換えは利用者の了承待ちで、本文はまだ編集されていない）
  - scope の判定の場所: 本文は「`build.rs` の method 走査で生成 handler に判定を埋め込む。手書きの RPC は個別に付ける」。この開発では、token を照合する認証の middleware 1 か所で判定する（docs/architecture/CONTROLLER.md「認証は `api/mod.rs` が router 全体へまとめて掛ける」）。
  - hook の scope: 本文は「`hook`（`ReceiveProviderSignal` だけ）」。#1902 の流れ（発見 → `GetServerInfo` → 互換の判定 → 呼び出し）が hook にも当たるため、hook の token は `ReceiveProviderSignal` と `GetServerInfo` を呼べる。
  - review の一覧: 本文は「Session の ID を指定した review の一覧」。`--session-id` なしの一覧（`RELEASH_WORKTREE_PATH` の worktree）が builtin の workflow（`workflows/03_full-review.yml:312`、`04_review-fix-policy.yml:79`、`04_review-fix-policy-manual.yml:79`、`05_review-fix.yml:176`）と利用者の Lua で使われているため、worktree の path を指定した一覧も用意する。

# Outcome

- 対象者: Releash の CLI（agent と workflow の Command Node が呼ぶ `releash workflow …`／`releash review …`）と provider の hook（`releash hook receive`）、およびそれらを #1902 で Connect に移す開発者。
- 現在の問題: CLI と hook が使う入口は HTTP `/v1` にしか無いか、Connect にあっても意味が違う。HTTP 側は起動状態の受付と worktree の認可を通らず、method ごとの権限が無い。hook の payload の解釈と review の書き手の解決が CLI のプロセスの中にある。proto の互換を検査する仕組みが無い。
- 変更後の状態: CLI と hook が使う全ての入口が Connect にあり、読み取りは購読、変更と入力に対する計算は単発の呼び出しで使える。hook の payload の解釈と review の書き手・worktree の解決はサーバが行う。全ての RPC が、受け付ける token の scope を proto に持ち、サーバがそれに従って受け付ける。proto の変更は CI で lint と wire の互換を検査される。

# Current Behavior

main `998a6e59` で読んで確かめた挙動。パスは `src-tauri/src/` 起点（proto は `proto/`）。

- HTTP と Connect の 2 系統を `adaptor/controller/api/mod.rs:26-60` が合成する。HTTP の workflow・provider lifecycle の router は master token（`require_bearer`、`api/auth.rs:12-37`）、Connect は client token と Origin（`require_client`、`api/auth.rs:62-143`）で認証する。同時実行の枠は両方に掛かる（HTTP は `api/mod.rs:84-113` の `local_ingress`、Connect は `api/client.rs:126-140`）。起動状態の受付は Connect だけ（Interceptor `api/client_admission.rs:3-40`）。
- CLI が HTTP で使う入口は 5 つ（`cli/api_client.rs:48-119`）: 実行の状態 `GET /v1/workflow/executions/{id}`、output の提出 `POST /v1/workflow/node-executions/{id}/submit`、output の取得 `GET /v1/workflow/executions/{id}/artifacts/{node}`、診断 `GET /v1/workflow/diagnostics?dir=`、信号 `POST /v1/provider-lifecycle/signals`。`status` と `output get` はサーバに届かないとき同じプロセス内でファイルを直接読む（`cli/file_direct.rs`）。
- Connect にある対応する入口
  - 実行の ID で引く購読の対象は無い（`adaptor/presenter/state_subscription_target.rs:33-103`）。proto の `WorkflowExecutionView`（`proto/client.proto:1373-1391`）はどの RPC・購読からも使われていない。
  - `WorkflowSubmitOutput`・`WorkflowGetOutput`・`WorkflowValidateOutput`（`proto/client.proto:2603-2605`）は `worktree_path` が必須で、サーバが worktree で認可する（`adaptor/controller/client/workflow/output.rs:19-39`）。この 3 つを呼ぶ本番のコードは無い（画面にも CLI にも無い）。
  - 診断は購読 `diagnostics`（引数なし、適用済みの config directory 固定。`usecase/state_subscription/reads.rs:340-346`）だけ。
  - review の購読は `review-threads:<worktree path>`（絞り込みなし、書き手は人）だけ。変更系（`CreateReviewThread` など）は worktree の文字列を受け、書き手は常に人（`adaptor/controller/client/comment/commands.rs:19-107`）。履歴の購読・RPC は無い。
  - provider の信号を受ける入口は無い。
- review の CLI（`cli/review.rs`）は HTTP を使わず、同じプロセス内で usecase を組み立ててファイルを直接読み書きする。`--session-id` から書き手（provider の agent）と worktree を引き、変更系では Session が open かを検査する（`:110-131`）。`--session-id` なしの一覧は `RELEASH_WORKTREE_PATH` を `workspace_worktree_path` で解決し（隔離 worktree の path は親の workspace の worktree に寄せる。`adaptor/gateway/workflow/worktree_context.rs:114-150`）、書き手は人で `--author`／`--unread` は使えない（`:179-210`）。
- hook（`cli/hook.rs:37-118`）は stdin を 65,536 byte まで読み、env の slot・binding・capability・agent session と合わせて、payload を CLI のプロセスの中で解釈し（`adaptor/gateway/provider_lifecycle/payload.rs:21-`）、解釈済みの信号を HTTP で送る。Claude の subagent の payload は送らずに成功にする（`:62`）。
- token は master（`local-api.json`）と client（`client-api.json`）の 2 つ（`infrastructure/local_api/server.rs:55-80`）。method 単位の権限は無い。hook は発見ファイルの master token を使う。
- `require_client` は Origin が無いか allowlist に無ければ 403 を返す（`api/auth.rs:68-79`）。画面側の Rust は Origin `tauri://localhost` を名乗って通している（`releash-desktop/src/adaptor/gateway/desktop_client.rs:81`）。
- `buf.yaml` は無く、CI（`.github/workflows/ci.yml`）に proto の lint・breaking の検査は無い。buf 1.47.2 で今の proto に STANDARD の lint を掛けると 406 件の違反がある（releash-e7 が実行して確認）。

# Scope / Non-goals

## Scope

- CLI と hook が使う入口を Connect に用意する: 実行の状態の購読、output の取得の購読、worktree を要求しない output の提出、directory を指定した診断、review の一覧（Session の形と worktree の path の形）・取得・履歴の購読、Session を指定した review の作成・コメント・解決、provider の信号を生の payload で受ける RPC。
- 状態を単発で返す `WorkflowGetOutput` と、呼び出し元の無い `WorkflowValidateOutput` を Connect から削除する。
- method ごとの scope（operator／hook）を proto に書き、サーバがそれに従って受け付ける。hook の token を作り、agent のプロセスに env で渡す。
- Origin が付いていない要求を通し、Rust 側の Origin の名乗りをやめる。
- `buf.yaml` を置き、lint と breaking を CI に入れる。AGENTS.md に同じコマンドを足し、token の 1 文を言い換える。

## Non-goals

- HTTP `/v1`、`require_bearer`、WebSocket subprotocol の対応、master token と `local-api.json`、`cli/file_direct.rs` の削除、CLI と hook の Connect への移行、CLI のクレート分け（#1902）。
- CLI のプロセス側の振る舞い（出力、終了コード、HTTP の利用、hook の health ファイルの読み書き）の変更（#1902）。
- `/v1/provider-lifecycle/unavailable` に当たる RPC。
- HTTP の validate（呼び出し側が contract を指定する意味論）を Connect に足すこと。
- 改名（#1903）、画面の監督の削除（#1904）、CLI の status など（#1905）。
- 今の proto にある 406 件の lint の違反を直すこと（ファイルの移動を含む）。`buf format` の導入。
- サーバの再起動をまたいで生き残る agent が持つ hook の token の扱い。
- review 以外の usecase の DTO（`ReviewSnapshotDto` など）や `usecase/provider_dto.rs:10` の `From` の見直し。
- `DaemonAdmission` と `Daemon::admits`（起動状態による受付）の変更。
- allowlist と CORS 自体の撤去（#78）。

# Requirements

- R-001: 実行の ID を指定して workflow の実行の状態を購読でき、その実行の状態が配信される。その ID の実行が無いときは「無い」という状態が配信される。ID の形が不正なときは読み取りの失敗になる。
- R-002: 実行の ID と Node 名を指定して output を購読でき、「提出済み（contract、値、提出時刻、request ID、時刻）」「未提出」「実行が無い」のいずれかが状態として配信される。その実行の定義に Node が無いとき、または ID の形が不正なときは読み取りの失敗になる。
- R-003: node-execution の ID を指定し、worktree を指定せずに output を提出できる。
- R-004: 絶対 path の directory を指定して workflow の定義を診断し、診断の結果を受け取れる。相対 path は引数の誤り（`InvalidArgument`）、存在しない directory は `NotFound` になる。
- R-005: Session の ID を指定して review の Thread の一覧を購読でき、その Session の worktree の Thread が、その Session の agent を閲覧者として配信される。file・state・author・unread・thread の絞り込みを指定でき、サーバが絞り込む。Session が無いときは「無い」という状態が配信される。
- R-006: worktree の path（Node の隔離 worktree の path を含む）を指定して review の Thread の一覧を購読でき、その path が属する workspace の worktree の Thread が、人を閲覧者として配信される。file・state・thread の絞り込みを指定でき、サーバが絞り込む。author・unread は指定できない。path を workspace の worktree に解決できないときは読み取りの失敗になる。
- R-007: Session の ID と Thread の ID を指定して、Thread とその履歴をそれぞれ購読できる。Session の lifecycle に関わらず配信される。Session または Thread が無いときは「無い」という状態が配信される。
- R-008: Session の ID を指定して review の Thread の作成・コメントの追記・解決ができ、書き手はその Session の agent になり、変更後の Thread が返る。Session が open でないときは `FailedPrecondition`、Session が無いときは `NotFound` になる。
- R-009: provider の hook の信号を、provider・slot・binding・capability・agent session と生の payload で受け取れ、サーバが payload を解釈して処理する。結果は「適用」「重複」「拒否（理由付き）」「無視（subagent の payload）」のいずれかで返る。payload が 65,536 byte を超えるとき、または解釈できないときは `InvalidArgument` になる。
- R-010: Connect の ClientService は `WorkflowGetOutput` と `WorkflowValidateOutput` を提供しない。
- R-011: ClientService の全ての RPC が、受け付ける token の scope（operator、hook）の集合を proto に持つ。operator の token は operator の RPC を、hook の token は `ReceiveProviderSignal` と `GetServerInfo` だけを呼べる。operator の token で `ReceiveProviderSignal` は呼べない。scope が合わない要求は `PermissionDenied`、どの token とも一致しない要求は `Unauthenticated` になる。ClientService に定義されていない method への要求は、token がどれかと一致すれば scope の検査を受けずに `Unimplemented` になる。
- R-012: hook の token はサーバの起動ごとに作られ、ファイルに書かれず、provider の agent の起動の env `RELEASH_PROVIDER_LIFECYCLE_TOKEN` で渡される。サーバの停止の後は受け付けられない。
- R-013: operator の token は、今の client の発見ファイル（`client-api.json`）の token である。発見ファイルの名前と形は変わらない。
- R-014: Origin が付いた要求は、今と同じく allowlist と照合される。Origin が付いていない要求は Origin の検査を受けずに token と scope の検査に進み、応答に CORS の header は付かない。Rust の client は Origin を名乗らない。
- R-015（互換性）: proto の変更は CI で `buf lint`（STANDARD）と `buf breaking`（WIRE_JSON、PR の base と比較）で検査される。この開発で足す RPC・message・enum は STANDARD の規則に従う。削除するフィールドと oneof の番号・名前は reserved にする。意図した非互換は PR の `buf skip breaking` ラベルで breaking の検査から外せる。
- R-016（互換性）: 画面が使っている既存の購読（`review-threads` など）と RPC（`CreateReviewThread`・`AppendReviewComment`・`ResolveReviewThread` など）の引数・応答・振る舞いは変わらない。HTTP `/v1` と master token は今と同じに動き、CLI と hook の振る舞い（出力の形を含む）は変わらない。
- R-017: AGENTS.md の「local API の master token を renderer JS へ渡さない」は「hook の token は hook のプロセスにしか渡さない」に変わり、「ビルド・テスト・Lint」に CI と同じ buf のコマンドと、`buf skip breaking` ラベルを効かせる手順が載る。

# Assumptions

- この ISSUE で足す Connect の入口には、#1902 まで本番の呼び出し元が無い（CLI と hook は HTTP を使い続ける）。releash-e7（利用者の中央管理セッション）が受け入れた。
- 実行の ID・Session の ID で引く購読の対象は、購読している間、どの worktree（または review comment）の変化でも読み直す。使い手（#1902 の CLI）は最初の状態を受け取って閉じるため、絞り込みは長く購読する使い手が出てきたときに足す。releash-e7 が受け入れた。
