# ユースケース層 規約

## 原則

- **アプリケーション固有の業務手順**を表現する
- 外部世界へのアクセスは domain 層の trait または usecase 層の query port を介し、具体実装は知らない
- 外部世界との接続を担う依存禁止（`tauri`, `git2`, `reqwest` 等を直接 `use` しない）。Usecase自身が所有する非同期排他・通知・タスク協調には`tokio::sync`等の実行制御primitiveを使用してよいが、I/O、時刻、process、transport、永続化をUsecaseへ持ち込む理由にはならない
- CQRS に従い、Command の業務手順は Usecase、Query の読み取り処理は QueryService に分離する。両者は別ファイルに置き、controller からの入口は読み取りも含めて Usecase に統一する
- **QueryService は Usecase ではない。** Usecase はアプリケーション固有の業務手順（オーケストレーション）を表現する唯一の単位であり、QueryService は読み取りクエリのサービスにすぎない。「ユースケース」と呼んでよいのは Usecase のみ。QueryService を「Query 側ユースケース」等と呼んで usecase 扱いしない
- **usecase は状態機械を持たない**: 状態・ライフサイクルの表現主体は domain の集約である（[DOMAIN.md](./DOMAIN.md) モデルが実行を担う）。usecase が持つのは「何を、どの順で呼ぶか」であり、「この状態でこの操作を受理してよいか」は domain に問う。domain の状態型を usecase で再定義しない
- **usecase が肥大化したら domain の欠落を疑う**: 手順ではなく判断（受理可否・遷移・分類・検証）が usecase に溜まっているなら、それは domain 集約またはドメインサービスに引き上げるべきものである。特に、対応する `domain/<name>/` が存在しないまま usecase に状態と判断が集まっている場合は、domain 境界の欠落を意味する

> **CQRS は「Command/Query のサービス分離」であって、「Repository を read 用 / write 用の trait に分割すること」ではない。** Repository は読み書きを問わず Entity を生成・取得する単位であり、read メソッドを持つこと自体は CQRS 違反ではない。Query 専用のテストダブルが未使用の write メソッドを実装させられる程度のことは、trait 分割の理由にならない。

## Usecase

読み取り専用の操作と、書き込み・状態変更を伴う操作を提供するアプリケーション操作の入口。Repository / Gateway / QueryService を組み合わせて業務手順を実行する。アプリケーション層で唯一「ユースケース」と呼べる単位であり、読み取りと書き込みを跨ぐオーケストレーション（例: 一覧取得後にそのタイミングで GC を実行する等）もここに集約する。読み取り専用の操作では QueryService に委譲し、DTO を返してよい。QueryService 等の読み取り部品は Usecase から呼ぶ協力者であって、Usecase ではない。

**複数の集約・Repository をまたぐオーケストレーションは usecase の業務手順である。** 操作の順序制御も usecase が持つ。例: 「ブランチ削除前に、紐づく worktree を先に削除する」——これは git の機構的制約（checkout 中ブランチは削除不可）に由来する順序だが、複数集約をまたぐ手順なので usecase の責務とする。gateway は単一集約に対する純粋な I/O プリミティブに分解し、業務手順を gateway に潰し込まない。usecase が肥大化した場合は domain サービスの導入を検討する（[DOMAIN.md](./DOMAIN.md) ドメインサービス）。

## QueryService（Query 側）

読み込み専用のクエリサービス。**Usecase ではない**（「Query 側ユースケース」ではない）。表示向けに整形した DTO を返す。

**Query 側の port が usecase 層にあるのは、ドメインの言語をスキップするためである。** DTO は読み取り要求の出力仕様であり、その言語はフロント側の都合で決まる。domain 層の port がドメインの言語で書かれる（[DOMAIN.md](./DOMAIN.md) port）のに対し、Query 側の port はフロントの言語で書かれる。port の置き場所が、その port の話す言語を決めている。

**Query 側は読み取り要求に応えて、データソースから read model を直接組み立てて返す。** DTO は読み取り要求の出力仕様であり、その形は要求の都合で決まる。Entity を生成する Repository を再利用して `Entity → DTO` に詰め替えてはならない——向きが逆である（DTO は要求起点であって Entity 起点ではない）。1:1 写像に見える場合も例外ではない。

集約・表示集計（例: ブランチ + worktree 配置 + ahead/behind + マージ状態をまとめた一覧）でも同じである。Entity を構築して詰め替えるのではなく、QueryService 実装がデータソースから read model を直接組み立てて返す。read model は domain の Entity ではない（[DOMAIN.md](./DOMAIN.md)「Entity か DTO か」）。

## DTO

- DTO は **QueryService（Query 側）が返す Response** である。読み取り要求（ユースケース／画面）の出力仕様であり、その形は要求の都合だけで決まる。
- ドメイン（Entity）から導かれない。QueryService がデータソースから直接組み立てる read model であり、**「ドメイン型 ↔ DTO の変換」という工程は存在しない**。`From<Entity> for Dto` を書きたくなったら、向き（ドメイン起点）が誤っているサイン。
- 表示・転送の形（`camelCase` 等）はこの Response が持つ。
- **Command 操作の入出力ではない。** Command はドメイン（Entity）を操作する。Usecase の読み取り操作は QueryService の DTO を返してよい。

永続化モデルと転送メッセージは DTO と呼ばない。永続化モデルの配置・変換は [GATEWAY.md](./GATEWAY.md#read-model)、転送メッセージの配置と DTO の内包は [CONTROLLER.md](./CONTROLLER.md#protocolメッセージ型) に従う。

## DI への組み込み

ユースケースの構造体を trait で抽象化するかは判断する。

- **trait を切る**: 複数の実装が想定される、テストでモック差し替えしたい
- **構造体のまま**: 単一実装、シンプルな手続きで十分

迷ったら **trait を切らず構造体を直接持たせる**ことから始めて、必要が出たら trait 化する。

## エラー型

- ユースケース固有のエラーは `UsecaseError` として定義する
- ドメインエラーから変換する
- adaptor 層で `AppError` に集約される
