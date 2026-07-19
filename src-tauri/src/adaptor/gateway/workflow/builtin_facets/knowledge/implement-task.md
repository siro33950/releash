# Implement Task

Implement Taskは、Specから導出された、独立して実装結果を検証できる実行単位である。

Task群は確認済みSpecの実装内容を完全に表し、Specにない要求や観測可能な挙動を追加しない。

## フォーマット

```json
{
  "tasks": [
    {
      "task_id": "T-001",
      "requirements": ["R-001", "R-003"],
      "depends_on": [],
      "parallel": true,
      "files": ["src-tauri/src/usecase/..."],
      "outputs": [
        "rejectionを処理するusecase",
        "正常系・異常系テスト"
      ],
      "verify": [
        {
          "condition": "理由付きでrejectするとrunがrejectedになり、理由が履歴へ保存される"
        },
        {
          "condition": "reject後に実行中actionが再開されない"
        }
      ]
    }
  ]
}
```

## 必須項目

### Task ID

- `T-001`形式の、Task群の中で一意かつ安定したIDを使用する。
- 並び替えや他Taskの追加によって、既存TaskのIDを別のTaskへ再利用しない。

### Requirements

- 対応するRequirement IDを一つ以上記載する。
- 複数ある場合はすべて列挙する。
- Requirementに対応しないTaskを作らない。

### Depends on

- 先に完了していなければ実装または検証できないTask IDを列挙する。
- 依存がない場合は空配列にする。
- Task群の依存関係を循環させない。

### Parallel

- 依存Taskの完了後、他の実行可能なTaskと並列に実装できる場合は`true`、できない場合は`false`と記載する。
- 同じファイル、同じ状態、同じ生成物への競合する変更があるTaskを`true`にしない。
- `true`は並列実行可能であることを表し、実際にFanoutする指示ではない。

### Files

- 変更または追加する正確なリポジトリ相対パスを列挙する。
- 対象を特定できないディレクトリ名、glob、存在を確認していないパスを使用しない。

### Output

- Task完了時に存在すべきコード、型、処理、文書、その他の成果を列挙する。
- 実装担当が完了を判定できる具体的な成果を書く。
- Specの範囲を超えた成果を追加しない。

### Verify

- Taskが完了したと一意に判断できる`condition`を一つ以上記載する。
- 各`condition`は、現在の実装に対して成立・不成立を判定できる観測可能な条件にする。
- 対象、入力または事前条件、成立時に観測できる状態や出力を具体的に記載する。
- 実行コマンドや確認手段ではなく、確認によって成立を判断する条件を書く。
- 「正しく動作する」「問題がない」など、複数の意味に解釈できる表現を使用しない。
- Specにない完了条件を追加しない。

## Task群全体の条件

- すべてのRequirement IDが一つ以上のTaskに対応する。
- 各Taskは単独でOutputとVerifyの成否を判定できる。
- 依存関係から実装順序を決定できる。
- Task間で要求の重複、矛盾、未割り当てを作らない。
