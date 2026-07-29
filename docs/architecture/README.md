# バックエンド アーキテクチャ概要

`src-tauri/` 配下のバックエンドは、**クリーンアーキテクチャ**に基づいたレイヤー構成を採用する。本ドキュメントは全体像と規約の入口。

## 目的

1. **ドメインとインフラの分離** — ビジネスロジックを Tauri / git2 / file I/O から切り離す
2. **ファイル分割の徹底** — 単一責務に従ったモジュール構造
3. **再利用性の向上** — ドメイン層を CLI 等の別エントリポイントから利用可能にする
4. **API層の分離** — Tauri コマンド / WebSocket ハンドラを薄い入口に閉じ込める

## 参考実装

`no9-monorepo/services/revenue-server` のクリーンアーキテクチャ構成を参考にする。Releash は Tauri + 多様な外部リソース（git2、ファイル、WebSocket、外部API）を扱うため、一部の解釈は本リポジトリ向けに調整している。

## レイヤー構成（単一クレート）

```
src-tauri/src/
├── main.rs
├── lib.rs                              # DI配線（AppState組み立て + manage）
├── domain/                             # コアビジネスロジック（外部依存なし）
│   └── <bounded-context>/
│       ├── entities/                   # エンティティ（1構造体1ファイル）
│       ├── value_objects/              # 値オブジェクト
│       ├── repository.rs               # 永続化 trait（domain側で定義）
│       ├── gateway.rs                  # 外部リソース trait（Streamを返す）
│       └── services.rs                 # ドメインサービス
├── usecase/                            # アプリケーションビジネスルール
│   ├── <domain>_usecase.rs             # Command側
│   ├── <domain>_query_service.rs       # Query側（CQRS）
│   └── <domain>_dto.rs
├── adaptor/                            # インターフェースアダプタ
│   ├── controller/
│   │   ├── command/                    # Tauriコマンド（#[tauri::command]）
│   │   ├── handler/                    # WebSocketハンドラ
│   │   └── state.rs                    # AppState 構造体（DI受け皿）
│   ├── gateway/                        # Repository/Service の具体実装
│   │   └── <domain>/
│   │       ├── repository_impl.rs
│   │       ├── query_service_impl.rs
│   │       ├── service_impl.rs
│   │       ├── command_models.rs
│   │       ├── query_models.rs
│   │       └── service_models.rs
│   ├── presenter/                      # レスポンス整形
│   └── protocol/                       # WebSocketメッセージ等（リクエスト／レスポンス型）
├── infrastructure/                     # 外部世界の都合をその形のまま扱う
│   ├── agent_session/                  # Agent CLI のプロセスと wire
│   ├── file_watcher/                   # ファイル監視
│   ├── git/                            # git2 クライアント
│   ├── local_api/                      # ローカル HTTP サーバ / クライアント
│   ├── platform/                       # OS・Tauri プラットフォーム連携
│   ├── process/                        # 子プロセス起動と管理
│   └── telemetry/                      # テレメトリ送出
└── other/                              # 横断的関心事
    ├── error.rs                        # AppError（thiserror + serde::Serialize）
    └── logging/
```

### 依存方向

```
infrastructure ← adaptor（controller / gateway / presenter）→ usecase → domain
```

依存は内向き（外側の層が内側の層に依存する）にのみ許される。adaptor/gateway だけは、変換の材料を得るために外側の infrastructure にも依存する。gateway が外部世界と内側を橋渡しする層だからである。

- ドメインは外側を一切知らない（依存を持たない）
- usecase は domain にのみ依存する
- adaptor（gateway / controller / presenter）は usecase と domain に依存してよい（依存は内向き）
- adaptor/gateway は domain が定義する trait（repository 等）を実装し、集約読み取り等では usecase の DTO / query ポートにも依存してよい
- adaptor/controller は usecase を呼ぶ
- adaptor/gateway は infrastructure が提供する外部世界への接触能力を使う（[INFRASTRUCTURE.md](./INFRASTRUCTURE.md)）
- infrastructure は内側のどの層にも依存しない。domain 型を import せず、domain の trait を実装しない

**逆依存（内側の層が外側の層に依存すること）に例外はない。** 例えば domain → usecase、usecase → adaptor のような向きは禁止する。利便性（例:「ステートレスだから任意のエントリポイントから生成できる」）は逆依存を正当化しない。逆依存したくなった場合は、設計自体に問題があるサインとして扱う。なお DI 配線（composition root）は controller の責務とし、gateway や任意のエントリポイントへ配線責務を漏らさない。

## 横断的な設計原則

- **同じ操作の実装は 1 つに集約する。** 同一の操作（例: dirty count 算出、worktree 列挙）が複数箇所に実装されていること自体が問題であり、設定差異・挙動差はその症状にすぎない。単一の関数・イテレータに集約し、結果の一致を構造的に保証する。

## ドメイン一覧（14個）

| ドメイン | 含まれる責務 |
|---|---|
| `code` | ファイル内容（at_ref, at_branch_base, staged）、diff、hunk、patch、staging（差分Approve）、language、file_mention、visible/hidden ranges |
| `repository` | branch、commit、log、worktree、status、repo_paths、git_config |
| `workflow` | ワークフロー定義、実行、facet、承認 |
| `comment` | diff_comment_store、diff_comment_sender |
| `agent_session` | agent_sdk、session、agent_status |
| `pty_session` | PTY 管理全般 |
| `app_config` | 現 config.rs を分解 |
| `workspace_state` | ワークスペース状態保存 |
| `hooks` | フック設定・適用 |
| `notification` | webhook (Slack/Discord)、将来の他チャネル |
| `remote_access` | vpn_detect、qr_code、tls |
| `git_host` | GitHub PR/Issue |
| `notion` | Notion API |
| `external_editor` | 外部エディタ起動 |

## 各層の規約

- [DOMAIN.md](./DOMAIN.md) — ドメイン層
- [USECASE.md](./USECASE.md) — ユースケース層
- [GATEWAY.md](./GATEWAY.md) — ゲートウェイ層
- [INFRASTRUCTURE.md](./INFRASTRUCTURE.md) — インフラストラクチャ層
- [CONTROLLER.md](./CONTROLLER.md) — コントローラ層（Tauriコマンド／WebSocketハンドラ）
- [TEST.md](./TEST.md) — テスト方針
