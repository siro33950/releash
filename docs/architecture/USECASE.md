# ユースケース層 規約

## 原則

- **アプリケーション固有の業務手順**を表現する
- 外部世界へのアクセスは domain の trait（Repository、ドメインサービス）または usecase の QueryService の trait を介する。購読の配信は usecase の配信の口の trait を介して出す。具体実装は知らない
- 外部世界との接続を担う依存禁止（`tauri`, `git2`, `reqwest` 等を直接 `use` しない）。Usecase自身が所有する非同期排他・通知・タスク協調には`tokio::sync`等の実行制御primitiveを使用してよいが、I/O、時刻、process、transport、永続化をUsecaseへ持ち込む理由にはならない
- CQRS に従い、Command の業務手順は Usecase、Query の読み取り処理は QueryService に分離する。両者は別ファイルに置き、controller からの入口は読み取りも含めて Usecase に統一する
- **QueryService は Usecase ではない。** Usecase はアプリケーション固有の業務手順（オーケストレーション）を表現する唯一の単位であり、QueryService は読み取りクエリのサービスにすぎない。「ユースケース」と呼んでよいのは Usecase のみ。QueryService を「Query 側ユースケース」等と呼んで usecase 扱いしない
- **usecase は業務の状態機械を持たない**: 業務の状態・ライフサイクルの表現主体は domain の集約である（[DOMAIN.md](./DOMAIN.md) モデルが実行を担う）。usecase が持つのは「何を、どの順で呼ぶか」であり、「この状態でこの操作を受理してよいか」は domain に問う。domain の状態型を usecase で再定義しない
- **横断的関心事は Usecase の中に書かない**: ログ・計測・検証などは、common の包みを Usecase の入口に掛けて足す
- **usecase が肥大化したら domain の欠落を疑う**: 手順ではなく業務の判断（受理可否・遷移・分類・検証）が usecase に溜まっているなら、それは domain 集約またはドメインサービスに引き上げるべきものである。特に、対応する `domain/<name>/` が存在しないまま usecase に状態と判断が集まっている場合は、domain 境界の欠落を意味する

> **CQRS は「Command/Query のサービス分離」であって、「Repository を read 用 / write 用の trait に分割すること」ではない。** Repository は読み書きを問わず Entity を生成・取得する単位であり、read メソッドを持つこと自体は CQRS 違反ではない。Query 専用のテストダブルが未使用の write メソッドを実装させられる程度のことは、trait 分割の理由にならない。

## Usecase

読み取り専用の操作と、書き込み・状態変更を伴う操作を提供するアプリケーション操作の入口。Repository・ドメインサービス・QueryService を組み合わせて業務手順を実行する。アプリケーション層で唯一「ユースケース」と呼べる単位であり、読み取りと書き込みを跨ぐオーケストレーション（例: 一覧取得後にそのタイミングで GC を実行する等）もここに集約する。Usecase は Entity でも DTO でも返してよい。読み取り専用の操作では QueryService に委譲し、その DTO を返してよい。QueryService 等の読み取り部品は Usecase から呼ぶ協力者であって、Usecase ではない。

**複数の集約・Repository をまたぐオーケストレーションは usecase の業務手順である。** 操作の順序制御も usecase が持つ。例: 「ブランチ削除前に、紐づく worktree を先に削除する」——これは git の機構的制約（checkout 中ブランチは削除不可）に由来する順序だが、複数集約をまたぐ手順なので usecase の責務とする。gateway は単一集約に対する純粋な I/O プリミティブに分解し、業務手順を gateway に潰し込まない。usecase が肥大化した場合は domain サービスの導入を検討する（[DOMAIN.md](./DOMAIN.md) ドメインサービス）。

## 購読の配信の口

購読で、Usecase が状態の変化を外へ押し出す口（`StateSubscriptionOutput` などの「…Output」）。usecase が trait として定義し、adaptor/presenter が実装する。

- 要求に応える操作は戻り値で返し、この口を使わない。
- trait の引数に転送の形を持ち込まない。

## QueryService（Query 側）

読み込み専用のクエリサービス。**Usecase ではない**（「Query 側ユースケース」ではない）。trait を usecase に置き、adaptor/gateway が実装する。DTO を返す。

**QueryService の trait が usecase 層にあるのは、ドメインの言語をスキップするためである。** DTO は読み取り要求の出力仕様であり、その言語はフロント側の都合で決まる。domain の trait がドメインの言語で書かれる（[DOMAIN.md](./DOMAIN.md)）のに対し、QueryService の trait はフロントの言語で書かれる。trait の置き場所が、その trait の話す言語を決めている。

**Query 側は読み取り要求に応えて、データソースから DTO を直接組み立てて返す。** DTO は読み取り要求の出力仕様であり、その形は要求の都合で決まる。Entity を生成する Repository を再利用して `Entity → DTO` に詰め替えてはならない——向きが逆である（DTO は要求起点であって Entity 起点ではない）。1:1 写像に見える場合も例外ではない。

集約・表示集計でも同じである。Entity を構築して詰め替えるのではなく、QueryService の実装がデータソースから DTO を直接組み立てて返す。DTO は domain の Entity ではない（[DOMAIN.md](./DOMAIN.md)「Entity か DTO か」）。

## DTO

- DTO は **QueryService（Query 側）が返す Response** である。読み取り要求（ユースケース／画面）の出力仕様であり、その形は要求の都合だけで決まる。
- ドメイン（Entity）から導かれない。QueryService がデータソースから直接組み立てるものであり、**「ドメイン型 ↔ DTO の変換」という工程は存在しない**。`From<Entity> for Dto` を書きたくなったら、向き（ドメイン起点）が誤っているサイン。
- 表示・転送の形（`camelCase` 等）はこの Response が持つ。
- **Command 操作の入出力ではない。** Command はドメイン（Entity）を操作する。Usecase の読み取り操作は QueryService の DTO を返してよい。

永続化モデルと転送メッセージは DTO と呼ばない。永続化モデルは [GATEWAY.md](./GATEWAY.md) の実装の内側に、転送メッセージとその DTO の内包は [PRESENTER.md](./PRESENTER.md) に従う。

## DI への組み込み

ユースケースの構造体を trait で抽象化するかは判断する。

- **trait を切る**: 複数の実装が想定される、テストでモック差し替えしたい
- **構造体のまま**: 単一実装、シンプルな手続きで十分

迷ったら **trait を切らず構造体を直接持たせる**ことから始めて、必要が出たら trait 化する。

## 失敗

usecase は独自のエラーの部品を持たない。失敗は戻り値で返し、業務の失敗と技術的な失敗を区別する。

- 業務の手順をやり直すか（集約の版が競合したら読み直して再実行する等）は Usecase が決める。判断に使うのは domain の失敗が持つ業務の意味である
- 技術的な失敗は、中身で分類し直さずにそのまま出す。失敗を文字列へ変換して落とさない
