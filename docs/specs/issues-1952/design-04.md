# Design 04

## 開始状態

基準は branch `feat/issues/1952` の `0711664b`。直前の Design は `design-03.md`。この周までに、`design-03.md` が扱った 2 件（`74954dce`・`64594197`）が閉じた。

## 変える部分

- `ErrorCode::Aborted` の分類を元へ戻す: `desktop_client.rs:378-385` の Transient の枝から `Aborted` を外し、`_ => TechnicalFailureNature::Other` に落ちる形に戻す。根拠: Thread `e4a5d8fd`。ルート: 下の「固定するルート」のとおり
- 削ったテストを戻す: `test_生存確認_connectの失敗を技術的な失敗の性質へ写す` から消えた `(ErrorCode::Unauthenticated, TechnicalFailureNature::Other)` のケースと、message が元の診断を保つことの検証を戻す。根拠: Thread `e4a5d8fd`。ルート: 分類をどうするかに関わらず戻す
- 重複した `Ready` の検証を独立したテストに分ける: `test_設定受信_生存確認と同じstreamで受け取る` を `Ready` 1 回の形に戻し、重複した `Ready` で購読の開始要求が増えないことは `test_購読開始_*` の系列に独立したテストとして置く。根拠: Thread `6f521621`。ルート: 委任

## 固定するルート

- 転送のステータスコードから技術的な失敗の性質への写し方は、この開発では変えない。ABORTED は gRPC では並行の変更との衝突を示すコードで、読み直してからやり直すものであり、そのまま繰り返せば通る UNAVAILABLE とは違う。daemon も競合のときだけ返している（`adaptor/presenter/connect.rs:115,138,184,208,224,259,630` ほか）。proto の `reconnect_status_code` に ABORTED が含まれることは、分類の根拠にしない。
- `design-01.md`〜`design-03.md` で固定したルートを維持する。

## 変えないもの

- `requirements.md` と `behavior.md`。今周の変更は、要求に根拠の無い変更を戻すことと、テストの分け方であり、要求と受入条件の追加・変更を伴わないため。

## 未確定・リスク

なし。
