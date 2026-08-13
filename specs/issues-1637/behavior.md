## B-001: Shift+Enterによる改行挿入入力

GIVEN workspaceターミナルまたはAgent sessionのTUIのターミナルsurfaceにフォーカスがある
WHEN 利用者がShift+Enterを押す
THEN PTY入力へ`\x1b\r`（ESC + CRの2バイト）が送られる
AND `\r`は重複して送られない

## B-002: Cmd+Enterによる改行挿入入力

GIVEN workspaceターミナルまたはAgent sessionのTUIのターミナルsurfaceにフォーカスがある
WHEN 利用者がCmd+Enterを押す
THEN PTY入力へ`\x1b\r`（ESC + CRの2バイト）が送られる
AND `\r`は重複して送られない

## B-003: 修飾キーなしのEnter入力の維持

GIVEN workspaceターミナルまたはAgent sessionのTUIのターミナルsurfaceにフォーカスがある
WHEN 利用者が修飾キーなしのEnterを押す
THEN PTY入力へ`\r`が送られる

## B-004: IME変換中のEnter入力の維持

GIVEN workspaceターミナルまたはAgent sessionのTUIのターミナルsurfaceにフォーカスがあり、IMEによる未確定文字列がある
WHEN 利用者がEnterを押す（修飾キーなし、Shift、Cmd、Ctrl、Altのいずれを伴う場合も含む）
THEN PTY入力へ`\x1b\r`は送られない
AND PTY入力へEnter由来の`\r`も送られない
AND IMEが確定した文字列がPTY入力へ送られる

## B-005: その他のキー入力の維持

GIVEN workspaceターミナルまたはAgent sessionのTUIのターミナルsurfaceにフォーカスがある
WHEN 利用者がShift+Enter、Cmd+Enter、IME変換中のEnter、または既存のペイン操作ショートカット以外のキー入力を行う
THEN PTY入力へ送られる内容は本変更前と同じである

## B-006: ペイン操作ショートカットの入力抑止の維持

GIVEN workspaceターミナルまたはAgent sessionのTUIのターミナルsurfaceにフォーカスがある
WHEN 利用者がCmd+D、Cmd+Shift+D、またはCmd+Option+矢印を押す
THEN そのキー入力はPTY入力へ送られない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-003, B-005, B-006 |
| R-006 | B-001, B-002, B-003, B-004, B-005, B-006 |
