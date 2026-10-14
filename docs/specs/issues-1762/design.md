# Design

## 変える部分
- 右カラムの面の置き換え: 「Root の一覧」の面を「Node」の面に置き換え、上段に実行木、下段に選択した Node の詳細を縦に積む。根拠: R-001「右カラムに「Node」の面がある」、R-002、R-010。ルート: 委任
- 実行木の部品: 上段の実行木を SwiftUI の標準の部品で作る。根拠: R-002「開くと子の Node・Sequence・Fanout を実行木の形で字下げして並べる」、R-007。ルート: 固定（固定するルートを参照）
- 行の選択とタブを開く操作の分離: 行を押すと選択、`▷` でタブを開く。Sequence と Fanout の行は押すと開閉する。根拠: R-008、R-009。ルート: 委任
- 実行木の項目に出す値: 実行木の Node に、実行の ID、Node の実行の ID、Node 名、attempt、Artifact の有無、MODEL を出す。MODEL は、定義の Session の指定の model からサーバが導出する。根拠: R-003、R-004「model の指定が無い Session と、Command の行の MODEL は空欄である」、R-005。ルート: 委任
- Node の実行ごとの Artifact の購読: Node の実行の ID を引数にとり、その Node の実行の Artifact だけを返す購読を足す。下段はこの購読で Artifact を受け取る。根拠: R-011「同じ名前の Node の周回は、それぞれの Node の実行の Artifact を出す」、R-021。ルート: 固定（固定するルートを参照）
- 下段の詳細: 名前、種別、MODEL、失敗理由、FIELD / VALUE の表、JSON 表示の切り替え、Retry / Abort を出す。操作の可否はサーバの capability に従う。根拠: R-010、R-011、R-012、R-013。ルート: 委任
- 行の Approve: 実行木の行に、サーバの capability が Approve を可とするときだけ Approve ボタンを出し、`ApproveWorkspaceNode` を呼ぶ。根拠: R-022。ルート: 委任
- 面の上端と右クリックメニューの引き継ぎ: 今の「Root の一覧」の New Session / New Workflow、History、Root の行の Rename / Resume / Archive を Node 面へ移す。根拠: R-014。ルート: 委任
- History への実行の復元: History に、`workflowHistory` の archive 済みの実行を並べ、選ぶと `RestoreWorkspaceWorkflowExecution` を呼ぶ。根拠: R-015。ルート: 委任
- 右カラムと中央のカラムの幅: 右カラムの最大幅の制約を外し、中央のカラムに最小幅の定数を足す。根拠: R-016、R-017。ルート: 委任
- タブの種類 workflow の削除: Swift の `PaneTab.Kind`、proto の `PaneTabKind`、domain の `PaneTabKind`、保存形式から workflow を消す。根拠: R-018。ルート: 固定（固定するルートを参照）
- 保存済みの workflow タブの読み込み: 保存形式の読み込みで、種類 workflow のタブを捨て、残りのレイアウトを返す。根拠: R-019。ルート: 固定（固定するルートを参照）
- 過去の試行の配信: `node-detail` に過去の試行を足し、Command タブはそれを出す。画面で実行木の Root から過去の試行を探す処理は消す。根拠: R-020。ルート: 固定（固定するルートを参照）
- 回帰の E2E: 面の名前を探す文字列を「Node」に変え、タブを開く操作を行の `▷` に変える。Node 面に実行木と各 Node の状態が出て、行から Session を開ける回帰の E2E を足す。根拠: R-001、R-009。ルート: 委任

## 固定するルート
- 実行木は SwiftUI の標準の部品で作る。行の数は数十から百数十を想定する。
- Artifact は、Node の実行の ID を引数にとる新しい購読で、その Node の実行の 1 件だけを返す。既存の `workflow-execution` を購読して画面で選ぶ経路はとらない。実行全体を選択のたびに送ることになり、full-retention を避ける方針に反するためである。
- タブの種類 workflow は、proto と Swift だけでなく、サーバの domain と保存形式からも消す。保存済みのイベントに残る `"kind":"workflow"` のタブは、読み込みで捨てる。
- 過去の試行はサーバが `node-detail` で配る。画面で実行木をたどって探さない。

## 変えないもの
なし

## 未確定・リスク
- Sequence と Fanout が承認待ちになるかは、実装で確かめていない。定義の構文は 4 種類すべてに承認の要求を許す（`docs/glossary/WORKFLOW.md:476-479`）。承認待ちに入る経路を確かめたのは Session（`domain/workflow/entities/workflow_execution/mod.rs:2826-2838`）と Command（`adaptor/gateway/workflow/workflow_host.rs:1767-1800`）だけである。wire の Sequence と Fanout は Approve の可否を持たない（`proto/client.proto:1756-1774`）。Sequence と Fanout が承認待ちになる場合、R-022 を満たすには Sequence と Fanout の項目に可否を足す必要がある。
- SwiftUI の標準の部品で、数十から百数十行の実行木の開閉と選択が十分な速さで動くかは、確かめていない。着手時に macOS 15 で確かめる。遅い場合は R-002・R-007 を満たす部品の選び方を人と決め直す。
