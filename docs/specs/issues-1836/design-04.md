# Design 04

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)` で、作業 branch `feat/issues/1836` の派生点も同じ commit である。直前のDesignは `docs/specs/issues-1836/design-03.md` であり、その実装は未コミット差分として存在する。

Design 03 で扱ったThread `4b868aa7-ff46-4849-8464-333a6f6ca5eb` と `4350362b-6edd-4e6e-93e5-7210b7245b83` は解消済みである。今周の開始時点では、`[FIX_POLICY]` 付きopen Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87` と `59fc794d-3e38-4aec-b3f6-7c7f2eb30057` が未解消である。見送りとなったThreadはない。

## 変える部分

- 終端事実を持つ実行の復元経路の単一化: 終端事実の有無で通常foldと終端専用replayを切り替える二重実装を解消し、保存定義本文を解釈せず、Abort・合成Node・dynamic fanoutを含む公開状態を通常foldと同じ結果で復元する。根拠: R-002、B-002、Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87`。ルート: 委任
- 起動時Abort手順の所有循環の解消: 保存定義を解釈できない未完了実行のAbort要否と遷移をdomainが決め、起動時の読み取り・理由付きAbort factの永続化・後続reconciliationの手順をusecaseが所有し、gateway hostがusecaseを所有して同じhostへ再呼出しする循環をなくす。根拠: R-003、B-003、Thread `59fc794d-3e38-4aec-b3f6-7c7f2eb30057`。ルート: 委任

## 固定するルート

固定する実装上の指定なし

## 変えないもの

なし

## 未確定・リスク

なし
