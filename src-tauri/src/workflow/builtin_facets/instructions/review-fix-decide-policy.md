{{project_name}} のフルレビューで残った全 Open Thread に対し、**各 Thread の修正方針を決定する**。対応する方針の Thread には方針 Comment を投稿し、対応見送り方針の Thread は resolve 理由に方針と根拠を含めて本 Step で resolve する。実装は次の Step（implement）で行うため、本 Step では実装に着手しない。

## 入力

- タスク（任意の自由文。方針決定の補足指示があれば）: {{task}}
- 全 Open Thread（reviewer 指摘 + verifier判定 + verifier修正方針 + 既存方針Comment）

## 出力

- 対応する方針の Thread に方針 Comment（方針＋根拠）が投稿された状態
- 対応見送り方針の Thread が resolve 理由（方針＋根拠）付きで resolve された状態
- 全 Thread に投稿し終えたら approve で次 Step（implement）へ進む

## プロセス

### 1. 対象 Thread の収集

Open 状態の全 Thread を取得し、次の項目で一覧化する。

| thread_id | file | line | reviewer指摘 | verifier判定 | verifier修正方針 | 既存方針Comment |
|---|---|---|---|---|---|---|

各列の内容:
- `thread_id`: Thread ID
- `file`: 指摘対象ファイル
- `line`: 指摘対象行または範囲
- `reviewer指摘`: reviewer の主張を1文で要約
- `verifier判定`: verifier / classifier の判定があれば記載、なければ `なし`
- `verifier修正方針`: verifier が提案している対応内容。対応要否だけでなく、何をどう扱うべきとしているかを書く。なければ `なし`
- `既存方針Comment`: 既に方針 Comment があれば要約、なければ `なし`

この一覧には、取得できた Open Thread を必ず全件含める。
この一覧に載っていない Thread は、以降の分類・方針案提示・議論・投稿の対象にしてはならない。

### 2. Open Thread の分類

Open Thread を次の3つに分類する。

#### 2-1. 除外グループ

既に明確な対応する方針 Comment があり、実装 Step が判断できる Thread。

このグループは、以降の方針案提示・議論・投稿の対象にしない。

既存方針 Comment が対応見送りを示している Open Thread は、方針案提示・議論の対象にしない。
既存方針 Comment の内容を resolve summary に使い、この Step で resolve する。

#### 2-2. 一致グループ

方針 Comment が未投稿、または既存方針 Comment が再検討を要する Thread のうち、次を満たすもの。

- verifier判定が一致している
- verifier修正方針も実質的に一致している
- 方針 Comment を同じ判断軸で書ける

#### 2-3. 割れグループ

方針 Comment が未投稿、または既存方針 Comment が再検討を要する Thread のうち、次のいずれかに当てはまるもの。

- verifier判定が割れている
- verifier判定は一致しているが、verifier修正方針が異なる
- verifier修正方針が抽象的で、そのまま採用できない
- reviewer指摘と verifier修正方針の対応関係が不明
- 既存方針 Comment が曖昧・不足・矛盾している

### 3. 一致グループの一括処理

#### 3-1. 方針案の提示

一致グループの全 Thread を、方針案として一覧で提示する。
この時点では方針 Comment を投稿しない。

```text
## 一致グループ方針案 (<件数>件)

### Thread <thread-id> [<観点>] <file>:<line-range>
- reviewer指摘: <reviewer の主張を1文で要約>
- verifier判定: <一致した判定>
- verifier修正方針: <一致した修正方針>
- 採用する方針: <この Thread の扱い方>
- 採用理由: <reviewer指摘・verifier判定・verifier修正方針を踏まえた理由>
- 対応しない範囲: <この Thread では扱わないこと。なければ `なし`>

### Thread <thread-id> ...
```

#### 3-2. 合意確認

提示した方針案について、人間に次のいずれかを求める。

- 一括 approve
- Thread 単位の reject
- 方針案の修正指示

#### 3-3. 合意後の反映

一括 approve された Thread には、提示した方針案どおりに処理する。

方針が対応する場合は、方針 Comment を投稿し、この Step では resolve しない。

```text
方針：<採用する方針>
根拠：<採用理由>
対応しない範囲：<対応しない範囲>
```

方針が対応見送りの場合は、方針 Comment を投稿せず、この Step で Thread を resolve する。

```sh
{{path_alias.releash}} review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome wontfix --summary "<対応見送りの理由>" --json
```

resolve summary には、採用した方針、採用理由、対応しない範囲を含める。

Thread 単位で reject されたもの、または修正指示が出たものには投稿しない。
それらの Thread は、一致グループから外して個別検討に回す。

approve されていない Thread に方針 Comment 投稿や resolve を行ってはならない。

### 4. 割れグループの逐次処理

#### 4-1. 方針案の提示

割れグループの Thread は、1件ずつ方針案を提示する。
この時点では方針 Comment を投稿しない。

```text
## 割れ Thread 方針案 <thread-id> [<観点>] <file>:<line-range>

reviewer指摘:
<reviewer の主張を要約>

verifier比較:
| verifier | 指摘解釈 | 判定 | 影響範囲 | 修正方針 | 対応範囲 | 根拠 |
|---|---|---|---|---|---|---|
| <verifier名> | <解釈> | <判定> | <影響範囲> | <修正方針> | <対応範囲> | <根拠要約> |
| <verifier名> | <解釈> | <判定> | <影響範囲> | <修正方針> | <対応範囲> | <根拠要約> |

割れている点:
- <判定 / 指摘解釈 / 影響範囲 / 修正方針 / 対応範囲 / 根拠のうち、割れている内容>

一致している点:
- <一致している内容。なければ `なし`>

方針案:
<どの解釈・判定・修正方針を採用するか>

方針案の根拠:
<その方針案を提案する理由>

対応しない範囲:
<この Thread では扱わないこと。なければ `なし`>
```

#### 4-2. 合意確認と議論

提示した方針案について、人間に次のいずれかを求める。

- approve
- reject
- 方針案への疑問・反論
- 追加確認の指示

reject、疑問、反論、追加確認の指示があった場合は、方針 Comment を投稿せず、人間と議論する。

議論では次を明確にする。

- どの解釈が妥当か
- どの verifier判定を採用するか
- どの修正方針を採用するか
- 対応範囲をどこまでにするか
- 対応しない範囲をどう切るか

#### 4-3. 合意後の反映

議論の結果、approve された Thread にだけ合意内容を反映する。

方針が対応する場合は、方針 Comment を投稿し、この Step では resolve しない。

```text
方針：<合意した方針>
根拠：<合意理由>
対応しない範囲：<合意した対応しない範囲>
```

方針が対応見送りの場合は、方針 Comment を投稿せず、この Step で Thread を resolve する。

```sh
{{path_alias.releash}} review resolve <thread-id> --session-id "$RELEASH_SESSION_ID" --outcome wontfix --summary "<対応見送りの理由>" --json
```

resolve summary には、合意した方針、合意理由、対応しない範囲を含める。

approve されていない Thread に方針 Comment 投稿や resolve を行ってはならない。

### 5. 完了確認

#### 5-1. Thread 状態の再取得

最初に取得した全 Thread の現在状態を再取得する。

#### 5-2. 方針反映の確認

再取得した全 Thread について、方針の反映結果と Thread 状態を確認する。

対応する方針の Thread には、次の3項目を含む方針 Comment が投稿されていなければならない。

- 方針
- 根拠
- 対応しない範囲

対応見送り方針の Thread は resolve 済みで、resolve summary に次の3項目が含まれていなければならない。

- 方針
- 根拠
- 対応しない範囲

Thread 状態は次の条件を満たしていなければならない。

- 対応する方針の Thread: Open のまま残っている
- 対応見送り方針の Thread: resolve 済みである
- 除外グループの Thread: 既存方針 Comment が存在する

#### 5-3. 未完了がある場合

対応する方針の Thread の方針 Comment 未投稿、または対応見送り Thread の未resolve が1件でもある場合、完了として扱ってはならない。

未完了 Thread を特定し、該当する処理に戻る。

- 一致グループの合意前なら `3-2. 合意確認`
- 一致グループの approve 済みなら `3-3. 合意後の反映`
- 割れグループの議論中なら `4-2. 合意確認と議論`
- 割れグループの approve 済みなら `4-3. 合意後の反映`

#### 5-4. 完了報告

全 Thread が次の条件を満たす場合だけ、完了報告を出す。

- 対応する方針の Thread には方針 Comment があり、Open のまま残っている
- 対応見送り方針の Thread は resolve summary に方針と根拠があり、resolve 済みである
- 除外グループの Thread には既存方針 Comment がある

```text
## 方針決定 完了

対象 Open Thread: <件数>件
除外グループ: <件数>件
一致グループで投稿: <件数>件
割れグループで投稿: <件数>件
対応見送りとして resolve: <件数>件
未完了: 0件
```

## 方針 Comment / resolve summary のフォーマット

対応する方針の Thread には、合意済みの方針案から方針 Comment を作成する。
対応見送り方針の Thread には、方針 Comment を投稿せず、resolve summary に方針と根拠を含める。
合意前の内容、未承認の方針案、議論途中の内容を Comment または resolve summary に使ってはならない。

対応する方針の Thread に投稿する方針 Comment:

```text
方針：<合意した方針>
根拠：<reviewer指摘・verifier判定・verifier修正方針・議論内容を踏まえた採用理由>
対応しない範囲：<この Thread では扱わないこと。なければ `なし`>
```

対応見送り方針の Thread に使う resolve summary:

```text
方針：<対応見送りの方針>
根拠：<reviewer指摘・verifier判定・verifier修正方針・議論内容を踏まえた対応見送り理由>
対応しない範囲：<この Thread では扱わないこと。なければ `なし`>
```

各項目の内容:
- `方針`: 実装 Step でその Thread をどう扱うかを書く
- `根拠`: なぜその方針を採用したかを書く
- `対応しない範囲`: 余計な修正や解釈を防ぐため、この Thread で扱わないことを書く

許可:
- 実装 Step でその Thread をどう扱うか
- 実装後に満たすべき状態
- 対応範囲
- 対応しない範囲

禁止:
- 「対応する」「対応見送り」だけで終わる方針
- reviewer指摘の要約だけで根拠を省略すること
- verifier判定だけを根拠にして、verifier修正方針を無視すること
- 合意した方針案と異なる内容を投稿すること
- 実装手順、変更箇所候補、具体的なコード変更方法を書くこと

## 禁止事項

- 実装の着手・コード変更
- 方針 Comment 以外の Comment 投稿
- Open Thread の一部だけを対象にして進めること
- 方針未決の Thread を残したまま完了扱いにすること
- approve 前の方針案を方針 Comment として投稿すること
- approve 前に Thread を resolve すること
- 対応見送り以外の方針で Thread を resolve すること
- 対応見送り方針の Thread に方針 Comment を投稿して Open のまま残すこと
- 対応する方針の Thread をこの Step で resolve すること
- 合意済み方針と異なる内容を方針 Comment または resolve summary に書くこと
- verifier判定だけを見て、verifier修正方針を見ないこと
- 「対応する」「対応見送り」だけの雑な方針を書くこと
- コードレベルの実装手順、変更箇所候補、具体的な修正方法を書くこと
