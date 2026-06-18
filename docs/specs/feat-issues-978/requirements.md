# Requirements

> 本ファイルは `workflow` ドメイン移行（[#978](https://github.com/siro33950/releash/issues/978)）の **大方針** を定める。実装で迷ったときに立ち返る判断基準。詳細なターゲット構造・責務マッピング・移行順序は [design.md](./design.md) を参照。

## 参照する規約

- [README.md](../../architecture/README.md) — 全体像・レイヤー構成・依存方向・ドメイン一覧
- [DOMAIN.md](../../architecture/DOMAIN.md) — ドメイン層（entities / value_objects / repository / gateway / services、Aggregates パターン）
- [USECASE.md](../../architecture/USECASE.md) — ユースケース層（Command / Query 分離、DTO、サブディレクトリ化）
- [GATEWAY.md](../../architecture/GATEWAY.md) — ゲートウェイ層（repository_impl / query_service_impl / service_impl、モデル分離）
- [CONTROLLER.md](../../architecture/CONTROLLER.md) — コントローラ層（Tauri command / WS handler、register 方式）
- [TEST.md](../../architecture/TEST.md) — テスト方針（層別の必須／柔軟、命名規則）

先行移行事例: [`agent_session`（#977）](../feat-issues-977/)（同じ no-shim 方針・register パターン・状態通知パターンを踏襲）

## Goal

`src-tauri/src/workflow/`（全 38,946 行、`engine.rs` が 17,891 行）を、クリーンアーキテクチャ規約に従ったレイヤー構成へ移行する。完了時には「ワークフロー定義・実行・facet・承認」の責務が domain / usecase / adaptor / infrastructure の各規約に沿って配置され、`tauri`・`git2`・ファイル I/O 等の外部依存が gateway／infrastructure に閉じ込められ、旧 `workflow/` モジュールが除去された状態になる。フロント／リモート／CLI から見える基本機能は引き続き提供する。

依存先の `agent_session`（#977）は本移行の対象に含めず、抽象 gateway 経由で参照するのみとする。

## 大方針（迷ったらこれに従う）

1. **責務境界として再構成する。名前だけの移設はしない。** domain ロジック（状態遷移・contract・並列集約・承認可否・変数描画・secret マスキング）と外部依存（SDK・I/O・Tauri event・WS broadcast）を実質的に分離する。
2. **依存は内向きのみ。逆依存に例外なし。** `infrastructure → adaptor → usecase → domain`（[README.md](../../architecture/README.md)）。「ステートレスだから／便利だから」は逆依存を正当化しない。
3. **domain は外部を知らない。** `tauri` / `git2` / `tokio` / ファイル I/O を domain で `use` しない（[DOMAIN.md](../../architecture/DOMAIN.md)）。
4. **配置は「どのドメインに凝集するか」で決める。** 外部リソース依存か否かは配置基準ではない（[DOMAIN.md](../../architecture/DOMAIN.md)）。
5. **Entity か read model かは「誰の都合で形が決まるか」で判断する。** 表示・転送（フロント／CLI の都合）の型は DTO / `query_models` であり domain に置かない。`*View`・`runtime_view` 系は read model（[DOMAIN.md](../../architecture/DOMAIN.md) / [USECASE.md](../../architecture/USECASE.md)）。
6. **CQRS は Command / Query のサービス分離。** QueryService は Usecase ではない。read model はデータソースから直接組み立て、`Entity → DTO` 詰め替えはしない（[USECASE.md](../../architecture/USECASE.md)）。
7. **オーケストレーション（順序制御・複数集約またぎ）は usecase の責務。** gateway は単一集約の純粋 I/O プリミティブに留める（[USECASE.md](../../architecture/USECASE.md) / [GATEWAY.md](../../architecture/GATEWAY.md)）。
8. **controller は薄い入口。** usecase のみ呼ぶ。QueryService / Repository を controller から直呼びしない（[CONTROLLER.md](../../architecture/CONTROLLER.md)）。
9. **同じ操作の実装は1つに集約する。** event projection・active run 解決など、複数箇所の重複実装自体を問題として単一経路に集約する（[README.md](../../architecture/README.md)）。
10. **外部契約は移行に必須でない限り維持する。** Tauri command 名・WS message 名・主要 request/response shape を保つ。整理目的だけの rename / shape 変更はしない。
11. **永続化形状は維持する。** `workflow_runs/` の JSON・event log 形式を保ち、domain model とは gateway の mapper で変換する。storage schema 変更を本移行と同時に抱えない。
12. **no-shim。** 旧 `workflow/` を compatibility facade / re-export shim として残さない。最終的に完全削除する。

## Constraints

- レイヤー間の依存方向が規約に従い、逆方向（内側→外側）の依存を生まないこと。
- 既存の CI 品質チェック（フロント lint / test / build、Rust fmt / clippy / test）が移行後も通過すること。
- 各サブIssue（#1031〜#1037）はそれぞれ単独でビルド・テストが通る単位とすること。
- domain / usecase / gateway 層のテストを必須とすること（[TEST.md](../../architecture/TEST.md)）。
- `lib.rs` の `invoke_handler!` 直接列挙を、ドメイン単位の `register` 関数経由に置き換えること（[CONTROLLER.md](../../architecture/CONTROLLER.md)）。

## Non-goals

- `workflow` 以外のドメイン（`agent_session` を含む依存先）の移行。参照のみとする。
- ワークフロー機能そのものの仕様追加・機能拡張（新ノード種別・新承認フロー・新状態の追加等）。
- フロント／リモートクライアント側のコード変更（外部契約の必須変更に伴う最小限の追従を除く）。
- 永続化 schema（`workflow_runs/` JSON・event log 形式）の変更。
- 新しい storage backend の導入。
