# コントローラ層 規約

## 原則

- **外部入力の受け口**として薄く保つ
- 転送の形を Input Data に変えて Usecase を呼ぶことのみ
- 業務ロジックを書かない（Usecase を呼ぶだけ。QueryService や Repository を controller から直接呼ばない）
- **受理判定を controller で書かない**: 「この状態でこの操作を受理してよいか」の判断は domain の集約が答える（[DOMAIN.md](./DOMAIN.md) モデルが実行を担う）。controller が状態型を独自解釈してゲートを設けると、同じ判断が層をまたいで二重化し、domain 側の不変条件が効かなくなる
- 3系統の入口を分離する：
  - `controller/api/client*.rs` と `controller/client/` — Connect の ClientService。画面からの呼び出しと購読の入口。契約は `proto/client.proto`
  - `controller/api/` の HTTP local API — CLI と hook の入口
  - `controller/command/` — Tauri コマンド（`#[tauri::command]`）。desktop 固有の操作（ウィンドウ、daemon の起動と監視、更新、接続先の受け渡し）だけの入口

Connect と HTTP local API は同じ Usecase を呼ぶ。入口が増えても業務手順は複製しない。

## AppState（DI 受け皿）

- 各 Usecase を `Arc<T>` または `Arc<dyn Trait>` で保持する
- **QueryService は AppState に直接持たせない。** 読み取りクエリサービスは各 Usecase が内部に保持する協力者であり、Main が Usecase に注入する。controller は QueryService を保持・直呼びしない
- AppState の組み立て（composition root）は Main の責務である。controller は組み立て済みのものを受け取る

## 受け手の側の横断的関心事

期限・取り消し・同時実行の制限・優先度・ログと計測は、common の包みを入口の手前に掛けて足す。handler の中に書かない。

## Connect（ClientService）

ドメインごとに `controller/client/<domain>/` に登録関数を用意し、`client/dispatch.rs` がそれらをまとめる。Main にコマンド名を列挙しない。

画面はこの入口を React（`src/lib/client.ts`）から直接呼ぶ。Tauri コマンドを経由した中継を作らない。

## Tauri コマンド

desktop 固有の操作だけを置く。サーバの業務操作や、Connect の呼び出し・購読の中継を Tauri コマンドに追加しない。ドメインごとに登録関数を用意し、`command/mod.rs` がそれらをまとめる。

## local API

Connect と同じ Usecase を呼ぶ薄い入口であり、業務ロジックを持たない点も同じ。

- ドメインごとに router を定義し、`api/mod.rs` で合成する
- 認証は `api/mod.rs` が router 全体へまとめて掛ける。個々のハンドラに認証を書かない
- リクエスト型は `api/protocol.rs` に置く。レスポンスへの変換（`ApiError` を含む）は presenter が行う（[PRESENTER.md](./PRESENTER.md)）

## 失敗

- 入力の形の誤りは、Usecase を呼ぶ前に controller が返す
- Usecase の結果（失敗を含む）を転送の形とステータスコードに変えるのは presenter である。controller は変えない
