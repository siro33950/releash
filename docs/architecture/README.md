# バックエンド アーキテクチャ概要

`src-tauri/` 配下のバックエンドは、**クリーンアーキテクチャ**に基づいたレイヤー構成を採用する。domain の中の部品は DDD の置き方に従う。本ドキュメントは全体像と規約の入口。

## 目的

1. **ドメインとインフラの分離** — ビジネスロジックを Tauri / git2 / file I/O から切り離す
2. **ファイル分割の徹底** — 単一責務に従ったモジュール構造
3. **再利用性の向上** — ドメイン層を CLI 等の別エントリポイントから利用可能にする
4. **API層の分離** — Connect の ClientService / HTTP local API / Tauri コマンドを薄い入口に閉じ込める

## 部品の一覧

置いてよい部品は次の表のものだけである。表に無い部品は置かない。新しい種類の部品が要るときは、先にこの表を変える。

| 層 | 部品 | 役割 |
|---|---|---|
| domain | Entity・集約 | 業務の状態、遷移、不変条件 |
| domain | 値オブジェクト | 業務の値と、状態を持たない業務の規則 |
| domain | ドメインサービス | 複数の集約にまたがる業務の規則。外部世界が要るときは trait を domain に置き、adaptor/gateway が実装する |
| domain | Repository の trait | 集約の保存。adaptor/gateway が実装する |
| usecase | Usecase（Interactor） | アプリの業務手順 |
| usecase | Input Data / Output Data | 操作の入力と出力。Entity を参照しない単純なデータ。失敗も出力として表す |
| usecase | Output Boundary の trait | 結果を外へ出す口。画面への状態の配信もここを通る。adaptor/presenter が実装する |
| usecase | QueryService の trait | 読み取り要求に Output Data で答える口。adaptor/gateway が実装する |
| adaptor/controller | Controller | 転送の形を Input Data に変えて、Usecase を呼ぶ |
| adaptor/presenter | Presenter、転送のメッセージ型 | Output Data を転送の形とステータスコードに変える |
| adaptor/gateway | domain と usecase の trait の実装 | 内側の型と外部世界の形の変換 |
| infrastructure | 外部世界の駆動部 | 外部世界そのもの（SQLite、Web サーバ、process・PTY、HTTP、OS）。内側へつなぐ部分だけを書く |
| common | 横断的関心事の包み | 処理を外から包んで振る舞いを足す |
| Main | 配線（composition root） | 全ての部品を組み立てる。最も外側 |

gateway は層の名前であり、trait の種類の名前ではない。「Gateway」という trait は存在しない。

## 依存方向

```
infrastructure ← adaptor（controller / gateway / presenter）→ usecase → domain
usecase / adaptor / infrastructure → common
Main → 全ての層
```

依存は内向き（外側の層が内側の層に依存する）にのみ許される。adaptor/gateway だけは、変換の材料を得るために外側の infrastructure にも依存する。gateway が外部世界と内側を橋渡しする層だからである。

- ドメインは外側を一切知らない（依存を持たない）。common も使わない
- usecase の業務依存は domain に限る。Usecase自身が所有する非同期排他・通知等の実行制御primitiveは使用してよいが、外部世界との接続やその型を持ち込まない
- adaptor（gateway / controller / presenter）は usecase と domain に依存してよい（依存は内向き）
- adaptor/gateway は domain の trait（Repository、ドメインサービス）と usecase の trait（QueryService）を実装する
- adaptor/presenter は usecase の Output Boundary を実装する
- adaptor/controller は usecase を呼ぶ
- adaptor/gateway は infrastructure が提供する外部世界への接触能力を使う（[INFRASTRUCTURE.md](./INFRASTRUCTURE.md)）
- infrastructure は内側のどの層にも依存しない。domain 型を import せず、domain の trait を実装しない
- common はどの層にも依存しない

**逆依存（内側の層が外側の層に依存すること）に例外はない。** 例えば domain → usecase、usecase → adaptor のような向きは禁止する。利便性（例:「ステートレスだから任意のエントリポイントから生成できる」）は逆依存を正当化しない。逆依存したくなった場合は、設計自体に問題があるサインとして扱う。DI 配線（composition root）は Main の責務とし、gateway や controller へ配線責務を漏らさない。

## 横断的な設計原則

- **同じ操作の実装は 1 つに集約する。** 同一の操作（例: dirty count 算出、worktree 列挙）が複数箇所に実装されていること自体が問題であり、設定差異・挙動差はその症状にすぎない。単一の関数・イテレータに集約し、結果の一致を構造的に保証する。
- **横断的関心事は、処理の中に書かず、包みとして掛ける。** 包みの定義は common に置く。掛ける位置は、受け手の側（controller の手前）、Usecase の入口、出ていく側（adaptor/gateway と infrastructure）の 3 つ。domain は包みを持たない。

### Agent TUIの状態所有

- canonical語は `AgentSession` である。
- Releashは `Turn`、`Message`、`MessagePart`、`PermissionRequest` を所有しない。
- Provider CLI / transcriptがconversationの正本である。
- `AgentSession`はlifecycleとTerminal ownershipを所有する。
- Terminal Surfaceは`Workspace`または`AgentSession`に所有される。
- `NodeExecution`は`AgentSession`を参照するが所有しない。
- Workflow completionとAgentSession lifecycleは独立する。
- Submit / Stop / Approval / ArtifactはWorkflowが所有する。
- Provider lifecycleとProvider availabilityは別の境界である。

## ドメイン一覧（16個）

| ドメイン | 含まれる責務 |
|---|---|
| `code` | ファイル内容（at_ref, at_branch_base, staged）、diff、hunk、patch、staging（差分Approve）、language、file_mention、visible/hidden ranges |
| `repository` | branch、commit、log、worktree、status、repo_paths、git_config |
| `workflow` | 定義、実行木、Artifact、Contract、facet、completion と承認、Diagnostic |
| `local_event` | 永続 local event store の語彙。store identity、atomic batch、state mutation、query、transaction port |
| `local_api_discovery` | local API discovery の内容、プロセス観測、接続先観測に基づく受理・拒否判定 |
| `workspace_tree` | Workspace / Session の bounded な query 集約。canonical な execution / node / session record から復元する |
| `comment` | diff_comment_store、diff_comment_sender |
| `agent_session` | AgentSession identity、lifecycle、Provider、Terminal ownership |
| `terminal_surface` | Terminal の backend 実装（durable terminal surface: PTY runtime lifecycle、attachment、入力 ingress、registry） |
| `app_config` | アプリ設定、secret、Notion 設定の永続化境界 |
| `workspace_state` | ワークスペース状態保存 |
| `app_data_gc` | アプリケーションデータの GC 対象分類（削除済み workspace、再生成可能キャッシュ等） |
| `provider_lifecycle` | Provider session、transcript参照、StopとAgentSession / NodeExecution attemptの関連付け |
| `git_host` | GitHub PR/Issue |
| `notion` | Notion API |
| `external_editor` | 外部エディタ起動 |

## 各層の規約

- [DOMAIN.md](./DOMAIN.md) — ドメイン層
- [USECASE.md](./USECASE.md) — ユースケース層
- [CONTROLLER.md](./CONTROLLER.md) — コントローラ層（Connect／local API／Tauriコマンド）
- [PRESENTER.md](./PRESENTER.md) — プレゼンター層
- [GATEWAY.md](./GATEWAY.md) — ゲートウェイ層
- [INFRASTRUCTURE.md](./INFRASTRUCTURE.md) — インフラストラクチャ層
- [TEST.md](./TEST.md) — テスト方針
