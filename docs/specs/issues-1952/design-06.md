# Design 06

## 開始状態

基準は branch `feat/issues/1952` の `cb9dae72`。直前の Design は `design-05.md`。この周までに、`design-05.md` が扱った 2 件（`6f521621`・`af394f00`）が閉じた。加えてこの周で立った 4 件（`f243d803`・`203495f4`・`40e56d82`・`b8e28f4f`）を、`a2b9c3b2` と同じ問題を指すものとして `duplicate` で閉じた。

## 変える部分

- `before_bookmark_deadline` を消す: `desktop_client.rs:366-375` の関数を消し、`observe()` の受信を `487765a1` と同じ `tokio::time::timeout_at(bookmark_deadline, stream.message())` に戻す。`match` の腕も元の形に戻す。テスト `test_生存確認_bookmark期限後は即時受信可能な変更も無音と扱う` を消す。根拠: Thread `a2b9c3b2`。ルート: 下の「固定するルート」のとおり
- 残るテストの Given / When / Then の区切りを直す: `test_購読開始_重複したreadyで開始要求が増えない` の `// Given` から検証を外し、`// Then` にまとめる。根拠: Thread `6dfa36f7`。ルート: 委任

## 固定するルート

- シェルの無音の判定は `tokio::time::timeout_at` による受信の打ち切りだけにする。受信と独立した厳密な期限の判定は置かない。合図が届かないままイベントだけが連続する状態は、daemon が R-001 に反した場合にだけ起きる。R-001 は daemon 側の要求であり、daemon 側のテストで確かめる。その状態でもイベントが届いている以上 daemon は生きているため、生存の判定としての害は無い。
- `design-01.md`〜`design-05.md` で固定したルートを維持する。

## 変えないもの

- `requirements.md` と `behavior.md`。今周の変更は、要求に根拠の無い変更を戻すことと、テストの区切りであり、要求と受入条件の追加・変更を伴わないため。

## 未確定・リスク

なし。
