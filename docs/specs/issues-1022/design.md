# Design

## Behavior Coverage
- Thread は主張となる初回 Comment を伴って成立する: review comment domain は Thread 作成を「Thread と初回 Comment の同時成立」として扱う。対象位置は任意の属性であり、位置付き Thread と位置不依存 Thread は同じ Thread 契約で扱う。
- Thread は worktree ごとに独立した議論として扱われる: review comment store は worktree を分離境界とし、List / Get / mutation は常に対象 worktree の範囲に閉じる。
- Thread の状態は open と resolved のみで表される: Thread lifecycle は `open` / `resolved` の 2 状態だけを公開し、`resolved` は終状態として扱う。Resolve には outcome と説明を持たせ、状態の細分化はしない。
- Comment は Thread 直下の時系列発言として追記される: Comment は Thread に属する append-only の発言列として扱い、返信ツリー、編集、削除は契約に含めない。
- 参加者の種類と Agent の同一性はレビュー議論上で区別される: Actor は human と agent を区別し、Agent の同一性は Releash が Agent session から解決した backend / model に基づく。session id は監査情報であり、Stance や Resolve 権限の同一性には使わない。
- Thread に対する Stance は参加者ごとに現在値を 1 つだけ持つ: Stance は Thread 単位かつ Actor 単位の現在値として投影する。再表明は同じ Actor の現在値を置き換え、未表明の作成者は `none` として扱う。
- Resolve は起票者または人間だけが実行できる: Resolve 権限は Rust の review comment domain が判定し、human は任意の open Thread を Resolve できる。Agent は同じ backend / model 由来の Actor が作成した Thread だけを Resolve できる。
- Agent と人間は同じレビュー議論を観測できる: CLI / Tauri command / Remote WebSocket は同じ Rust usecase に接続し、Thread 一覧、詳細、現在 Stance、解決情報、新着発見に必要な更新情報を同じ意味で返す。
- 拒否された操作では理由を確認できる: 権限不足、resolved 後 mutation、無効な Thread 作成などの拒否は、入口ごとに独自解釈せず Rust 側の同じ理由分類として返す。
- 既存のレビューコメント導線は新しい議論モデルを表示し操作できる: Desktop UI と Remote UI は新しい Thread / Comment / Stance / Resolve の投影 DTO を表示し、人間操作の入力入口として振る舞う。意味解釈と権限判断は持たない。
- Thread の履歴は監査目的で確認できる: 永続化される履歴は Thread の作成、Comment 追加、Stance 表明、Resolve を時系列に確認できる契約を持つ。通常の List / Get は現在状態の投影を返し、監査用の取得では履歴を返す。
- 並行した操作は確定した順序に従って一貫した状態になる: 書き込み境界は worktree ごとに順序を確定し、Comment は欠落させず、Stance は確定順序上の最後の値を現在値にし、Resolve は最初に有効確定したものだけを終状態にする。
- レビュー議論は Releash ローカルの対象に限定される: この設計の対象は Releash ローカルの review comment 基盤であり、GitHub PR review comment や外部サービス同期は契約に含めない。
- 人間は Diff ビュー上の Thread を現在の Agent との対話に共有できる: Desktop UI は対象 Thread の参照情報を組み立て、active な AgentChat session の入力として送る経路を持つ。active な session が無いときは送信不可とする。
- Agent は Releash 上で起動した時点でレビュー議論操作の方法を把握している: Releash は Agent process 起動時に `releash` CLI の long help を system_prompt に含めることで、Agent が起動直後から CLI 操作を把握している状態を保証する。

## Key Decisions
- 主境界は CLI / API とする。Issue #1022 と milestone #69 の目的は Agent が review comment 基盤を CLI/API 経由で扱うことであり、MCP tool は今回の主要入口にしない。
- Review comment は Thread / Comment / Stance / Resolve の議論モデルとして扱う。既存の単一コメントモデルでは、複数参加者の議論、現在の立場、解決権限、監査履歴を一貫して表せないため。
- Source of truth は worktree ごとの append-only 履歴とし、現在状態は Rust 側で投影する。Comment の append-only、Stance の現在値、Resolve の終状態を同じ順序モデルで扱えるため。
- Resolve は solo-arbitration とする。Agent 同士の多数決や自動合意では閉じず、Thread 作成者または人間だけが解決できる。人間は最終判断者として Agent より強い Resolve 権限を持つ。
- Agent identity は backend / model 由来の Actor とする。session ごとの分離は行わず、同じ backend / model の Agent は同じ参加者として Stance と Resolve 権限を共有する。
- CLI、Tauri command、Remote WebSocket は別々の意味論を持たない。同じ Rust usecase に接続し、経路差は入出力プロトコルと transport の違いに限定する。
- 既存コメントデータの自動移行は行わない。要求上の完了条件は新モデルへの接続であり、過去 JSON の変換は今回の契約外とする。
- Diff Thread の Agent への能動共有は、Agent への push 通知ではなく人間ユーザーのチャット入力の一形態として扱う。送信先は active な AgentChat session に限定し、active session が無いときは送信操作を提供しない。
- Agent への CLI 仕様伝達は system_prompt への long help append で行う。clap 由来の help を単一ソースとし、Agent 向けに別個に手書きしない。

## Responsibility Boundaries
- Review comment domain: Thread lifecycle、Comment append-only、Actor identity の意味、Stance 現在値、Resolve 権限、resolved 後 mutation 拒否を担当する。UI 表示、CLI flag、WebSocket transport、永続化ファイルの物理形式は担当しない。
- Review comment usecase: Create / List / Get / Append Comment / Set Stance / Resolve / History の業務境界を提供し、各入口から共通に呼ばれる。入口別の表示文言や画面状態は担当しない。
- Persistence gateway: worktree ごとの永続化、順序確定、履歴からの投影、同時書き込み時の一貫性を担当する。権限判断や Stance の意味解釈は担当しない。
- CLI controller: Agent 向け review command boundary を提供し、Releash が解決した Agent identity で usecase を呼ぶ。author を任意に偽装する入口は提供しない。
- Tauri command controller: Desktop UI の人間操作入口を提供し、human Actor として usecase を呼ぶ。ビジネスルールは持たない。
- Remote WebSocket handler: Remote UI からの review 操作を受け、Desktop UI と同じ human 操作として usecase に接続する。Remote 独自の review semantics は持たない。
- Frontend / Remote UI: Thread、Comment、Stance、Resolve 情報の表示、入力受付、呼び出し、表示用フォーマットを担当する。フィルタの意味、権限可否、状態遷移、集計は Rust から返された結果に従う。
- Agent backend orchestration: Agent process の起動時、Releash CLI の long help を system_prompt に組み込む責務を持つ。Agent 種別ごとの起動経路はこの system_prompt 注入規約に従う。

## Contracts
- CLI contract: Agent は CLI から review Thread の Create / List / Get / Resolve、Comment Append (Stance 表明を任意フラグとして同時更新可)、History 取得を実行できる。mutation は成功時に確定済みの結果を返し、拒否時には理由を機械的に判別できるエラーを返す。CLI は `--session-id` を唯一の参加者コンテキスト境界とし、worktree は session から解決する (`--worktree` 等の worktree 指定引数は提供しない)。
- Tauri command contract: Desktop UI は人間操作として Thread / Comment / Stance / Resolve / History を扱える command を呼び出す。返却値は UI がそのまま表示判断に使える投影 DTO とする。
- Remote message contract: Remote client は WebSocket 経由で Desktop UI と同等の human review 操作を要求できる。応答は同じ Thread / Comment / Stance / Resolve の意味を持つ。
- List contract: Thread 一覧は worktree を必須境界とし、以下 6 軸で絞り込める (各軸は AND、未指定は絞らない):
  - `author`: `self` (自分が作成した Thread) / `other` (自分以外が作成した Thread)
  - `state`: `open` / `resolved`
  - `stance`: 自分の現在 Stance が `agree` / `disagree` / `none` のいずれか
  - `unread`: 自分の最後の Comment 追記時刻以降に他者の Comment 追記があるか (true / false)。Stance 表明や Resolve は「Comment 追記」に含めない
  - `file`: Thread 対象の file path
  - `thread-id`: 指定 id の Thread のみ (複数指定可、配列内は OR、他軸とは AND)
  - 「自分」は呼び出し参加者コンテキスト (CLI なら `--session-id`、Tauri / WebSocket なら入口が解決する human Actor) から Releash 側が解決する。返却される一覧項目は、参加者が現在状態と次に確認すべき詳細を判断できる情報を含む。
- Get contract: Thread 詳細は Comment の時系列、参加者ごとの現在 Stance、Thread 状態、Resolve 情報、更新検出に必要な情報を返す。Agent の変化発見は push 通知ではなく List / Get の再取得で成立する。
- Rejection contract: 権限、Thread 状態、入力契約のいずれにより拒否されたかを CLI / Tauri command / Remote WebSocket で判別できる形で返す。表示文言は入口ごとに整えてよいが、拒否理由の意味は Rust 側の contract に従う。
- Thread contract: Thread は worktree に属し、作成者、状態、任意の対象位置、初回 Comment、現在 Stance 群、Resolve 情報、更新検出に必要な情報を外部に公開する。
- Comment contract: Comment は Thread に属する時系列発言であり、author kind、Agent 表示に必要な identity 情報、本文、作成時刻、監査用 metadata を公開する。編集・削除 contract は提供しない。
- Stance contract: Stance は Thread に対する Actor ごとの現在値であり、値は `agree` / `disagree` / `none` に限定する。Stance の書き込みは Comment 追記操作の任意フラグとして行い、Stance のみを単独で書き込む経路は提供しない。フラグで `agree` / `disagree` のいずれかを指定すると現在 Stance を上書きし、`none` を指定すると現在 Stance を未表明状態に撤回する。フラグ省略時は現在 Stance を維持する。
- Resolve contract: Resolve は open Thread を resolved にする終状態操作であり、実行者、outcome、説明、時刻を公開する。reopen contract は提供しない。
- History contract: Thread 単位で、作成、Comment 追加、Stance 表明、Resolve の履歴を時系列に取得できる。通常表示用の現在状態とは別の監査用 contract として扱う。
- Persistence document contract: worktree ごとの review comment 履歴は append-only な event document として保持する。外部 contract はイベントの意味と履歴取得可能性であり、内部 helper や具体的な読み書き手順は実装に委ねる。
- Agent process environment contract: Releash が起動する Agent process には、自分の session_id が `RELEASH_SESSION_ID` 環境変数として渡される。Agent CLI 呼び出し時はこの値を `--session-id` に渡せば、Releash 側が session から identity (backend / model) を解決して Actor を確定する。Agent は session_id を facet template や自前の query で導出する必要はない。さらに、Agent process の system_prompt には Releash CLI の long help が常に含まれる。Agent は help を別経路で取得する必要を持たない。
- Thread handoff contract: Desktop UI は対象 Thread の参照情報を含む Agent 共有メッセージを Rust 側で組み立て、active な AgentChat session の入力として送れる Tauri command を呼び出せる。メッセージ本文の整形は Rust contract に従い、Agent は受け取った参照情報から既存 review CLI 経路で Thread 詳細を確認できる。

## Data / Communication Flow
- Agent mutation flow: Agent CLI invocation は Releash が管理する session context から Agent Actor を確定し、review usecase に要求を渡す。usecase は domain rule を適用し、gateway が worktree 履歴へ確定し、CLI は確定後の Thread / Comment / Stance / Resolve 結果を返す。
- Human desktop flow: React UI は入力を受けて Tauri command を呼ぶ。Tauri command は human Actor として review usecase に接続し、Rust 側で確定した投影結果を UI に返す。
- Human remote flow: Remote UI は WebSocket message を送る。ws_server は認証済み remote session の human 操作として review usecase に接続し、結果を Remote UI に返す。
- Query flow: List / Get は worktree と filter を入口から受け取り、gateway が履歴から現在状態を投影し、usecase が外部 contract の DTO として返す。Agent の新着発見はこの Query flow の能動ポーリングで成立する。
- History flow: Thread history 要求は対象 worktree と Thread を指定し、gateway が対象 Thread の履歴を時系列で返す。これは監査目的の flow であり、現在状態の List / Get と混同しない。
- Change reflection flow: Desktop / Remote の人間操作で状態が変わった場合、UI 更新に必要な通知は既存のアプリ内更新経路に接続する。Agent への push 通知や subscribe は提供せず、Agent は List / Get で変化を取得する。
- Human-to-agent thread handoff flow: Desktop UI は対象 Thread を選択し Tauri command を呼ぶ。Rust 側で Thread 参照情報を含むメッセージを組み立て、UI はそれを active な AgentChat session の入力として送信する。送信先 session の判定（workflow step session 除外を含む）は UI 側で AgentChat 表示と同じ規則を用いる。
- Agent process bootstrap flow: Releash が Agent process を起動するとき、Releash CLI の long help を system_prompt に append し、その合成結果を Agent backend の起動入力として渡す。

## State Ownership
- Thread / Comment / Stance / Resolve の正: Rust persistence gateway が管理する worktree ごとの review comment 履歴。
- 現在状態の投影: Rust review usecase / gateway。UI や Remote は独自に Stance 集計、Resolve 可否、状態遷移を再計算しない。
- Domain invariants: Rust review comment domain。append-only、resolved 終状態、Stance 上書き、Resolve 権限をここが owner として扱う。
- Agent Actor: CLI / command 入口で Releash の Agent session から解決された backend / model 由来の identity。
- Human Actor: Tauri command と Remote WebSocket の人間操作入口。個別 user id ではなく単一ローカル操作者として扱う。
- UI local state: 選択中 Thread、入力中本文、送信中表示、展開状態などの一時表示状態。レビュー議論の正規状態は owner ではない。

## Boundaries
- フロントエンドにレビュー議論の意味論を置かない。権限判定、Stance 集計、resolved 後拒否、filter の意味解釈は Rust が担う。
- CLI と Tauri / Remote で別々の業務ロジックを持たない。すべて同じ usecase と domain rule を通す。
- GitHub PR review comment とは接続しない。Releash ローカルの review comment と外部サービスの review comment を同一視しない。
- MCP は今回の主境界にしない。CLI/API を Agent と UI / Remote の共通入口とする。
- Comment / Thread の編集、削除、reopen は公開しない。訂正や取り下げは新しい Comment または Resolve metadata で表現する。
- Agent は外部から任意 author として投稿できない。正式な Agent 操作は Releash が起動し identity を解決できる Agent session に限定する。
- 人間の Resolve 超越権限は Rust domain rule として扱う。UI 側のボタン表示は補助であり、拒否可否の正ではない。
- worktree 境界を越えた Thread 操作は行わない。一覧、詳細、履歴、mutation は対象 worktree 内に閉じる。
- resolved 後の mutation は受け入れない。後続議論が必要な場合は新しい Thread として表現する。
- Stance を単独で書き込む経路は持たない。Stance の表明・撤回は Comment 追記操作と一体で扱う。
- Diff Thread の Agent 共有メッセージの整形は Rust が owner であり、フロントエンドはメッセージ本文の作成を行わない。
- Remote UI (`src/remote/`) は本 Spec のスコープ外であり、Diff Thread → Agent 共有機能と system_prompt への CLI help 注入は Desktop 入口のみを対象とする。Remote 経路は別 Issue として扱う。

## Implementation Freedom
- CLI サブコマンド名、flag、標準出力の詳細形状、Tauri command 名、WebSocket message 名は、上記 contracts を満たす範囲で実装時に決めてよい。
- 永続化 document の具体フィールド、event schema の内部表現、snapshot / cache の有無は、外部 contract と監査可能性を保つ範囲で実装に委ねる。
- List / Get の filter セットと並び順の詳細は、worktree、file、状態、著者、自分の Stance、新着発見に必要な条件を満たす範囲で実装に委ねる。
- Agent 表示名の生成方法は、backend / model 由来で human と区別でき、session id に依存しない範囲で実装に委ねる。
- UI の配置、既存 ReviewPanel / inline diff / bottom comments への統合方法、Remote UI の画面構成は、Rust から返る contract に従う範囲で実装に委ねる。
- 履歴を通常 DTO に含めるか、監査用 API / CLI で別取得にするかは、通常表示と監査用途の contract を分けられる範囲で実装に委ねる。
- 同時書き込み時の順序確定方式とファイル保護方式は、Comment 欠落なし、Stance 最後勝ち、Resolve 単一確定、worktree ごとの破損防止を満たす範囲で実装に委ねる。

## Supplement: Review-only Workflow Candidate

この補足は、review comment 基盤を利用する後続の組み込み workflow 候補を示す。対象は Fix なしのレビュー専用 workflow とし、コード修正、修正方針承認、修正後 resolve は扱わない。workflow は Summary を出して完了し、人間は完了後に Summary と各 Thread を読み、必要に応じて Summary Agent と追加議論できる。

### Principles
- レビュー結果の正は OutputForm / structured output ではなく review Thread / Comment / Stance とする。
- Reviewer Agent はレビュー中に自分で review CLI を呼び、Thread 作成、Comment 追加、Stance 表明を行う。
- 投稿フェーズは既存 6 観点を Opus 4.7 と GPT-5.5 の 2 系統で実行し、合計 12 reviewer Agent が担当する。
- 投稿フェーズでは他者 Thread への反論や同意は行わず、指摘の投稿だけを行う。
- 投稿フェーズ後の議論、Stance 収集、最終 Stance 確定は 2 Agent が担当する。投稿した 12 reviewer Agent 全員を議論に参加させ続けない。
- 議論担当 2 Agent は全 open Thread を読み、各 Thread に必ず `agree` または `disagree` の Stance を表明する。
- `none` は未表明として扱う。`none` が残る Thread は議論の準備ができていないため `NOT_READY` とする。
- `disagree` には理由 Comment を必須とする。`agree` は補足がある場合のみ Comment を追加する。
- Agent 間の Split は最大 3 段階の固定ラウンドで扱う。無限に合意形成を続けない。
- 最終判断や修正実行はこの workflow では行わない。workflow は Summary を出して完了する。人間は workflow 完了後に Summary と Thread を読み、必要に応じて Summary Agent と自由議論する。

### Workflow Shape

```text
review_post_parallel
  ↓
discussion
  ↓
discussion_gate
  ├─ NOT_READY
  │   → discussion
  ├─ SPLIT
  │   → split_discussion
  └─ READY_FOR_SUMMARY
      → final_stance_collection

split_discussion
  ├─ claim
  ├─ rebuttal
  └─ restatement
      ↓
    final_stance_collection

final_stance_collection
  ↓
review_summary
  ↓
completed
```

### Phases
- 投稿フェーズ (`review_post_parallel`): 既存 6 観点を Opus 4.7 と GPT-5.5 の 2 系統で実行する。各 reviewer Agent は担当観点でレビューし、問題だと判断した内容だけを Thread として投稿する。投稿者は自分の Thread に `agree` を表明する。投稿フェーズでは他者 Thread への Stance、反論、誤検知指摘は行わない。
- 議論フェーズ (`discussion`): 議論担当 2 Agent は自分が未表明の他者 Thread を読み、議論を伴って判断し、各 Thread に `agree` または `disagree` を表明する。判断根拠は Comment として残す (Stance 表明は Comment 追記の任意フラグとして同時に行う)。`none` は未表明として扱い、残してはならない。
- Gate フェーズ (`discussion_gate`): Rust 側の集計により、Thread ごとの Stance matrix から workflow 全体の次状態として `NOT_READY`、`SPLIT`、`READY_FOR_SUMMARY` のいずれかを判定する。`NOT_READY` は議論の完了主張を Gate が検証して弾いた状態として扱い、議論フェーズへ戻す。`SPLIT` は追加議論へ進める。`READY_FOR_SUMMARY` は最終 Stance 確定へ進める。
- Split 議論フェーズ (`split_discussion`): Split した Thread だけを対象に、`claim`、`rebuttal`、`restatement` の 3 段階で議論する。再主張後も Stance が割れる場合は `SPLIT_FOR_HUMAN` として Summary 対象にする。
- 最終 Stance 確定フェーズ (`final_stance_collection`): 議論担当 2 Agent は、議論結果を踏まえて各 open Thread の最終 Stance を再表明する。ここでは追加議論を行わない。`none` は未表明として扱う。
- Summary フェーズ (`review_summary`): Summary Agent は全 Thread、Comment、Stance、Split 結果を読み、総評、合意済み指摘、修正不要合意、Split の論点、人間が判断すべき事項を整理する。Summary Agent はコード修正も Thread resolve も行わない。

### Gate Classification
- `NOT_READY`: `discussion` 完了後の Gate 検証で、少なくとも 1 つの open Thread に議論担当 2 Agent の Stance 未表明、または `none` が残っている。これは合意状態ではなく discussion phase の完了検証失敗であり、`discussion` に戻す。
- `SPLIT`: 議論担当 2 Agent の Stance は揃っているが、少なくとも 1 つの open Thread について `agree` と `disagree` が混在しており、かつ Split 議論の残ラウンドがある。対象 Thread を `split_discussion` に渡す。
- `READY_FOR_SUMMARY`: 議論担当 2 Agent の Stance は揃っており、追加議論すべき Split が残っていない。全 open Thread が Agent 間で合意済み、Split 議論の上限到達により Summary へ回すべき、または open Thread が存在しない場合を含む。

### Thread Classification For Summary
- `AGREED_ISSUE`: 対象 Thread について議論担当 2 Agent がどちらも `agree` を表明している。
- `AGREED_NO_ISSUE`: 対象 Thread について議論担当 2 Agent がどちらも `disagree` を表明している。
- `SPLIT_FOR_DISCUSSION`: 対象 Thread について議論担当 2 Agent の `agree` と `disagree` が混在しており、Split 議論の残ラウンドがある。
- `SPLIT_FOR_HUMAN`: `claim`、`rebuttal`、`restatement` 後も Split が解消しない、または Split 議論の上限に達した。

### Completion
- workflow は `review_summary` の完了をもって終了する。
- 完了後も Thread / Comment / Stance は review comment 基盤に残る。
- 人間は workflow 完了後に Summary と Thread を読み、必要に応じて Summary Agent と追加の自由議論を行える。
- この workflow は Fix なしで完結するため、修正実行、修正後の再レビュー、Fix 担当 Agent による resolve 権限は後続 workflow の検討対象とする。
