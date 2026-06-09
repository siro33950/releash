# Design

## The actual design

### Architecture
<!-- 責務分割・構造に関する設計判断 -->

#### Agent session context boundary

`agent_session` は単一の bounded context として扱う。セッションのライフサイクル、turn phase、permission/model/backend の選択、外部へ公開する状態の純粋な導出ルールは `agent_session` の domain/usecase に集約する。

一方で、Claude/Codex SDK 実行、PTY、MCP、ファイル I/O、Tauri event、WebSocket broadcast、runtime handle 管理は domain へ入れず、gateway/infrastructure または controller 側の責務として扱う。desktop / remote / workflow の入口は transport 差だけを持ち、同じ Rust usecase に接続する。

この方針により旧 `agent_sdk.rs`、`agent_status.rs`、`session/`、`backends/` の責務混在を名前だけ移すのではなく、Agent セッションの意味論と外部依存を分離する。

#### Legacy module removal strategy

移行後の最終構成では、旧 `agent_sdk.rs`、`agent_status.rs`、`session/`、`backends/` を compatibility facade や re-export shim としても残さない。Tauri command、WebSocket handler、workflow integration、`lib.rs` の composition root は、新しい `agent_session` の domain/usecase/gateway/infrastructure 構成へ直接接続する。

この方針は旧構成の責務混在を確実に解消できる一方、旧 module 名を前提にした既存呼び出し側を同時に更新する必要があり、移行差分は大きくなる。

#### Adjacent UI and workflow state

open tab registry、workflow-step session の制約、workflow engine との連携は、`agent_session` domain の内部状態としては持たない。`agent_session` usecase はセッション起動・停止・復元に必要な制約や副作用を抽象 gateway 経由で扱い、tab registry、workflow engine、Tauri window などの具体実装は controller/gateway/infrastructure 側に置く。

これにより desktop / remote / workflow の入口で同じ Agent セッション意味論を共有しつつ、UI 状態や workflow 実行基盤の都合を domain へ混入させない。

### Interface
<!-- 外部から見えるインターフェースに関する設計判断 -->

#### External command and protocol compatibility

Tauri command 名、WebSocket message 名、既存の主要 request/response shape は、移行に必須でない限り維持する。内部では新しい `agent_session` usecase、domain value object、typed error を使っても、controller / handler が既存の外部契約へ変換する。

ただし、旧 module 除去や新しい usecase 境界へ直接接続するために外部 contract の変更が不可避な場合は、フロントエンド／リモートクライアントの追従を許容する。その場合も変更範囲は必要最小限に限定し、単なる整理目的の protocol rename や shape 変更は行わない。

#### Entry point semantic parity

desktop Tauri command、remote WebSocket handler、workflow integration は、behavior spec が列挙する Agent セッション機能について同じ意味論を共有する。対象は起動、停止、状態確認・状態通知、出力 broadcast、メッセージ送信、中断、モデル変更、権限変更とする。

既存実装にある入口固有の細かな edge case は、behavior spec の維持に必要なものだけを残す。旧実装の都合で生じた差異は、新しい shared usecase へ寄せる過程で整理してよい。

### Data Model
<!-- データの持ち方に関する設計判断 -->

#### Domain session model and external mappers

`agent_session` は、セッション id、worktree、lifecycle state、turn phase、backend/model/permission 選択、workflow-step session marker など、Agent セッションの意味論を表す domain model を持つ。

永続化 JSON、Tauri command DTO、WebSocket protocol DTO、runtime handle 管理用の構造は domain model と同一視しない。controller / gateway が mapper を通じて変換し、永続化形式や wire 形式の都合を domain へ持ち込まない。

#### Persistent session state and transient runtime state

永続化する Agent session record は、再起動後の復元や履歴表示に必要な情報に限定する。runtime handle、実行中プロセス、stream、cancellation handle、一時 output buffer などは永続化対象にせず、gateway/infrastructure 側の transient state として扱う。

状態通知や status query で両者が必要な場合は、usecase が永続 session record と runtime gateway から得た transient state を合成する。domain model は外部プロセス handle の具体型を持たない。

### Database
<!-- 永続化先・スキーマに関する設計判断 -->

#### Existing session persistence compatibility

新しい database や storage backend は導入しない。既存の file-backed session persistence と JSON 形状は原則維持し、`agent_session` domain model とは repository mapper で変換する。

この方針により既存セッションの復元互換性を保ち、clean architecture への移行を永続化 schema 変更と切り離す。domain の都合だけで保存形式を変更しない。

### UI/UX
<!-- 画面・操作フローに関する設計判断 -->

#### No intentional UX changes

デスクトップ UI と remote UI の画面構成、操作フロー、表示文言は意図的には変更しない。必要な変更は、Rust 側の command/protocol 境界変更へ追従するための最小限の修正に限定する。

Agent セッションの起動、停止、状態表示、出力表示、メッセージ送信、中断、モデル変更、権限変更は、behavior spec 上のユーザー体験を維持する。

### Algorithm
<!-- 非自明なロジックに関する設計判断 -->

#### Status derivation and aggregation

session state、turn phase、runtime state から Agent セッションの公開状態を導く純粋なルールは domain に置く。永続 session record と transient runtime state の取得、複数セッションの集約、通知タイミングの制御は usecase が担う。

Tauri event や WebSocket broadcast の具体処理、wire DTO への変換は controller / presenter / gateway 側に置き、status 導出ロジックと通知 transport を分離する。

#### Interrupt and future queued turns

中断は usecase operation として扱う。domain は現在の turn、将来導入される pending queue、interrupt 後に次の指示を受け付けられる状態へ戻るための状態遷移ルールを表す。

実際の Claude/Codex runtime に対する cancel / abort / process handle 操作は runtime gateway に閉じ込める。usecase は domain の queue / turn state に中断を適用し、runtime gateway の結果を受けて永続状態・runtime 状態・通知を更新する。

将来キューを導入する場合も、backend bridge ごとに中断・キュー消化の意味論を分散させず、`agent_session` usecase と domain rule に集約する。

#### Model and permission change timing

model / permission の変更は persisted session state に保存し、現在実行中の turn には割り込まない。変更後の値は、次の user message または次に開始される turn から runtime に適用する。

これにより Claude/Codex backend ごとの実行中設定変更可否に依存せず、desktop / remote / workflow で同じ意味論を保つ。UI は保存済みの選択値を表示できるが、実行中 turn の runtime 挙動は開始時点の値に従う。

### Infra
<!-- インフラ構成・デプロイに関する設計判断 -->

#### Agent runtime infrastructure boundary

Claude/Codex SDK bridge、process spawning、stdio / stream handling、runtime coordinator、backend-specific permission flag conversion は infrastructure の具体実装として扱う。`agent_session` usecase は runtime gateway trait を通じて start、send、interrupt、close などの操作を依頼し、SDK や process handle の具体型には依存しない。

adaptor/gateway は domain/usecase が要求する抽象と infrastructure 実装を接続する境界とし、Tauri `AppHandle`、WebSocket broadcaster、file I/O、SDK client の詳細を domain/usecase へ漏らさない。

#### Backend and model catalog responsibility

`BackendId`、`ModelId`、permission mode、supported model catalog に関する選択値の妥当性は `agent_session` domain value object として扱う。Claude/Codex の実行方法、SDK 呼び出し、backend-specific permission flag 変換は infrastructure に置く。

backend/model catalog の供給元が将来 config や外部取得に変わる場合も、domain は選択値の意味論的検証を担い、取得・実行の詳細は gateway/infrastructure に閉じ込める。

## Alternatives Considered
<!-- 採らなかった主要な設計案と、却下理由・トレードオフ -->

#### Legacy-module-shaped migration

旧 `agent_sdk`、`session`、`agent_status`、`backends` に対応する新レイヤーファイルへほぼそのまま移す案は採らない。差分は小さくできるが、現状の責務混在を名前だけ変えて残すリスクが高く、requirements の clean architecture 移行意図に合わない。

#### Permanent compatibility shims

旧 module 名を facade や re-export shim として最終状態に残す案は採らない。既存呼び出し側の変更は抑えられるが、旧構成の除去という成功条件と衝突する。

#### Broad external protocol redesign

Tauri command / WebSocket protocol を architecture 移行に合わせて広く整理する案は採らない。Rust 側の形は整えやすいが、フロントエンド／remote 追従が増え、requirements の scope を超えやすい。

#### Current ChatSession as the domain model

既存 `ChatSession` 相当をそのまま domain model として継続する案は採らない。移行差分は小さいが、永続化、wire DTO、runtime 都合が domain に残りやすい。

#### New session storage schema

既存 session JSON から新 schema へ切り替える案は採らない。domain-owned schema は作りやすいが、既存セッション復元を壊すリスクがあり、今回の migration と storage migration を同時に抱えることになる。

#### Backend-owned interrupt semantics

interrupt / cancellation の意味論を backend bridge に任せる案は採らない。実装は局所化できるが、Claude/Codex や desktop/remote/workflow 間で中断後の状態・将来 queue の扱いが割れやすい。

## Cross-cutting concerns
<!-- セキュリティ・パフォーマンス・可観測性などの横断的考慮事項のうち、判断が分かれたもの -->

#### State and notification consistency

Agent セッションの起動、停止、中断、モデル変更、権限変更では、usecase が永続 session state と必要な runtime state の更新結果を確定した後に、公開 status を導出して通知する。

repository、runtime gateway、backend bridge が個別に外部通知を発火する設計にはしない。Tauri event / WebSocket broadcast は usecase が合成した状態を基準に controller / gateway から送ることで、desktop / remote の観測順序と意味論を揃える。

## Risks
<!-- 既知のリスク・不確定要素・追加調査が必要な点 -->

#### Large migration without compatibility shims

旧 module 名を最終状態だけでなく互換 shim としても残さないため、Tauri command、WebSocket handler、workflow integration、`lib.rs` wiring を同時に新構成へ切り替える必要がある。差分が大きくなり、部分移行中のコンパイル不能期間が長くなるリスクがある。

#### Existing edge cases

desktop / remote / workflow は behavior spec の範囲で意味論を揃える方針のため、旧実装に存在する入口固有 edge case をすべて保存するわけではない。既存テストが旧 edge case を固定している場合、仕様として残すか旧実装由来として整理するかの判断が必要になる。

#### Persistence mapper compatibility

既存 session JSON 形状を維持しつつ domain model を分離するため、repository mapper の互換性が重要になる。mapper の欠落は既存セッション復元や履歴表示の回帰につながる。

#### Runtime and notification ordering

永続状態、runtime 状態、status 導出、Tauri/WebSocket 通知を usecase で合成するため、失敗時や中断時の順序制御を誤ると desktop / remote の表示が一時的に不整合になる。

#### Future queue semantics

将来 pending queue を導入する前提で interrupt / turn state を設計するが、本 spec では queue 自体を実装しない。今回の状態モデルが将来の queue 消化、破棄、再開の意味論を狭めすぎないよう注意が必要になる。
