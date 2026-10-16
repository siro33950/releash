# Design

## 変える部分

- タブの同定: 開く経路と保存・復元で Session タブの対象を AgentSession の ID に統一する。根拠: R-001「AgentSession の ID だけで対象を同定する」、B-001「既存の同じ Session タブが選択される」、B-002「保存前と同じ AgentSession を対象とする」。ルート: AgentSession の ID を使う。具体的な変更方法は委任。
- タブの購読: AgentSession の ID だけでタブに必要な情報を購読できるようにする。根拠: R-002「その AgentSession の ID だけを指定して購読できる」、B-003「名前、状態、操作可否を受け取れる」。ルート: 同定に使う AgentSession の ID だけを購読の指定に使う。具体的な変更方法は委任。
- タブ名: Session の購読で受け取った名前をタブ名に反映する。根拠: R-003「購読した Session 名をタブ名に反映し、改名後も追従する」、B-004「タブ名は届いた Session 名になる」。ルート: 委任。
- 状態色: Session タブのアイコンに一覧と同じ状態の色を反映できるようにする。根拠: R-004「アイコンの黄・青・緑で表示し、状態の変化に追従する」、B-005「一覧と同じ分類の色になる」。ルート: Bonsplit 側でアイコンの色を指定できるようにする。状態ごとの SF Symbol の差し替えは行わない。
- Rename の入口: Session タブのタブ部分から Rename を実行できるようにする。根拠: R-005「タブ部分から Rename を実行でき」、B-006「タブ名と一覧の Session 名に変更が反映される」。ルート: 委任。
- Archive の入口: 単独 Session のタブ部分から既存の確認と Archive を実行できるようにする。根拠: R-006「単独 Session のタブ部分から Archive を実行できる」「Workflow 内の Session タブからは Archive できない」、B-007「確認が表示される」、B-008「実行は中止され、Archive される」、B-009「実行の中止と Archive は行われない」、B-010「確認なしで Archive される」、B-011「そのタブから Archive を実行できない」。ルート: 委任。

## 固定するルート

- Session タブの同定と購読の指定には AgentSession の ID を使う。
- 状態の表示には Bonsplit 側のアイコン色指定を追加して使う。状態ごとに SF Symbol を差し替えない。

## 変えないもの

Workflow 内の Session の Archive を許可する変更は行わない。今回のタブへの操作追加は、その許可を拡張するものではない。

## 未確定・リスク

なし
