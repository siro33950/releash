{{ request }} のフルレビューで残った Open Thread を 1 件ずつ人間と確認し、実装前の修正方針を決定する。対応する Thread には `[FIX_POLICY_APPROVED]` 付きの方針 Comment を投稿し、対応見送り・誤検知・情報のみの Thread は方針と根拠を含めて resolve する。この node では実装しない。

## 入力

- タスク（任意の自由文。方針決定の補足指示があれば）: {{ request }}
- 全 Open Thread
- 各 Thread の本文、履歴、reviewer 指摘、verifier 判定、verifier 修正方針、既存方針 Comment
- `[FIX_POLICY_CHANGE_REQUEST]` 差し戻し Comment

## 出力

- 修正対応する Thread: 何をどう直すかが確定した `[FIX_POLICY_APPROVED]` Comment が投稿され、Open のまま残っている
- 対応見送り・誤検知・情報のみの Thread: 方針と根拠を含む resolve summary で resolve 済み
- 未承認、議論中、方針未決定の Thread が残っていない

## 正本ルール

- Thread 状態は `review history` の時系列を正本にする
- resolved Thread は対応済みとして扱い、追加の方針 Comment を投稿しない

## Thread 状態

| 状態 | 条件 | 操作 |
|---|---|---|
| `approved` | Open かつ最新の `[FIX_POLICY_CHANGE_REQUEST]` より後に `[FIX_POLICY_APPROVED]` がある | 何もしない |
| `change_requested` | Open かつ最新の `[FIX_POLICY_APPROVED]` より後に `[FIX_POLICY_CHANGE_REQUEST]` がある | 1 件ずつ人間と再議論する |
| `undecided` | Open かつ有効な `[FIX_POLICY_APPROVED]` がない | 1 件ずつ人間と方針を決める |
| `resolved` | Thread が resolved | 何もしない |

## Comment / Resolve フォーマット

修正対応する Thread に投稿する Comment:

```text
[FIX_POLICY_APPROVED]
処理区分: 修正対応
修正方針: <何をどう変更するか。実装 node がこの内容だけで着手できる粒度で書く>
受入条件: <修正後に満たすべき条件。確認観点・期待動作・テスト観点を含める>
根拠: <reviewer指摘・verifier判定・verifier修正方針・議論内容を踏まえた採用理由>
対応しない範囲: <この Thread では扱わないこと。なければ `なし`>
CHANGE_REQUESTへの回答: <該当する場合のみ。差し戻しを修正方針にどう反映したか>
```

対応見送り・誤検知・情報のみの Thread に使う resolve summary:

```text
処理区分: <対応見送り / 誤検知 / 情報のみ>
方針: <対応しない理由と扱い>
根拠: <reviewer指摘・verifier判定・verifier修正方針・議論内容を踏まえた理由>
対応しない範囲: <この Thread では扱わないこと。なければ `なし`>
```

## 手順

### 1. Thread を収集する

1. `releash review list --session-id "$RELEASH_SESSION_ID" --state open --json` で Open Thread を取得する
2. 各 Thread に対して `releash review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` と `releash review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json` を実行する
3. reviewer 指摘、verifier 判定、verifier 修正方針、既存 `[FIX_POLICY_APPROVED]`、`[FIX_POLICY_CHANGE_REQUEST]` を整理する

Open Thread が 0 件なら、完了報告を出して node を終える。

### 2. 未完了 Thread を選ぶ

Open Thread を履歴の時系列で分類し、`change_requested` または `undecided` の Thread だけを処理対象にする。

処理対象が複数ある場合でも、同時にまとめて承認を求めない。1 件を選び、その Thread の方針を確定または resolve してから次の Thread に進む。

### 3. 1 件ずつ人間に提示する

処理対象 Thread を 1 件だけ選び、次の形式で提示する。

```text
Thread <thread-id> [<観点>] <file>:<line-range>

reviewer指摘:
<reviewer の主張を要約>

verifier情報:
| verifier | 判定 | 修正方針 | 根拠 |
|---|---|---|---|
| <verifier名> | <判定> | <修正方針> | <根拠要約> |

既存方針:
<有効な既存方針があれば要約。なければ `なし`>

CHANGE_REQUEST:
<差し戻しがあれば問題・関連 Thread・決めてほしいことを要約。なければ `なし`>

CHANGE_REQUEST対応案:
<差し戻しを修正方針にどう反映するか / 該当なし>

処理区分案:
<修正対応 / 対応見送り / 誤検知 / 情報のみ>

修正方針案:
<修正対応の場合、何をどう変更するか。実装 node がこの内容だけで着手できる粒度で書く>

受入条件案:
<修正後に満たすべき条件。確認観点・期待動作・テスト観点を含める>

方針案の根拠:
<採用理由>

対応しない範囲:
<この Thread では扱わないこと。なければ `なし`>
```

### 4. 一問一答で合意する

提示した Thread について、人間に次のいずれかを求める。修正対応の場合、approve は「修正するかどうか」ではなく「この修正方針・受入条件で実装 node に渡してよいか」の承認として扱う。

- approve: 方針案を採用する
- 修正指示: 方針案を修正して再提示する
- 追加確認: `review get` / `review history` 等で確認し、根拠を補強して再提示する

approve されるまで、方針 Comment 投稿や resolve を行ってはならない。人間が疑問・反論・修正指示を出した場合は、その Thread について再提示し、同じ Thread の合意が終わるまで次の Thread に進まない。

### 5. 合意内容を反映する

approve された Thread だけ処理する。

- 修正対応なら、修正方針と受入条件を含む `[FIX_POLICY_APPROVED]` Comment を投稿し、Thread は Open のまま残す
- 対応見送り・誤検知・情報のみなら、方針 Comment を投稿せず、resolve summary に方針と根拠を含めて resolve する
- `[FIX_POLICY_CHANGE_REQUEST]` があった Thread は、新しい `[FIX_POLICY_APPROVED]` に差し戻しをどう反映したかを含める
- 関連 Thread の方針も同時に変える必要がある場合でも、この Thread の処理として勝手に他 Thread を変更しない。関連 Thread は次以降の一問一答で扱う

対応見送りの resolve 例:

```sh
releash review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome wontfix --summary "<対応見送りの理由>" --json
```

### 6. 次の Thread に進む

反映後に Open Thread を再取得し、未完了 Thread があれば手順 2 に戻る。

完了条件:

- 修正対応する Thread は、最新の `[FIX_POLICY_CHANGE_REQUEST]` より後に修正方針と受入条件を含む `[FIX_POLICY_APPROVED]` Comment がある
- 対応見送り・誤検知・情報のみの Thread は Open のまま残っていない
- 未承認、議論中、方針未決定の Thread が残っていない

### 7. 完了報告

完了時は次の形式で報告する。

```text
## 方針決定 完了

対象 Open Thread: <件数>件
承認済み方針: <件数>件
CHANGE_REQUEST対応: <件数>件
対応見送りとして resolve: <件数>件
未完了: 0件
```

## 禁止事項

- 実装に着手すること
- 複数 Thread をまとめて approve させること
- 1 件の Thread が未確定のまま次の Thread に進むこと
- approve されていない方針を `[FIX_POLICY_APPROVED]` として投稿すること
- `[FIX_POLICY_CHANGE_REQUEST]` を無視して既存方針のまま完了扱いにすること
- 修正対応する Thread をこの node で resolve すること
- 対応見送り・誤検知・情報のみの Thread をこの node で resolve せず Open のまま残すこと
- 対応見送り・誤検知・情報のみの Thread に `[FIX_POLICY_APPROVED]` Comment を投稿すること
- 合意済み方針と異なる内容を Comment または resolve summary に書くこと
