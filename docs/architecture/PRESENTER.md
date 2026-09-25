# プレゼンター層 規約

## 原則

- **Usecase の Output Boundary を実装する。** Usecase が出した Output Data を、転送の形（Connect のメッセージ、HTTP local API のレスポンス）に変える
- **変換だけを持つ。** 業務の判断を書かない。受け取った Output Data を解釈して結果を変えない
- 画面への状態の配信（購読）も、Output Boundary を通して届いたものを転送の形に変えて送る。送る仕組みそのもの（接続、送り待ち、送る量の制御）は infrastructure と common の包みを使う

## 転送のメッセージ型

`proto/client.proto` のメッセージ型と、複数の入口で共有する転送の型は presenter に置く。これらは転送の形であって、ドメイン型でも Output Data でもない。読み取り結果を返す場合は、Output Data をこの型に載せる。

## 失敗

- Usecase が出した失敗を、転送の失敗（Connect のステータスコード、HTTP status）へ対応付けるのは presenter の 1 か所だけである。業務の失敗と技術的な失敗を区別して対応付ける
- 転送の失敗をその外で組み立てない。失敗を文字列にして転送の失敗にしない
