# ゲートウェイ層 規約

## 原則

- **gateway は変換する層である。** 外部世界の都合を内側の言語へ、内側の言語を外部世界の都合へ、相互に変換する。変換していない処理は gateway ではなく infrastructure に属する（[INFRASTRUCTURE.md](./INFRASTRUCTURE.md)）
- **変換先は port の所在で決まる。** domain 層の port（Repository / Gateway trait）を実装するときはドメインの言語へ、usecase 層の port（QueryService）を実装するときはフロントの言語（DTO）へ変換する
- **ドメインの trait の具体実装**を提供する
- 外部ライブラリ（`git2`, `reqwest` 等）は gateway が直接呼んでも、infrastructure が提供する能力を使ってもよい。**どちらで呼ぶかは gateway と infrastructure を分ける基準ではない**（基準は変換しているかどうか）。ただし外部ライブラリの型・エラー・形式を port の外側（domain / usecase / controller）へ漏らさない
- CQRS に従い、Command（書き込み）と Query（読み込み）を分離する
- **gateway は単一集約に対する純粋な I/O プリミティブを提供する**: 複数集約をまたぐオーケストレーションや操作の順序制御（業務手順）は usecase の責務であり、gateway に潰し込まない（[USECASE.md](./USECASE.md)）
- **gateway は状態機械を持たない**: 状態・ライフサイクルの表現主体は domain の集約である（[DOMAIN.md](./DOMAIN.md) モデルが実行を担う）。gateway が domain の状態を別の型で表現し直したり、domain 集約を経由せず自前の可変状態を進めたりしてはならない。gateway が状態を扱う場合は、domain の集約を保持して判断を委譲する（参照: `domain/terminal_surface/entities/terminal_surface_registry.rs` と `adaptor/gateway/terminal_surface/runtime_gateway_impl.rs`）
- **業務判断を gateway に沈めない**: 「マージ済みか」「削除してよいか」のような判定規則は、外部ライブラリ（git2 等）を使う位置にあっても domain のサービス・集約に置き、gateway はその入力となる生データの取得に徹する

## 外向き通知の送信

**サーバ → クライアントの通知もこのレイヤーで扱う**：

- 送信実装（コネクション管理、シリアライズ、送信）は `infrastructure/` に置く
- ドメイン側は `domain/<context>/gateway.rs` で送信用 trait を定義する
- Gateway 実装は `adaptor/gateway/<context>/` で trait を実装し、infrastructure の送信実装を呼ぶ
- 送信経路（Tauri event、local API、その他）の選択は Gateway 実装の内側に閉じる。ドメインと usecase は経路を知らない

## read model

**read model は domain の Entity ではない。** Query 側は読み取り要求に応えて、Entity を経由せずデータソースから read model を直接組み立てて返す。Entity を生成する Repository を再利用して `Entity → DTO` に詰め替えてはならない——向きが逆である（read model は要求起点であって Entity 起点ではない）。1:1 写像に見える場合も例外ではない（[USECASE.md](./USECASE.md) QueryService）。

read model か Entity かの判定は「**誰の都合でその形が決まっているか**」で行う。表示・転送（フロントの都合）のためにその形が必要なら read model であり、domain に置かない（[DOMAIN.md](./DOMAIN.md)「Entity か DTO か」）。

永続化用モデル・外部 API のリクエスト / レスポンス型も同様に、変換とともにこのレイヤーに置く。ドメイン型に外部システムの詳細を漏らさない。

## エラー変換

外部システムのエラーはドメインエラーに変換する。共通の変換ロジックは `adaptor/gateway/shared/` に集約する。
