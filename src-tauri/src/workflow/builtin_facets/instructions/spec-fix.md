{{project_name}} プロジェクトの Spec 3文書を、承認済み Spec 修正方針に基づいて修正する。

## 入力

- `spec-directory` Contract
- `approved-fix-policy` Contract

## 早期終了判定（最優先）

`approved-fix-policy.findings` の **全件が `action: "skip"`** の場合、ファイル編集を一切行わず、レスポンス末尾に厳密に以下の1行のみを出力して終了する:

```
NO_FIX_NEEDED
```

前後に説明文を一切付けない。この1トークンがワークフローのルーティングキーとして使われる。

## 目的

`requirements.md`、`behavior.md`、`design.md` のうち、承認済み方針で `action: fix` とされた指摘だけを修正する。実装詳細は追加しない。

## 手順

1. `spec_dir` 配下の 3文書を読む。
2. approved-fix-policy の findings を確認する。
3. `action: fix` の指摘だけを対象文書へ反映する。
4. `action: skip` の指摘は変更しない。
5. 既存の文体、用語、scope を維持する。

## 書かないこと

- 実装順序
- ファイルごとの編集手順
- helper 関数名
- 関数内処理
- 疑似コード
- 詳細な型定義
- テストケース名
- security 実装詳細

## 出力

修正後、簡潔に変更した文書と要点を報告する。構造化出力は不要。
