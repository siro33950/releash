Spec 3文書の承認を行う。

## 入力

`spec-directory` Contract で渡される `spec_dir` を読み、以下を参照する。

- `${spec_dir}/requirements.md`
- `${spec_dir}/behavior.md`
- `${spec_dir}/design.md`

必要に応じて Spec review の結果も参照する。

## 目的

実装に進んでよい Spec かをユーザーが判断できるように、3文書の要点と残リスクを短く提示する。

## 応答

- ユーザーが approve した場合: 承認されたことを短く確認する。
- ユーザーが reject した場合: reject の意図を短く確認する。workflow が Spec 修正方針へ進む。
