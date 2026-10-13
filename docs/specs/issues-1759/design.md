# Design

## 変える部分
- Terminal の持ち主: worktree の path に加えて、worktree の中で Terminal を区別する id を持てるようにする。持ち主の id を持たない今の形は React の画面のために残す。根拠: R-002「同じ worktree で Terminal タブを開くたびに、別の shell が起動する」。ルート: 「固定するルート」1
- タブと shell の結び付け: Swift.app の Terminal タブを、持ち主の id と結び付けて保存し、復元したタブを同じ id の shell に繋ぐ。根拠: R-009「そのタブは同じ shell に繋がり」。ルート: 「固定するルート」1
- Terminal タブの表示: SwiftTerm を依存に加え、Terminal タブの中身を SwiftTerm の端末表示にする。購読で受けたデータを `feed` し、入力を `WriteTerminalSurface` で送る。根拠: R-001「端末の表示には SwiftTerm を使う」、R-004。ルート: 「固定するルート」2
- 購読の手順: `GetOrSpawnTerminalSurface`、購読の開始、snapshot、output・resize・exit の順に扱う。snapshot は `replay` があれば記録時の cols・rows に合わせてから `feed` し、`replay` が空なら表示領域の実寸を `ResizeTerminalSurface` で送る。表示領域の大きさが変わったら `ResizeTerminalSurface` を送る。根拠: R-001、R-005。ルート: 委任
- 処理した量の報告: output の `data` の UTF-16 code unit 数を数え、snapshot の `processed_report_units` ちょうどの単位で `ReportTerminalProcessed` を送る。根拠: R-006。ルート: 委任
- 入力の送信: 入力に、開始済みの terminal の購読の ID を `attachment_id`、購読ごとに 0 から数える `sequence` を付けて送る。購読の開始より前の入力は貯めて、開始後に順に送る。根拠: R-004、R-007。ルート: 委任
- 再同期: 接続が回復したら terminal の購読を開始し直し、新しい snapshot で描き直し、報告の数と入力の `sequence` を 0 に戻す。根拠: R-008。ルート: 委任
- 帯: Terminal タブの上に、shell の今の作業ディレクトリと worktree の branch を表示する。branch は worktree の一覧の配信から取る。根拠: R-003。ルート: 委任
- 作業ディレクトリの取得と配信: サーバが shell の今の作業ディレクトリを求め、購読で配信する。根拠: R-003「作業ディレクトリの取得と配信はサーバが行う」。ルート: 「固定するルート」3
- shell 以外の処理の判定と配信: サーバが shell の前面で shell 以外の処理が動いているかを判定し、購読で配信する。根拠: R-010「動いているかの判定はサーバが行う」。ルート: 「固定するルート」4
- タブを閉じる操作: Terminal タブを閉じるとき、サーバが配信した判定で処理が動いていれば確認を出し、閉じると決まったら `KillTerminalSurface` で shell を止める。アプリの終了と再起動ではタブを閉じたことにせず、shell を止めない。根拠: R-009、R-010。ルート: 「固定するルート」4
- リンク・Clear・終了の表示: リンクを開く操作、Clear、終了したことと終了コードの表示を Terminal タブに置く。根拠: R-011。ルート: 委任
- アクセシビリティ: Terminal タブの画面の文字をアクセシビリティで公開する。根拠: R-012。ルート: 委任
- 起動コマンドの設定: サーバの設定に起動コマンドの項目を足し、新しく shell を起こすときにその値を読む。`GetOrSpawnTerminalSurface` の要求から `startup_command` を外す。根拠: R-013。ルート: 「固定するルート」5
- 起動コマンドの変更と購読: 起動コマンドを変える呼び出しと、今の値を読む購読を足す。React の設定画面の欄をそれに切り替え、React の localStorage の起動コマンドの項目と、`GetOrSpawnTerminalSurface` へ渡す処理を消す。根拠: R-014。ルート: 「固定するルート」5
- 回帰の E2E: `clients/macos/UITests/` に、Terminal タブを開き、打ったコマンドの出力が表示されることを確かめるテストを足す。根拠: R-017。ルート: 委任

## 固定するルート
1. Terminal の持ち主とタブ
   - サーバの Terminal の持ち主に、worktree の中の Terminal の id を加える。Swift.app は Terminal タブごとに別の id を使い、その id をタブと結び付けてサーバの UI 状態に保存する。
   - サーバ、proto、保存形式の変更はこの開発で行う。proto は互換のある足し方にし、`buf.yaml` の breaking 検査（WIRE_JSON）を通す。
2. 端末の表示
   - terminal emulator は SwiftTerm を使う。
3. 作業ディレクトリ
   - 取得と配信はサーバに置く。取得は、shell が作業ディレクトリを知らせる OSC 7 を第一にする。OSC 7 が届かない shell の扱いは実装で決める。
4. タブを閉じる
   - タブを閉じたら `KillTerminalSurface` で shell を止める。
   - shell の前面で shell 以外の処理が動いているときだけ、閉じる前に確認を出す。判定とその配信はサーバに置き、client では判定しない。
5. 起動コマンド
   - 値はサーバの設定に置き、変える呼び出しと読む購読を足す。React の設定画面はそれを使う。#1763 の Swift の設定画面も同じ呼び出しと購読を使う。
   - React の localStorage にある値は引き継がない。
   - `GetOrSpawnTerminalSurfaceRequest` から `startup_command` を外すときは、フィールドの番号 6 と名前 `startup_command` を reserved にする。消すだけでは `buf.yaml` の breaking 検査（WIRE_JSON）に通らないためである。

## 変えないもの
- React の Terminal: worktree ごとに 1 つの Terminal を、持ち主の id を持たない今の形で使う。表示を外しても shell を止めない今の振る舞いも変えない。

## 未確定・リスク
- OSC 7 を出さない shell では、作業ディレクトリが変わったことをサーバが知る手段が実装の判断に残る。手段によっては、移った後の作業ディレクトリを帯に出せない場合があり、R-003 を満たせない。
