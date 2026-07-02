# Behavior

## Source
- requirements.md

## Behavior

```gherkin
# language: ja
機能: Agent backend 境界の分離
  ルール: backend 固有の振る舞いは各 backend infrastructure の内側に閉じる
    シナリオ: Claude 固有の詳細は Claude infrastructure 内に留まる
      前提 Agent session が Claude backend を使用している
      もし session が Claude 固有の実行状態、event、permission、skill、復旧結果を受け取る
      ならば それらの Claude 固有詳細は Claude backend infrastructure の内側で扱われる
      かつ Agent session は agent_session の Entity と backend interface の結果だけを受け取る

    シナリオ: Codex 固有の詳細は Codex infrastructure 内に留まる
      前提 Agent session が Codex backend を使用している
      もし session が Codex 固有の実行状態、event、permission、skill、復旧結果を受け取る
      ならば それらの Codex 固有詳細は Codex backend infrastructure の内側で扱われる
      かつ Agent session は agent_session の Entity と backend interface の結果だけを受け取る

    シナリオ: backend 固有の source 値は workflow surface に露出しない
      前提 workflow または user surface が Agent session を観測している
      もし 選択された backend が backend 固有の source 値を生成する
      ならば その source 値は backend infrastructure の外で session state として露出しない
      かつ 観測可能な状態は Session、Turn、Message、MessagePart、PermissionRequest、または表示・選択用 DTO として表現される

  ルール: backend 間で実行、変換、復旧、権限処理、skill 解決を共有しない
    シナリオ: 選択された backend が自身の lifecycle 作業を行う
      前提 Agent session が backend を選択している
      もし session が実行、event 変換、permission 解決、skill 解決、または復旧を行う
      ならば 選択された backend infrastructure が自身の lifecycle としてその作業を行う
      かつ その作業は Claude と Codex をまとめる共有実装へ流されない

    シナリオ: backend の変更は当該 backend 実装に局所化される
      前提 Claude と Codex が異なる runtime 概念を持っている
      もし 一方の backend が実行、変換、復旧、permission 処理、または skill 解決の方法を変更する
      ならば もう一方の backend はその backend 固有の振る舞いを経由する必要がない

  ルール: backend infrastructure は agent_session の Entity を返す
    シナリオ: backend event は infrastructure を出る前に変換される
      前提 backend infrastructure が backend 固有の入力、状態、または event を受け取っている
      もし backend infrastructure が Agent session へ進捗または結果を報告する
      ならば 変換済みの agent_session Entity を返す
      かつ backend 固有の入力、状態、event 値を session contract として返さない

    シナリオ: permission request は agent_session Entity として表現される
      前提 backend が turn 中に permission を要求している
      もし その permission request が backend infrastructure の外で観測可能になる
      ならば PermissionRequest Entity として表現される
      かつ backend 固有の permission source 値を読まなくても turn を理解できる

  ルール: Agent session 実行側は Entity と backend interface を使う
    シナリオ: session state は backend wire format や lifecycle 知識なしに扱われる
      前提 Agent session が対応 backend のいずれかで実行中である
      もし 実行側が session、turn、message、または permission state を観測する
      ならば 実行側は Entity と backend interface を通じてその状態を扱う
      かつ backend 固有の wire format や process lifecycle 詳細を解釈しない

    シナリオ: turn 完了は共通の session state として観測される
      前提 backend が turn を完了している
      もし Agent session 実行側がその完了を受け取る
      ならば 結果の turn と message は agent_session Entity として利用できる
      かつ 後続の workflow 振る舞いは backend native の完了形式に依存しない

  ルール: backend_id は metadata と dispatch 入力に限定する
    シナリオ: backend_id は dispatch 境界で backend を選択する
      前提 session が backend metadata を持っている
      もし session が開始または復帰される
      ならば backend_id は registry または dispatch 境界で対象 backend 実装を選ぶために使われる
      かつ 選ばれた backend がその後の backend 固有の振る舞いを所有する

    シナリオ: backend 選択後の振る舞いは backend_id で決めない
      前提 session の backend 実装がすでに選択されている
      もし 実行、変換、復旧、permission 処理、skill 解決、または model 解釈が行われる
      ならば それらの振る舞いは選択済み backend 実装の外で backend_id 分岐によって決められない

  ルール: Codex app-server event は Codex infrastructure が直接変換する
    シナリオ: Codex の turn 活動は agent_session Entity になる
      前提 Codex infrastructure が turn に関する Codex app-server 活動を受け取っている
      もし Codex infrastructure がその活動を外部へ報告する
      ならば その活動は agent_session Entity へ直接変換される
      かつ Agent session は Claude 互換の中間 message を受け取らない

    シナリオ: Codex の permission 活動は Codex の変換経路に留まる
      前提 Codex infrastructure が Codex approval または sandbox 活動を受け取っている
      もし その活動が permission request として観測可能になる
      ならば Codex infrastructure がそれを PermissionRequest Entity へ直接変換する
      かつ その活動は Claude event handling へ流されない

  ルール: Claude SDK と bridge event は Claude infrastructure が直接変換する
    シナリオ: Claude の turn 活動は agent_session Entity になる
      前提 Claude infrastructure が turn に関する Claude SDK または bridge 活動を受け取っている
      もし Claude infrastructure がその活動を外部へ報告する
      ならば その活動は agent_session Entity へ直接変換される
      かつ Agent session は backend native の Claude event 値を受け取らない

    シナリオ: Claude の permission 活動は Claude の変換経路に留まる
      前提 Claude infrastructure が Claude permission 活動を受け取っている
      もし その活動が permission request として観測可能になる
      ならば Claude infrastructure がそれを PermissionRequest Entity へ直接変換する
      かつ その活動は Codex event handling へ流されない

  ルール: permission と approval payload は共通の PermissionRequest 振る舞いとしてだけ露出する
    シナリオ: user surface が backend の permission request を観測する
      前提 任意の backend が permission decision を要求している
      もし desktop UI、remote UI、workflow、または headless 経路がその request を観測する
      ならば request は共通の PermissionRequest Entity として提示される
      かつ 観測者は backend 固有の permission payload または approval payload 値を扱わない

    シナリオ: permission response は共通の session contract を通じて受理される
      前提 PermissionRequest Entity が decision を待っている
      もし user surface または workflow 経路が decision を提供する
      ならば decision は PermissionRequest Entity に関連付けられる
      かつ 選択された backend infrastructure がその decision を自身の lifecycle 用に変換する

    シナリオ: permission 履歴は変換後も理解できる
      前提 session history に permission request と decision が含まれている
      もし history が復帰または確認される
      ならば permission state は共通の agent_session state として利用できる
      かつ backend 固有の permission payload を読まなくても history を理解できる

  ルール: 復旧は各 backend infrastructure lifecycle が所有する
    シナリオ: Claude の復旧は Claude infrastructure の振る舞いを使う
      前提 Claude session が復旧を必要としている
      もし 復旧が試みられる
      ならば Claude infrastructure が自身の lifecycle を使って復旧を行う
      かつ その復旧は Codex の復旧振る舞いと共有されない

    シナリオ: Codex の復旧は Codex infrastructure の振る舞いを使う
      前提 Codex session が復旧を必要としている
      もし 復旧が試みられる
      ならば Codex infrastructure が自身の lifecycle を使って復旧を行う
      かつ その復旧は Claude の復旧振る舞いと共有されない

    シナリオ: 復旧結果は session state として返される
      前提 backend の復旧試行が完了している
      もし Agent session が復帰後または失敗後の状態を観測する
      ならば その結果は agent_session state として表現される
      かつ backend 固有の復旧詳細は backend infrastructure の内側に留まる

  ルール: model 取得と解釈は backend infrastructure の内側に閉じる
    シナリオ: model choices は表示・選択用データとして露出する
      前提 user または workflow が backend の利用可能 model を必要としている
      もし backend が model choices を提供する
      ならば backend infrastructure が backend 固有の model 情報を解釈する
      かつ user または workflow は backend native の model 値ではなく表示・選択用データを受け取る

    シナリオ: model selection は変換済み session state に保持される
      前提 新規作成または保存された session が selected model を持っている
      もし session が復帰または観測される
      ならば model selection は変換済み session state を通じて session に関連付けられたままである
      かつ 観測者は backend 固有の model 取得振る舞いを必要としない

  ルール: 既存 surface は Agent session state を Entity と backend interface で扱う
    シナリオ: desktop と remote の利用者は同じ backend 非依存 state を観測する
      前提 desktop または remote の利用者が Agent session を観測している
      もし session が Claude または Codex で実行される
      ならば session status、message、permission request、復旧結果、model selection は backend 非依存 state として観測される
      かつ 利用者は backend 実装詳細を区別する必要がない

    シナリオ: workflow と headless 実行は backend 非依存の結果を受け取る
      前提 workflow または headless 実行経路が Agent session を実行している
      もし 選択された backend が message、permission request、turn 完了、または復旧結果を生成する
      ならば workflow または headless 経路は Entity と backend interface の結果を受け取る
      かつ backend 固有 payload を workflow state として消費しない

  ルール: 既存の利用者向け session 振る舞いは分離後も backend ごとの境界で成立する
    シナリオ: 通常実行と turn 完了が共有実装なしで引き続き機能する
      前提 backend 分離後に Claude または Codex session が作成されている
      もし user または workflow が通常の turn を実行する
      ならば turn は観測可能な session state と message を伴って完了する
      かつ その完了は選択された backend infrastructure が直接返す Entity によって成立する
      かつ backend 境界の分離は利用者から見える通常実行の意味を変えない

    シナリオ: permission request と response が共有 payload 処理なしで引き続き機能する
      前提 Claude または Codex の turn が permission decision を必要としている
      もし decision が共通の permission 振る舞いを通じて提供される
      ならば 選択された backend はその decision に従って継続または停止できる
      かつ decision の backend 固有変換は選択された backend infrastructure の内側で行われる
      かつ user の permission response の意味は保持される

    シナリオ: 復旧と履歴復帰が共有復旧処理なしで引き続き機能する
      前提 backend 分離後に作成された Claude または Codex session が保存済み history を持っている
      もし session が復帰または復旧される
      ならば session に必要な backend_id、agent_session_id または backend thread identity、message、permission state、model selection が保持される
      かつ 復旧は選択された backend infrastructure の lifecycle として行われる
      かつ 復帰した session は共通の Agent session 振る舞いを通じて利用できる

    シナリオ: 既存実装経路の踏襲は受け入れ条件にならない
      前提 既存の Claude / Codex session で利用者から見える振る舞いが存在する
      もし backend 境界分離後の受け入れを評価する
      ならば 評価対象は利用者から見える意味が維持されていることである
      かつ Claude 互換の中間 message、共有 lifecycle、共有変換、共有復旧、共有 permission 処理を踏襲していることは受け入れ条件にならない

  ルール: 既存保存済み session は互換性保証の対象外である
    シナリオ: 分離前に保存された session が評価される
      前提 session がこの backend 境界分離より前に保存されている
      もし 分離後の backend 振る舞いの受け入れを評価する
      ならば その既存保存済み session に対する migration 互換性は要求されない
      かつ 受け入れは分離後に作成・保存される session を対象にする

  ルール: backend native 値を frontend や workflow の domain logic にしない
    シナリオ: user surface は backend 非依存の session data を表示または提出する
      前提 frontend または workflow surface が Agent session 振る舞いに参加している
      もし その surface が session state を表示し、user input を受け取り、または decision を提出する
      ならば 変換済み session data を通じて振る舞いに参加する
      かつ backend 固有の実行、変換、復旧、permission、skill、model decision を所有しない

    シナリオ: 判断に必要な backend 固有情報は変換によって保持される
      前提 表示、history、復帰、permission response、または workflow 判断に backend 固有情報が必要である
      もし backend infrastructure がその情報を Entity または DTO へ変換する
      ならば 変換済み state はそれらの振る舞いに必要な情報を保持する
      かつ 不要な backend native source 値は backend infrastructure の内側に隠れる

  ルール: scope は backend 境界分離に限定する
    シナリオ: 新しい backend は要求されない
      前提 backend 境界の分離が完了している
      もし 対応 backend を評価する
      ならば Claude と Codex が要求される対応 backend のままである
      かつ 別の Agent backend 追加は受け入れに要求されない

    シナリオ: user-facing な意味は変えない
      前提 既存 user が Agent session 実行、permission request への応答、model 選択、session 復帰、または workflow / headless 実行を行う
      もし backend 境界の分離が完了している
      ならば それらの user-facing な意味は変わらない
      かつ 受け入れに chat、permission、model selection surface の redesign は要求されない
```
