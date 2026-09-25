# ゲートウェイ層 規約

## 原則

- **gateway は変換する層である。** 外部世界の都合を内側の言語へ、内側の言語を外部世界の都合へ、相互に変換する。変換していない処理は gateway ではなく infrastructure に属する（[INFRASTRUCTURE.md](./INFRASTRUCTURE.md)）
- **gateway は層の名前である。** 実装するのは domain の trait（Repository、ドメインサービス）と usecase の trait（QueryService）である。「Gateway」という trait は存在しない
- **変換先は trait の所在で決まる。** domain の trait を実装するときはドメインの言語へ、QueryService を実装するときはフロントの言語（Output Data）へ変換する
- 外部ライブラリ（`git2`, `reqwest` 等）は gateway が直接呼んでも、infrastructure が提供する能力を使ってもよい。**どちらで呼ぶかは gateway と infrastructure を分ける基準ではない**（基準は変換しているかどうか）。ただし外部ライブラリの型・エラー・形式を gateway の外（domain / usecase / controller / presenter）へ漏らさない
- CQRS に従い、Command（書き込み）と Query（読み込み）を分離する
- **gateway は単一集約に対する純粋な I/O プリミティブを提供する**: 複数集約をまたぐオーケストレーションや操作の順序制御（業務手順）は usecase の責務であり、gateway に潰し込まない（[USECASE.md](./USECASE.md)）
- **gateway は業務の状態機械を持たない**: 業務の状態・ライフサイクルの表現主体は domain の集約である（[DOMAIN.md](./DOMAIN.md) モデルが実行を担う）。gateway が domain の状態を別の型で表現し直したり、domain 集約を経由せず自前の可変状態を進めたりしてはならない。gateway が状態を扱う場合は、domain の集約を保持して判断を委譲する（参照: `domain/terminal_surface/entities/terminal_surface_registry.rs` と `adaptor/gateway/terminal_surface/runtime_gateway_impl.rs`）
- **業務判断を gateway に沈めない**: 「マージ済みか」「削除してよいか」のような判定規則は、外部ライブラリ（git2 等）を使う位置にあっても domain のサービス・集約に置き、gateway はその入力となる生データの取得に徹する

## 出ていく側の横断的関心事

外部世界への呼び出しに掛ける期限・やり直し・同時実行の制限・計測は、common の包みを外部との接続に掛けて足す。trait の実装の中に直接書かない。一時的な失敗のやり直しは gateway の中で閉じ、内側へは結果だけを返す。

## QueryService の実装

**Output Data は domain の Entity ではない。** QueryService の実装は読み取り要求に応えて、Entity を経由せずデータソースから Output Data を直接組み立てて返す。Entity を生成する Repository を再利用して `Entity → Output Data` に詰め替えてはならない——向きが逆である（Output Data は要求起点であって Entity 起点ではない）。1:1 写像に見える場合も例外ではない（[USECASE.md](./USECASE.md) QueryService）。

Output Data か Entity かの判定は「**誰の都合でその形が決まっているか**」で行う。表示・転送（フロントの都合）のためにその形が必要なら Output Data であり、domain に置かない（[DOMAIN.md](./DOMAIN.md)「Entity か Output Data か」）。

永続化用モデル・外部 API のリクエスト / レスポンス型は、trait の実装の内側に閉じる。ドメイン型に外部システムの詳細を漏らさない。

## 失敗の変換

外部システムのエラーは trait の失敗へ変換する。業務の意味がある失敗は domain の失敗へ、それ以外は技術的な失敗の変種へ写す。共通の変換ロジックは `adaptor/gateway/shared/` に集約する。
