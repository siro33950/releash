# バックエンド アーキテクチャ概要

`src-tauri/` 配下のバックエンドは、**クリーンアーキテクチャ**に基づいたレイヤー構成を採用する。本ドキュメントは全体像と規約の入口。

## 目的

1. **ドメインとインフラの分離** — ビジネスロジックを Tauri / git2 / file I/O から切り離す
2. **ファイル分割の徹底** — 単一責務に従ったモジュール構造
3. **再利用性の向上** — ドメイン層を CLI 等の別エントリポイントから利用可能にする
4. **API層の分離** — Tauri コマンド / local API を薄い入口に閉じ込める

## 依存方向

```
infrastructure ← adaptor（controller / gateway / presenter）→ usecase → domain
```

依存は内向き（外側の層が内側の層に依存する）にのみ許される。adaptor/gateway だけは、変換の材料を得るために外側の infrastructure にも依存する。gateway が外部世界と内側を橋渡しする層だからである。

- ドメインは外側を一切知らない（依存を持たない）
- usecase の業務依存は domain に限る。Usecase自身が所有する非同期排他・通知等の実行制御primitiveは使用してよいが、外部世界との接続やその型を持ち込まない
- adaptor（gateway / controller / presenter）は usecase と domain に依存してよい（依存は内向き）
- adaptor/gateway は domain が定義する trait（repository 等）を実装し、集約読み取り等では usecase の DTO / query ポートにも依存してよい
- adaptor/controller は usecase を呼ぶ
- adaptor/gateway は infrastructure が提供する外部世界への接触能力を使う（[INFRASTRUCTURE.md](./INFRASTRUCTURE.md)）
- infrastructure は内側のどの層にも依存しない。domain 型を import せず、domain の trait を実装しない

**逆依存（内側の層が外側の層に依存すること）に例外はない。** 例えば domain → usecase、usecase → adaptor のような向きは禁止する。利便性（例:「ステートレスだから任意のエントリポイントから生成できる」）は逆依存を正当化しない。逆依存したくなった場合は、設計自体に問題があるサインとして扱う。なお DI 配線（composition root）は controller の責務とし、gateway や任意のエントリポイントへ配線責務を漏らさない。

## 横断的な設計原則

- **同じ操作の実装は 1 つに集約する。** 同一の操作（例: dirty count 算出、worktree 列挙）が複数箇所に実装されていること自体が問題であり、設定差異・挙動差はその症状にすぎない。単一の関数・イテレータに集約し、結果の一致を構造的に保証する。

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
- 旧Agent GUI specは現行正本ではない。

## ドメイン一覧（15個）

| ドメイン | 含まれる責務 |
|---|---|
| `code` | ファイル内容（at_ref, at_branch_base, staged）、diff、hunk、patch、staging（差分Approve）、language、file_mention、visible/hidden ranges |
| `repository` | branch、commit、log、worktree、status、repo_paths、git_config |
| `workflow` | 定義、実行木、Artifact、Contract、facet、completion と承認、Diagnostic |
| `local_event` | 永続 local event store の語彙。store identity、atomic batch、state mutation、query、transaction port |
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
- [GATEWAY.md](./GATEWAY.md) — ゲートウェイ層
- [INFRASTRUCTURE.md](./INFRASTRUCTURE.md) — インフラストラクチャ層
- [CONTROLLER.md](./CONTROLLER.md) — コントローラ層（Tauriコマンド／local API）
- [TEST.md](./TEST.md) — テスト方針
