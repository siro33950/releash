# Requirements

## Type
新機能

## Goal
Agent と人間が、Releash 上でレビューコメントを通じた議論と解決状況の追跡を行えるようにする。

完了時には、Agent は CLI 経由でレビュー議論に参加し、Thread 作成、Comment 追加、Resolve を実行できる。人間ユーザーは Releash の UI / Remote から同じレビューコメント基盤を利用でき、議論を閉じられる。

## Background
GitHub Issue #1022 は、マイルストーン `[09] Agent Review Capability - Review Comment CLI/API` の一部として、レビューコメント基盤を Agent 向け CLI と UI / Remote 向け API の command boundary へ接続することを目的としている。

ワークフロー操作は既に CLI/API 化が進んでいる一方、レビューコメントは人間 UI からの操作を中心とした単純なコメント機能に留まっている。そのため、Agent がレビュー上の議論へ参加し、他の Agent や人間と議論し、解決状況を追跡するための共通入口が不足している。

また、単一コメント中心のモデルでは、複数 Agent と人間が同じ主張について議論し、誰がどの権限で解決したかを追跡しにくい。レビューコメント基盤を Thread / Comment を持つ議論単位へ拡張することで、Agent review capability の土台を作る。

## Users / Actors
- 人間ユーザー: Releash のデスクトップ UI または Remote UI からレビューコメントを読み書きし、必要に応じて Thread を Resolve する。
- Agent: Releash の AgentChat 経由で起動され、CLI 経由でレビュー議論に参加する。
- Releash: Agent identity の解決、権限判定、レビューコメント状態の保持、現在状態の提供を担う system。
- Remote client: WebSocket 経由で Releash に接続し、人間ユーザー向けのレビューコメント操作を行う外部画面。

## Scope
- ローカル worktree ごとのレビューコメント基盤を、Thread / Comment を表現できるモデルにする。
- 対象は Releash 内のローカルレビューコメントとし、GitHub PR 上のレビューコメントとは独立して扱う。
- Thread は初回 Comment を伴う議論単位とし、コード位置に紐づく Thread と位置不依存の Thread の両方を扱えるようにする。
- Comment は Thread に属する発言として扱い、複数参加者の時系列議論を表現できるようにする。
- Thread の状態は `open` / `resolved` とし、Resolve 時の結論や説明を記録できるようにする。
- Agent は CLI 経由で Thread の作成、一覧取得、単体取得、Comment 追加、権限内の Resolve を実行できるようにする。
- 人間ユーザーは UI / Remote 経由で Thread の作成、一覧・詳細確認、Comment 追加、Resolve を実行できるようにする。
- Agent 向け CLI と UI / Remote 向け API は同じレビューコメント能力を利用し、経路によって意味や権限が変わらないようにする。
- Agent は能動的な List / Get によって、新着 Thread と Comment を発見できるようにする。
- Thread / Comment / Resolve の履歴を監査でき、Thread 単位で過去の変化を確認できるようにする。
- 既存のコメント導線は、新しいレビューコメント基盤で動作するようにする。
- 既存の手動送信状態を前提にしたコメント送信フローを、Agent が List / Get で発見するモデルに置き換える。
- Agent backend session 起動時、`releash` CLI の long help が system_prompt の一部として常時含まれるようにする。
- 人間ユーザーはデスクトップ UI 上の Diff 由来 Thread を、現在 active な AgentChat session の入力として能動送信できるようにする。
- active な AgentChat session が存在しない場合は、上記能動送信を実行できないようにする。

## Non-goals
- GitHub PR のレビューコメントを作成、更新、同期すること。
- MCP tool をレビューコメントの主境界として追加すること。
- Comment または Thread の編集・削除。
- resolved な Thread の reopen。
- 参加者ごとの賛否表明 (agree / disagree) など Thread に対する「立場」を集計・表示する機能。
- Agent への push 通知または event subscribe。
- 外部独立実行された Agent や外部スクリプトからの直接投稿を正式な Agent 操作として扱うこと。
- 複数の同一 backend/model Agent session を別参加者として区別すること。
- 既存コメントデータを自動移行すること。
- 既存の手動送信済み / 未送信状態を継続すること。
- ロールベース ACL、per-thread permission list、細分化された権限管理。
- 人間ユーザーによる Diff Thread の能動送信を、Agent への push 通知や event subscribe として扱うこと。本機能は人間ユーザーが自分の意思で行うチャット入力の一形態とする。

## Requirements
- Thread は、主張となる初回 Comment とともに作成されなければならない。
- Thread の対象位置は任意であり、file / line range 付きの Thread と位置不依存の Thread の両方を扱えなければならない。
- Thread は worktree ごとに独立して管理されなければならない。
- Thread の状態は `open` と `resolved` の 2 値でなければならない。
- `resolved` は終状態であり、Thread は reopen できてはならない。
- Resolve 時には、解決理由、結論、取り下げ、別 Thread への立て直しなどを説明できるメタ情報を残せなければならない。
- Comment は Thread 直下の時系列発言として追加されなければならない。
- Comment は append-only であり、投稿後に編集または削除できてはならない。
- Comment の著者は `agent` と `human` を区別できなければならない。
- Agent の同一性は、Releash が Agent session から解決した backend と model に基づかなければならない。
- 同じ backend/model の別 session は同一 Agent 参加者として扱われなければならない。
- session の識別子は監査情報として保存できるが、参加者の同一性判定（List filter / unread 判定）に使ってはならない。
- 人間は単一ローカル操作者として扱われ、個別 author id に依存しない。
- Resolve は、open Thread に対し参加者種別や起票者かどうかに関わらず実行できなければならない。
- resolved 後の mutation は拒否されなければならない。
- 並行して追加された Comment は、順序が確定すればどちらも失われず保持されなければならない。
- 同時 Resolve は、最初に有効として確定した Resolve だけが Thread を閉じなければならない。
- Agent 向け CLI は、Thread の Create / List / Get / Resolve、Comment Append を提供しなければならない。
- UI / Remote 向け API は、人間ユーザーが Thread / Comment / Resolve を操作できる能力を提供しなければならない。
- List / Get は worktree、file、状態、著者、未読状況などで必要な絞り込みができなければならない。
- Agent 向け CLI は、Agent が自分でポーリングして新着や変化を発見できる情報を返さなければならない。
- Thread / Comment / Resolve の履歴は、監査用途で Thread 単位に参照できなければならない。
- Review comment 操作が権限または状態により拒否される場合、利用者が理由を理解できる情報が返されなければならない。
- 既存のレビューコメント導線は、新モデルの Thread / Comment / Resolve を表示・操作できなければならない。
- UI は human / agent の著者種別と、Agent の backend/model 由来の表示名を区別して表示できなければならない。
- UI は Thread の open / resolved 状態、Comment の時系列、利用者が実行可能な Resolve 操作を確認できなければならない。
- 既存の Agent への手動送信状態を前提にしたフローは、新しい List / Get による発見モデルへ置き換えられなければならない。
- Agent backend 起動時の system_prompt には、`releash` CLI の long help が必ず含まれていなければならない。
- 人間ユーザーは Diff ビュー上の Thread から、Thread を一意に指す参照情報（thread id、worktree、file、line range）を含むメッセージを active な AgentChat session の入力として送信できなければならない。
- active な AgentChat session が存在しない場合、Diff Thread の能動送信操作は無効化されなければならない。
- 送信されたメッセージは、Agent が `releash review get` などの CLI 経路で Thread 本文・履歴を取得できる十分な参照情報を含まなければならない。

## Constraints
- 主境界は Agent 向け CLI と UI / Remote 向け API とし、MCP は今回の主境界にしない。
- 対象は Releash のローカルレビューコメント基盤であり、GitHub PR review comment とは分離する。
- Agent と UI と Remote は、同じ意味論と権限規則でレビューコメントを操作しなければならない。
- Agent 操作は Releash が起動した Agent session に紐づく identity 解決を前提とする。
- Agent の author identity は session 単位ではなく backend/model 単位とする。
- Resolve 権限は participant 識別に依存しない。
- Comment と Thread の履歴は監査可能でなければならない。
- worktree ごとの同時書き込みでデータが破損または欠落してはならない。
- 既存コメントデータの自動移行は要求しない。
- フロントエンドは表示、入力受付、API 呼び出し、表示用フォーマットに徹し、レビューコメントの意味解釈や権限判断を持たない。

## Success Criteria
- Agent が CLI から review Thread を作成し、Comment を追加し、open Thread を Resolve できる（起票者か否かを問わない）。
- 人間ユーザーが UI / Remote から review Thread を作成し、Comment を追加し、任意の open Thread を Resolve できる。
- resolved な Thread に対する Comment 追加、再 Resolve、reopen は受け入れられない。
- 権限または状態により拒否された操作について、Agent と人間ユーザーが拒否理由を確認できる。
- List / Get により、Agent と人間が Thread の現在状態、Comment 時系列、Resolve 情報を確認できる。
- 同じ backend/model の Agent が session を跨いでも、同じ参加者として List filter / unread 判定で扱われる。
- 並行操作が発生しても、Comment の欠落、二重 Resolve、永続化データの破損が起きない。
- 既存の review comment UI 導線が新しい Thread / Comment / Resolve モデルで利用できる。
- GitHub PR review comment 操作や MCP tool 追加なしに、Agent 向け CLI と UI / Remote 向け API の主境界としての review capability が成立する。

## Open Questions
なし。
