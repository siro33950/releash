# Design 02

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)` で、作業 branch `feat/issues/1836` の派生点も同じ commit である。直前のDesignは `docs/specs/issues-1836/design-01.md` であり、その実装は未コミット差分として存在する。

今周の開始時点では、Design 01 の実装に対する `[FIX_POLICY]` 付きopen Thread `ed57617f-29bd-423d-8629-9ceee2e7a5a1`、`535deb65-31ee-45c2-a9e6-16f2ef51dcaa`、`90b1c19a-4d27-4fb9-a175-4da7a9659a87`、`59fc794d-3e38-4aec-b3f6-7c7f2eb30057` が未解消である。この周までに解消・見送りとなったThreadはない。

## 変える部分

- 到達不能な `failure` 状態分類の削除: `Unresolved` の削除後に生成されなくなった `failure` を公開protocol、変換、生成型、TypeScript型、UI、acceptance型から取り除き、protoの番号2と名前 `failure` はreservedとして保持する。根拠: R-005、Thread `ed57617f-29bd-423d-8629-9ceee2e7a5a1`。ルート: 委任
- workflow factの永続化形式のレイヤー分離: domainは純粋なworkflow factと終端判定を所有し、DBのevent_type値およびdetail JSONのencode/decodeはgatewayの永続化境界が所有するようにする。根拠: Thread `535deb65-31ee-45c2-a9e6-16f2ef51dcaa`。ルート: 委任
- 終端事実を持つ実行の定義非依存な復元: 完了事実を持つ実行を、保存定義本文の解釈成否に依存しない単一の復元経路で `Completed` と判定し、部分的なWorkflowDefinitionと簡易replayの代替経路を生成しない。根拠: R-002、B-002、Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87`。ルート: 委任
- 解釈不能な未完了実行の起動時Abortのレイヤー是正: Abortの要否と遷移はdomainが決定し、起動時の読み取りと理由付きAbort factの永続化はusecaseが調停し、gatewayはdecodeとappendに限定する。根拠: R-003、B-003、Thread `59fc794d-3e38-4aec-b3f6-7c7f2eb30057`。ルート: 委任

## 固定するルート

固定する実装上の指定なし

## 変えないもの

なし

## 未確定・リスク

なし
