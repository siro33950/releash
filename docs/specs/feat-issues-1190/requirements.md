# Requirements

## Type

バグ修正

対象 Issue: #1190

## Goal

Session を再起動・履歴復帰したあと、復帰した Session で発話したとき、Agent が復帰前の会話コンテキストを保持しており、続きとして応答する状態にする。

完了時には、UI 上で「引き継ぎ済み」に見える復帰 Session が、表示上のメッセージ履歴だけでなく Agent 側の会話コンテキストも実際に引き継いだ状態で動作し、復帰直後の文脈依存の発話に対して Agent が過去のやり取りを踏まえて応答する。

## Background

Releash を再起動、または Closed セッションを「履歴から復帰」したとき、UI 上は Session を引き継いでいるように見える（セッション一覧に表示され、メッセージ件数や最初のメッセージも表示される）。しかし実際には、復帰した Session が会話の記憶（Agent プロセス側の会話コンテキスト）を失った状態で Start してしまう。復帰後に発話しても、それ以前のやり取りを Agent がまったく覚えていない。

調査により、Releash の Rust 側（`SessionStore` 〜 `SessionLifecycleController`）は `ChatSession.messages` を `~/.releash/sessions/{session_id}.json` に永続化し、復帰時も messages を保持していることが確認できている（messages の表示が復元されるのはこのため）。一方、Agent プロセス側のコンテキスト復帰は別経路で行われている。

確認できた経路（仮定を含む）:

- Claude backend: `build_init_cmd()`（`bridge_common.rs`）が起動時に `agent_session_id` を `"sessionId"` として Bridge に渡し、Bridge（`claude-sdk-bridge.mjs`）が `options.resume = cmd.sessionId` として Claude Agent SDK の resume 機能に渡している。SDK の resume は SDK 自身が保持するスレッド履歴を参照して復帰する仕組みであり、Releash が保持する `messages` を Agent へ再注入しているわけではない。
- `agent_session_id`（SDK の session_id）は、ターンが正常完了したとき（`turn_complete` かつ正常系）にのみ SessionStore へ永続化される。session_ready 受信時点ではプロセスのメモリ上に保持されるのみ。
- Codex backend は thread_id を介する別経路で、復帰の扱いが Claude backend と異なる。

このため、「UI 上は履歴が見えるのに Agent は記憶を失っている」という症状は、表示用の `messages` 復元と、Agent プロセス側のコンテキスト復帰が独立しており、後者が成立していない（または成立を保証できていない）ことに起因すると考えられる。原因の確定は behavior / design 検討時に切り分ける。

## Users / Actors

- 復帰した Session で会話を続けようとするユーザー（デスクトップ UI / リモート UI）
- 復帰した Session を実行する Agent（Claude backend / Codex backend）
- Session の永続化・復帰・Agent プロセス起動を担う Releash

## Scope

- 再起動後に Session 一覧から開いて続行する場合、および Closed セッションを「履歴から復帰」する場合の双方で、Agent が復帰前の会話コンテキストを保持した状態で発話を続行できるようにする。
- Agent プロセス側のコンテキスト復帰が、表示用の messages 復元と整合する（UI 上「引き継ぎ済み」に見える状態と、実際の Agent の記憶状態が一致する）ようにする。
- Claude backend / Codex backend のそれぞれで、復帰経路が会話コンテキストを引き継げるようにする。
- コンテキスト復帰が成立しない場合に、UI 上で「引き継ぎ済み」と誤認させない、または成立しなかったことを利用者・システムが識別できるようにする。

## Non-goals

- Session の永続化フォーマット（`messages` / `agent_session_id` 等の保存内容）そのものの再設計。必要な範囲の修正は含むが、保存モデルの全面刷新は対象外。
- 会話コンテキストの要約・圧縮・トリミングといった、コンテキスト量の最適化機能の新設。
- Agent SDK / Agent CLI そのものの resume 仕様の変更（Releash 側で利用・補完する範囲に留める）。
- Claude / Codex 以外の新しい Agent backend への対応。
- メッセージ履歴の表示 UI そのものの仕様変更。
- #1178（Claude Agent SDK 経路の応答停止）の修正。関連経路を共有する可能性はあるが、本要求の対象は会話コンテキスト復帰に限定する。

## Requirements

- 再起動後に Session を開いて発話したとき、Agent が復帰前の会話コンテキストを踏まえて応答すること。
- Closed セッションを「履歴から復帰」して発話したとき、Agent が復帰前の会話コンテキストを踏まえて応答すること。
- UI 上で「引き継ぎ済み」に見える復帰 Session は、表示用メッセージ履歴だけでなく Agent 側の会話コンテキストも引き継いだ状態であること。
- Claude backend で復帰した Session が会話コンテキストを引き継げること。
- Codex backend で復帰した Session が会話コンテキストを引き継げること。
- 復帰に必要な識別子（Claude の `agent_session_id`、Codex の thread_id 等）が、正常な利用フローで欠落せず永続化・復帰されること。
- resume が技術的に成立しない backend やケースであっても、Releash が保持する `messages` を Agent へ再注入することで、全 backend で会話コンテキストの引き継ぎを保証すること。
- 会話コンテキストの復帰が成立しなかった場合に、UI 上で「引き継ぎ済み」と誤認させないこと（成立しなかったことを利用者またはシステムが識別できること）。

## Constraints

- 表示上のメッセージ履歴の状態と、Agent プロセス側の会話コンテキストの状態が食い違わないこと（「見えているのに覚えていない」状態を残さない）。
- 復帰経路の変更が、新規 Session の開始や通常のターン継続の挙動を壊さないこと。
- Claude backend と Codex backend で復帰の意味（会話コンテキストを引き継ぐこと）が一致すること。
- 既存の永続化済み Session（旧フォーマットで保存されたもの）を開いた場合に、意図せずコンテキストや履歴を破壊しないこと。
- ロジックは Rust（Tauri バックエンド）側に置く方針（`.claude/rules/rust-first-logic.md`）に従うこと。

## Success Criteria

- 数ターン会話して文脈を作った Session を、再起動後に開いて文脈依存の質問をしたとき、Agent が以前の会話を踏まえて応答する。
- 同様の Session を「履歴から復帰」して文脈依存の質問をしたとき、Agent が以前の会話を踏まえて応答する。
- Claude backend / Codex backend のそれぞれで、上記の復帰後コンテキスト保持が確認できる。
- 復帰識別子の永続化・復帰が、正常フローで欠落しないことが検証で確認できる。
- 会話コンテキストの復帰が成立しないケースで、UI 上「引き継ぎ済み」と誤認させない（または成立しなかったことを識別できる）ことが確認できる。

## 仮定

- 本要求のスコープは Claude backend と Codex backend の両方を対象とする（Issue の検証項目で「どちらで再現するか切り分け」が未確定のため、安全側に両 backend を対象に含める）。再現が片側のみであっても、もう一方が確実に引き継げることを確認対象に含める。
- 全 backend で会話コンテキストの引き継ぎを保証する。達成範囲として、resume が技術的に成立しない backend やケースでは、Releash が保持する `messages` を Agent へ再注入することでコンテキスト復帰を成立させる（合意済み・案 A）。SDK / CLI の resume が成立する場合にそれを優先利用するか、再注入に一本化するか等の具体手段は behavior / design 検討時に決定する。
- 「会話コンテキストを引き継ぐ」とは、復帰前のやり取りを Agent が参照できる状態を指し、トークン上限等による自然な切り詰めは本要求の対象外とする。

## Open Questions

なし
