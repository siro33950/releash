# Requirements

## Goal

Agent backend ごとの個別実装を infrastructure 層に閉じ込め、Claude と Codex を同じ共通処理で扱う実装をなくし、infrastructure 層の実装結果を Releash の Entity に変換してから利用する状態にする。

完了時には、Claude / Codex などの backend 固有 wire format、process lifecycle、SDK / app-server 固有イベント、権限処理、skill directory、復旧処理などが infrastructure 層の外で直接扱われていない。Claude と Codex の実行・変換・復旧をまとめる共通処理は持たず、backend ごとの infrastructure 実装がそれぞれ Entity へ変換する。infrastructure 層の外では、backend 固有実装ではなく、変換済みの Session / Turn / Message / MessagePart / PermissionRequest などの Entity を使って処理する。

共有してよいものは、agent_session の Entity 定義と、backend 実装が Entity を返すための interface / trait に限定する。Claude / Codex の runtime、bridge、変換、復旧、権限処理、skill 解決などの実処理は共有しない。

## Background

現在の Agent backend 実装では、Claude と Codex の実行・変換・復旧を同じ共有処理で扱っている。コード上、Codex app-server の JSON-RPC message は、Claude 側と同じ処理へ流すための中間 message に組み立て直され、その中間 message を共有処理が解釈して Entity へ変換している。

つまり、Codex 固有の app-server event を Codex backend 実装内で直接 Entity 化しているのではなく、中間形式へ変換してから Claude と同じ message 処理へ流している。Claude / Codex の実装を interface で分けるのではなく、同じ処理に載せるための変換と判定が存在している。

同じ傾向は権限、skill 解決、永続化、復旧処理にもある。共有処理が backend_id を見て Claude 用の権限 payload と Codex 用の権限 payload を分けたり、backend ごとの skill directory を切り替えたりしている。永続化・復旧処理にも backend 固有の既定値や process lifecycle 前提が入り込んでいる。

これは、backend ごとの interface とカプセル化が成立していない実装上の問題である。backend 固有処理が infrastructure 層の内側に閉じていれば、呼び出し側は実行・変換・復旧などの処理内容を backend_id で分岐する必要がない。backend_id は session metadata として保持し、registry / dispatch 境界で対象 backend 実装を選ぶためには使うが、選ばれた後の backend 固有処理は各 infrastructure 実装の内側に閉じる。現状は、Claude / Codex を同じ処理で扱うために、backend 固有の変換や判断が共有処理へ入り込んでいる。

本変更は、語彙整理ではなく実装境界の修正である。backend ごとの個別実装を infrastructure 層へ分離し、infrastructure 層から外へ出すものを Entity に揃えることで、Agent session 側が backend 固有実装を直接扱わない状態にする。

## Users / Actors

- Agent session 実行側: backend 実装から返された Entity を使って session / turn / message / permission request を扱う。
- Claude backend infrastructure 実装: Claude SDK / Node bridge / Claude 固有の wire format、process lifecycle、権限処理、復旧処理を扱い、Entity へ変換して返す。
- Codex backend infrastructure 実装: Codex app-server / JSON-RPC / Codex 固有の thread、turn、item、approval、sandbox、復旧処理を扱い、Entity へ変換して返す。
- Workflow / headless 実行経路: Agent session を backend 非依存に実行し、backend 固有処理ではなく Entity と interface を介して結果を受け取る。
- デスクトップ UI / remote UI の利用者: backend 差分を意識せず、session の実行状態、message、permission request、復旧結果を観測・操作する。

## Requirements

- Claude / Codex など backend ごとの wire format、event、process lifecycle、権限 payload、skill 解決、復旧処理は、それぞれの infrastructure 実装の内側で扱うこと。
- Claude / Codex の実行・変換・復旧・権限処理・skill 解決をまとめる共有処理を持たないこと。
- 各 backend infrastructure 実装は、backend 固有の入力・状態・イベントを agent_session の Entity へ変換してから返すこと。
- Agent session 実行側は、backend 固有の wire format や process lifecycle ではなく、Entity と backend interface を介して session / turn / message / permission request を扱うこと。
- backend_id は session metadata として保持し、registry / dispatch 境界で対象 backend 実装を選ぶために使うこと。ただし、選ばれた後の実行・変換・復旧・権限処理・skill 解決の中身を backend_id で分岐しないこと。
- Codex の app-server event は、Claude 側と同じ処理へ流すための中間 message へ変換せず、Codex infrastructure 実装内で直接 Entity へ変換すること。
- Claude の SDK / bridge event は、Claude infrastructure 実装内で直接 Entity へ変換すること。
- backend 固有の permission / approval payload は infrastructure 実装の内側に閉じること。デスクトップ UI、remote UI、workflow / headless 経路が扱う permission request と応答は、backend 固有 payload の生値ではなく、共通の PermissionRequest Entity として提供すること。
- backend 固有の復旧処理は、他 backend と共有せず、各 backend infrastructure 実装の lifecycle として扱うこと。
- backend 固有の model 取得・解釈は infrastructure 実装の内側に閉じること。デスクトップ UI や session 実行側が扱う model list / model selection は、backend 固有 API の生値ではなく、表示・選択用に変換済みの Entity / DTO として提供すること。
- 既存のデスクトップ UI、remote UI、workflow / headless 経路は、backend 固有実装ではなく Entity と backend interface を介して Agent session の状態と結果を扱うこと。
- 既存の Claude / Codex session の通常実行、permission request、turn 完了、復旧、履歴復帰が、この分離後も成立すること。

## Constraints

- 本変更では既存の保存済み session との後方互換を要求しない。
- 変更後に作成・保存される Claude / Codex session について、backend_id、agent_session_id / thread_id、message、permission state、model selection を破壊しないこと。
- backend 固有の SDK / CLI / app-server / Node bridge / JSON-RPC / permission payload / sandbox policy の生値を frontend の domain logic や workflow 実行側へ持ち込まないこと。
- frontend は表示、入力受付、backend command 呼び出し、最小限の表示用整形に留めること。backend 固有の実行・変換・復旧判断を frontend に置かないこと。
- backend を選択する registry / dispatch 境界は残してよい。ただし、その境界の外で backend ごとの実処理内容を分岐しないこと。
- Claude / Codex の既存ユーザー操作、permission 応答、model 選択、session 復帰、workflow / headless 実行の意味を変えないこと。
- Entity に変換する際、backend 固有情報のうち UI 表示、履歴、復帰、権限応答、workflow 判断に必要な情報を落とさないこと。

## Scope

- Claude backend infrastructure 実装と Codex backend infrastructure 実装を分離し、両者を同じ実行・変換・復旧・権限処理へ流す構造を解消する。
- Claude 固有の SDK / bridge / process lifecycle / watchdog / permission payload / model 取得を Claude infrastructure 実装内に閉じる。
- Codex 固有の app-server / JSON-RPC / thread / turn / item / approval / sandbox / model 取得を Codex infrastructure 実装内に閉じる。
- Claude / Codex それぞれの infrastructure 実装から、agent_session の Entity と表示・選択用 DTO を返す境界を整える。
- Agent session 実行側、workflow / headless 経路、desktop / remote UI が、backend 固有 payload ではなく Entity と DTO を介して状態・結果・permission・model を扱うようにする。
- backend_id を使う場所を、session metadata と registry / dispatch 境界に限定する。
- 後方互換を前提にせず、変更後の保存・復帰・実行が分離後の境界で成立するようにする。

## Non-goals

- GLOSSARY の追記や語彙整理そのものを主目的にしない。
- Claude / Codex 以外の新しい Agent backend を追加しない。
- Agent SDK / CLI / Codex app-server 自体の仕様を変更しない。
- Agent chat UI、permission UI、model selector UI のデザイン刷新を行わない。必要な接続変更に留める。
- 既存保存済み session の後方互換 migration を行わない。
- backend 固有機能をすべて共通 Entity に抽象化しきることを目的にしない。UI 表示、履歴、復帰、権限応答、workflow 判断に必要な情報を Entity / DTO として提供する範囲に留める。
- performance 最適化や storage 再設計を主目的にしない。
