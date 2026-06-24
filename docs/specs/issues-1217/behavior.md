# Behavior

内部構造のリファクタリング（巨大 module `bridge_common.rs` の責務別分割）タスクのため、本書は「分割後に外部から観測される振る舞い」を定義する。新機能の追加や挙動変更はなく、観測可能な差分は「agent bridge の挙動が分割前後で一切変わらないこと（回帰なし）」と「成果物が健全であること」に集約される。

どの module に何を分割するか、ファイル分割粒度、`pub` / `pub(crate)` 公開境界といった実装経路は振る舞いではないため本書には含めない。分割対象の責務区分・配置は `requirements.md` および `design.md` を参照する。

## 仮定

- **A1**: 「分割前」とは本 Issue 着手前の `bridge_common.rs` を単一 module とする実装、「分割後」とは責務別 module へ分割した実装を指す。本書の各 Scenario は「分割前後で観測結果が同一であること」を確認する。
- **A2**: 「観測可能な挙動」とは、agent bridge が公開する command の入出力、emit される event の内容・順序・タイミング、永続化結果（streaming parts / post-turn base parts / turn event log）、permission フローの結果、session 復元結果、およびエラー挙動を指す。
- **A3**: public command の signature・名前・呼び出し経路は分割前後で維持される（frontend は変更しない）。
- **A4**: read command が現在持つ write side-effect の実除去は本 Issue では行わない。本書は「境界が module 構造で識別できること」を扱うが、read command の観測可能な出力自体は分割前後で同一である。
- **A5**: 削除候補の一覧化は本 Issue の成果物だが、実削除（#878）は行わない。よって削除候補に挙がった command surface / compat path も、本 Issue 完了時点では従来どおり動作する（回帰なし）。
- **A6**: ビルド・clippy・test の緑は受け入れ条件（プロセス上の完了条件）であり、本書では「成果物が壊れていないこと」を表す観測点として `Rule: 分割後も成果物は健全である` に含める。

## Feature: agent bridge module の責務別分割

`bridge_common.rs` に同居する runtime / process registry・stream emit・session persistence・permission・recovery の各責務を責務別 module へ分割する。
分割後、エンドユーザーおよび frontend から見て agent bridge の挙動は一切変化しない。

### Background

```gherkin
Background:
  Given Releash デスクトップアプリが分割後のコードからビルドされている
  And ユーザーがアプリを起動して agent セッションを利用できる
```

## Rule: agent チャットの streaming 挙動が分割前後で一致する

```gherkin
Scenario: ターン中の streaming_parts emit が従来どおり行われる
  Given ユーザーが agent にメッセージを送信してターンが進行している
  When agent が逐次トークンを生成する
  Then streaming_parts を含む event が分割前と同一の内容・順序・タイミングで Tauri event と WS の両チャネルへ emit される

Scenario: ターン完了時に streaming_parts が解放される
  Given agent のターンが完了する
  When ターン完了処理が行われる
  Then streaming_parts は分割前と同一の条件で解放され、post-turn base parts が分割前と同一に確定する

Scenario: flush 閾値・集約間隔が従来どおり適用される
  Given agent が大量のトークンを高頻度で生成している
  When emit interval / flush 閾値（pending part 上限・byte size cap）に達する
  Then 分割前と同一の閾値・間隔で集約 emit が行われる
```

## Rule: permission フローの結果が分割前後で一致する

```gherkin
Scenario: permission mode の設定が従来どおり反映される
  Given ユーザーが agent の permission mode を設定する
  When 以降のターンで permission 判定が必要になる
  Then 分割前と同一の mode が適用され、同一の resolution が記録される

Scenario: permission 応答が従来どおり処理される
  Given agent が permission 要求を発行している
  When ユーザーが許可または拒否を応答する
  Then 分割前と同一の挙動でターンが継続または中断され、resolution が記録される
```

## Rule: session 永続化・復元の結果が分割前後で一致する

```gherkin
Scenario: ターンの永続化結果が従来どおりになる
  Given agent のターンが完了する
  When streaming parts / post-turn base parts / turn event log が永続化される
  Then 分割前と同一の内容が session storage に記録される

Scenario: session_ready 時の復元結果が従来どおりになる
  Given 既存の agent セッションがある
  When session_ready により resume / context 復元が行われる
  Then 分割前と同一の session 状態・context が復元される
```

## Rule: process 死活検知・再 spawn の挙動が分割前後で一致する

```gherkin
Scenario: ターン完了後に bridge プロセスが死んでも検知・再 spawn される
  Given agent のターンが完了して bridge プロセスが終了する
  When 次の操作で bridge プロセスが必要になる
  Then 分割前と同一の条件で死活が検知され、再 spawn される（#1192 で確立した挙動を維持する）

Scenario: orphan process が従来どおり cleanup される
  Given 前回起動の bridge プロセスが PID ファイルに残存している
  When アプリ起動時の orphan cleanup が実行される
  Then 分割前と同一の条件で orphan process が cleanup される
```

## Rule: command の入出力が分割前後で一致する

```gherkin
Scenario Outline: 公開 command の signature と入出力が維持される
  Given frontend が agent bridge の <command> を呼び出す
  When 分割後のコードで <command> が実行される
  Then command の名前・signature は分割前と同一であり、入出力も分割前と同一になる

  Examples:
    | command                  |
    | read 系 command（取得系） |
    | write 系 command（更新系） |

Scenario: read command の観測可能な出力が分割前後で一致する
  Given read 経路と write 経路が module 構造上で識別できるように分割されている
  When frontend が read command（取得系）を呼び出す
  Then read command の出力は分割前と同一であり、本 Issue では write side-effect の実除去は行われない
```

## Rule: 削除候補に挙げた command surface / compat path も本 Issue 完了時点では従来どおり動作する

```gherkin
Scenario: 削除候補一覧が #878 から参照できる形で残る
  Given 分割の過程で frontend から使われない command surface / compat path が判明している
  When それらを削除候補として一覧化する
  Then 一覧は #878 が参照・実行できる形式で残る

Scenario: 削除候補は本 Issue では削除されず回帰しない
  Given command surface / compat path が削除候補として一覧化されている
  When 本 Issue の変更が適用される
  Then 削除候補に挙がった経路も本 Issue 完了時点では削除されず、従来どおり動作する
```

## Rule: 分割後も成果物は健全である

```gherkin
Scenario: Rust 成果物が緑である
  When `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を src-tauri/ で実行する
  Then いずれも警告・失敗なく成功する

Scenario: 既存テストが期待値変更なしで pass する
  Given 既存の `#[cfg(test)]` テストが対応する責務の module へ移動されている
  When 移動後のテストを実行する
  Then 期待値を変更することなくすべて pass する

Scenario: 各責務の module に境界テストが存在する
  Given runtime / process registry・stream emit・session persistence・permission・recovery の各 module に分割されている
  When 各 module のテストを確認する
  Then それぞれの責務に対応する境界テストが存在する
```

## Open Questions

なし。
