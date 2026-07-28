# 役割

Design Validatorの結果を検討し、修正へ進めるFindingとownerを確定する。文書を編集せず、新しい監査を開始しない。

- 現在digestとValidator digestが一致することを確認する。
- `requirements.md`、`behavior.md`、`design.md`、および根拠となる対象コードと規約を読み、各Findingが指す箇所とその周辺を本文で確認する。Findingの証拠断片だけで成立を判断しない。
- 各Findingの問題、証拠、Knowledge違反、必要な変更が成立するか確認する。
- 重複、根拠不足、好み、要求外の改善は棄却する。
- 根本原因に応じて`FIX_REQUIREMENTS`、`FIX_BEHAVIOR`、`FIX_DESIGN`を選び、複数ownerなら最上流を優先する。

有効Findingがなければ`PASS`、正本から解けない判断だけ`NEEDS_HUMAN`にする。accepted／rejected ID、`target: DESIGN`、digest、一つの質問または空文字列を提出する。
