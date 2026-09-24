# コントローラ層 規約

## 原則

- **外部入力の受け口**として薄く保つ
- 引数のシリアライズ／デシリアライズと型変換のみ
- 業務ロジックを書かない（Usecase を呼ぶだけ。QueryService や Repository を controller から直接呼ばない）
- **受理判定を controller で書かない**: 「この状態でこの操作を受理してよいか」の判断は domain の集約が答える（[DOMAIN.md](./DOMAIN.md) モデルが実行を担う）。controller が状態型を独自解釈してゲートを設けると、同じ判断が層をまたいで二重化し、domain 側の不変条件が効かなくなる
- 3系統の入口を分離する：
  - `controller/api/client*.rs` と `controller/client/` — Connect の ClientService。画面からの呼び出しと購読の入口。契約は `proto/client.proto`
  - `controller/api/` の HTTP local API — CLI と hook の入口
  - `controller/command/` — Tauri コマンド（`#[tauri::command]`）。desktop 固有の操作（ウィンドウ、daemon の起動と監視、更新、接続先の受け渡し）だけの入口

Connect と HTTP local API は同じ Usecase を呼ぶ。入口が増えても業務手順は複製しない。

## AppState（DI 受け皿）

- 各 Usecase を `Arc<T>` または `Arc<dyn Trait>` で保持する
- **QueryService は AppState に直接持たせない。** 読み取りクエリサービスは各 Usecase が内部に保持する協力者であり、composition root で Usecase に注入する。controller は QueryService を保持・直呼びしない
- 組み立て（composition root）は controller の責務とし、gateway や任意のエントリポイントへ配線責務を漏らさない

## Connect（ClientService）

ドメインごとに `controller/client/<domain>/` に登録関数を用意し、`client/dispatch.rs` がそれらをまとめる。composition root にコマンド名を列挙しない。

画面はこの入口を React（`src/lib/client.ts`）から直接呼ぶ。Tauri コマンドを経由した中継を作らない。

## Tauri コマンド

desktop 固有の操作だけを置く。サーバの業務操作や、Connect の呼び出し・購読の中継を Tauri コマンドに追加しない。ドメインごとに登録関数を用意し、`command/mod.rs` がそれらをまとめる。

## local API

Connect と同じ Usecase を呼ぶ薄い入口であり、業務ロジックを持たない点も同じ。

- ドメインごとに router を定義し、`api/mod.rs` で合成する
- 認証は `api/mod.rs` が router 全体へまとめて掛ける。個々のハンドラに認証を書かない
- リクエスト／レスポンス型は `api/protocol.rs` に置く。戻り値のエラーは `ApiError` で統一する

## protocol/（メッセージ型）

`proto/client.proto` のメッセージ型との変換（`adaptor/protocol/client/`）や、複数の入口で共有する view 型は `adaptor/protocol/` に配置する。local API だけで使うリクエスト／レスポンス型は `controller/api/protocol.rs` に置き、`adaptor/protocol/` へは上げない。

これらは外部入口のメッセージ型であって、ドメイン型でも DTO でもない。DTO は QueryService の Response（[USECASE.md](./USECASE.md)）を指し、別物である。読み取り結果を返す場合は、usecase の DTO をこのメッセージ型に内包して載せる。

## エラーハンドリング

- `controller/client/` の処理の戻り値は `Result<_, AppError>` で統一する。`AppError` は失敗の分類を持つ
- Connect のエラーへの変換は `adaptor/protocol/connect.rs` の 1 か所だけで行い、その入口は分類を持つエラーだけを受け取る。`ConnectError` をその外で組み立てない。失敗を文字列にして Connect のエラーにしない
- 詳細は [USECASE.md](./USECASE.md) と `other/error.rs` を参照
