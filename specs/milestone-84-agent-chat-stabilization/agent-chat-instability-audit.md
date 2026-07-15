# Agent チャット不安定性の問題点インベントリ（全数監査）

- 調査日: 2026-07-07
- 設計追補日: 2026-07-15
- 対象: main `b3f9f54c` 時点の working tree

手法: 7 視点の並列監査（72 subagent）で候補を洗い出し、全指摘を独立の検証者が実コードで反証確認した。
57 件を確定、8 件を却下（付録 B）。これに加え、監査に先立つ構造調査で特定した構造・基盤上の問題 9 件を ST 節に記載する（計 66 件）。以下の「詳細」は検証者による修正済み記述であり、file:line は調査時点の実コードで確認済み。

位置づけ: [milestone 84「Agentチャット安定化」](https://github.com/siro33950/releash/milestone/84) は、本ドキュメントに記載された問題点を逐一解消するマイルストーンとする。

2026-07-15 追補: milestone 84 に Agent 実行設定（Goal / 工数としての model reasoning effort / `Ask・Edit・Plan・Auto・Bypass` の 5 mode）を追加した。この追補は監査済み 66 問題の件数を変更するものではなく、追加 feature scope は [#1445](https://github.com/siro33950/releash/issues/1445)、[#1446](https://github.com/siro33950/releash/issues/1446)、[#1447](https://github.com/siro33950/releash/issues/1447)、[#1448](https://github.com/siro33950/releash/issues/1448)、[#1449](https://github.com/siro33950/releash/issues/1449)、[#1450](https://github.com/siro33950/releash/issues/1450)、[#1451](https://github.com/siro33950/releash/issues/1451) で追跡する。ここで「工数」は model が提供する応答・推論強度の behavioral signal を指し、TokenUsage、cost、時間、turn 数、token / cost / time budget、厳密な上限は含まない。規範仕様は vocabulary V-D10、lifecycle I14〜I16、presentation S9a〜S9c を参照する。

本監査には、wire.rs が想定した contract version と調査時に実際に spawn / 参照した CLI schema version が異なる箇所がある。これは同一 contract の表記揺れではなく runtime schema drift の実例であり、各問題の事実確認時 version はそのまま残す。実装時の解消規約は vocabulary V-D12 の `BackendProtocolIdentity` 照合と `ProtocolIncompatible` fail-closed を正本とする。

2026-07-15 の再確認では、Codex wire contract は `codex-cli 0.139.0` を明記する一方で PATH 上の executable は `0.144.2`、Claude wire contract は Agent SDK `0.3.x` を参照する一方で PATH 上の Claude Code は `2.1.195` だった。この一致を起動時に検証していないことを、D1 / V-D12 で閉じるべき具体的な drift として記録する。

milestone 84 のドキュメント群（本書が要求リスト、以下 3 書が理想形の正本）:

- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md) — 正規化語彙・データ構造の理想形
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) — ライフサイクルの理想形（不変条件）
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md) — UI 表示の理想形

既知・修正済みの問題（#1379 permission 復元、#1381 turn 終端明示化、#1352 信頼性修正一式）は本インベントリの対象外。

## サマリ

| 領域 | 件数 | high | medium | low |
|---|---|---|---|---|
| CL: Claude 入力側の「捨て」 | 7 | 0 | 5 | 2 |
| CX: Codex 入力側の「捨て」 | 11 | 3 | 4 | 4 |
| SD: Claude / Codex で同じ概念の扱いが違う | 7 | 1 | 4 | 2 |
| OB: 送信側（ユーザー → agent）の差・喪失 | 8 | 2 | 3 | 3 |
| RT: runtime 〜 event log 〜 read model 経路の喪失・変質 | 8 | 1 | 4 | 3 |
| FE: frontend の見せ方 | 7 | 2 | 3 | 2 |
| RG: 参照実装（Vibe Kanban / ACP）との語彙ギャップ | 9 | 2 | 4 | 3 |
| ST: 構造・基盤上の問題（前段の構造調査） | 9 | 4 | 4 | 1 |
| **合計** | **66** | **15** | **31** | **20** |

種別（kind）の意味: `dropped` = 捨てている / `divergent` = 扱いが違う / `lossy-lifecycle` = ライフサイクルで失われる / `presentation` = 見せ方 / `other` = その他 / `structural` = 構造要因（ST 節のみ）

## 同一・関連する根本原因の相互参照

複数の視点から同じ根本原因が検出されたもの。解消時は片方の修正でもう片方も解消されるか必ず確認する。

| 問題群 | 根本原因 | 備考 |
|---|---|---|
| CX-3, RG-1（関連: SD-4） | Codex reasoning の不可視 | RG-1 は語彙観点の同一指摘。SD-4 の stall 誤検知は reasoning delta 未購読が一因 |
| CX-5, RG-2（関連: RG-5） | Codex plan/todo の破棄 | RG-5 は todo 語彙の欠落で Claude にも影響する別問題 |
| CX-7, RG-6 | Codex 運用系通知の破棄 | RG-6 は受け皿語彙の欠落まで含む |
| CL-6, RG-7 | tool_result の image 破棄 | 同一根本原因 |
| CL-3, CL-4, RG-3（関連: RG-9） | turn 終了理由・result メタデータの未配線 | stop_reason / subtype / cost の配線欠落クラスタ |
| SD-2, OB-1 | Codex interrupt の信頼性 | 同一根本原因（turn_id 未取得ウィンドウ） |
| OB-3, RT-3 | pending queue の非永続 | 同一根本原因 |
| CL-1, FE-1 | permission 残骸（症状が同型） | CL-1 は Claude の cancel 未処理、FE-1 は表示経路の問題で **別問題** |

## CL: Claude 入力側の「捨て」

Claude CLI (stream-json) が送ってくる情報のうち、Releash が無視・部分変換・情報を落として変換しているもの。

### CL-1: control_cancel_request が未処理（catch-all で無言破棄）で、CLI 側が取り下げた permission request のダイアログが生き続ける

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: 中断直後などに permission ダイアログが残り続け、押しても何も起きない（ツールは実行されないのに履歴上は許可/拒否した決定として残る）。

**詳細**:

Claude Agent SDK 0.3.x の control protocol には CLI→クライアント方向の `control_cancel_request`（保留中の control request＝can_use_tool の取り下げ。sdk.d.ts:2815-2818、stdout メッセージ union sdk.d.ts:6128）が定義され、SDK 参照実装は受信時に保留中の can_use_tool を abort する。Releash には wire.rs に定数がなく（wire.rs:13-27）、convert_claude_message の catch-all `_ => ClaudeConversion::none()`（convert.rs:85）で無言破棄される。リポジトリ全体に処理は存在せず（grep 0件）、ログも出ない（process.rs:177 が warn するのは非JSON行のみ）。read_loop（session.rs:355-380）は convert 以外で生メッセージを消費せず、permission_denied system message（convert.rs:140-159）は tool_use_id キーの別リクエストを生成するだけで代替経路にならない。結果、CLI が interrupt 等で permission request を取り下げても Releash は PermissionRequested を Pending のまま保持し、turn finalize（finalization.rs:34 の Cancelled 畳み込み。interrupt 経路では result 到着か session.rs:493 の synthetic abort timer 約10秒が上限）までダイアログが操作可能なまま残る。その窓でユーザーが Allow/Deny すると、CLI がもう待っていない request_id へ control_response を書き込み（session.rs:233-248、書き込みは成功する）、usecase 層が PermissionResolved を永続化するため（usecase.rs:479-500, 3999）、read model には「Allowed/Denied」という実際には効いていない決定が恒久記録される。pending_inputs のエントリも turn 終端（session.rs:464,486,522,549）まで残留する。ユーザー可視の症状: 中断直後の数秒〜10秒程度、押しても効かない permission ダイアログが残り、その間に押した Allow/Deny は「ツールは実行されていないのに承認/拒否した」という誤った履歴として永続する。ダイアログ自体は turn finalize で Cancelled に自己回復するため無期限には残らない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:84-85`
- `src-tauri/src/infrastructure/agent_session/claude/wire.rs:13-20`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:409-428`

### CL-2: set_model / set_permission_mode / interrupt への control_response（エラー含む）を全て無言破棄し、状態は楽観更新のため UI と CLI 実態が乖離する

- 種別: 扱いが違う（divergent） / 重大度: **medium**

**ユーザー可視の症状**: モデル切替や permission mode 切替が UI 上は成功したように見えるのに、実際は旧モデル・旧モードで応答が続く。失敗の通知が一切出ないため「切り替えたのに挙動が変わらない」不安定さとして見える。

**詳細**:

convert_control_response は request_id が "releash-initialize" 以外の control_response を即 none() で捨て、response の subtype（success/error）を一切見ない（convert.rs:171-184。initialize のエラー応答も slash commands 抽出が空になり none() に落ちて不可視）。一方 session.rs は set_model で stdin 書き込み前に state.model を更新し（session.rs:270-280）、set_permission_mode も stdin 書き込み成功だけで state を更新する（session.rs:250-268。start_turn 内の mode_update 経路 199-207 も同様）。さらに usecase 層（runtime/usecase.rs:768-799）は runtime sync 前に session_store へ永続化し models_updated を UI に通知するため、CLI 側が無効・利用不可モデル等で subtype:"error" を返しても、イベントもログも出ず UI はモデル切替成功を表示し続ける。convert 側の wire_mode 判定（session.rs:359-363 で state から再計算し auto-allow 判断 permission.rs:17-23 に使用）も誤った前提のまま動く。permission mode に限っては system/status の permissionMode → PermissionModeChanged → resync_permission_mode（convert.rs:111-119、usecase.rs:4011-4050）という部分フィードバックがあるが、store 側を真として CLI へ再 push する方向のため CLI がモード変更を拒否したケースの乖離は解消されない。model にはフィードバック経路が一切ない。interrupt のエラー応答も破棄されるが synthetic abort timer（session.rs:214-231, 493-532、10秒）で補われるため実害は緩和される。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:171-184`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:270-280`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:250-268`

### CL-3: result メッセージの subtype を読まず、error_max_turns 等の失敗理由が汎用文言「Claude turn failed」に潰れる（cost/duration/num_turns/permission_denials も全破棄）

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: turn 数上限などで止まった時にチャットに「Claude turn failed」としか出ず、原因（max turns 等）も対処（続行指示）も分からない。コストや所要時間も一切見えない。

**詳細**:

convert_result は is_error と errors[]/result 文字列と usage しか読まず、result メッセージの subtype を一切参照しない（convert.rs:205-234。subtype の消費箇所はリポジトリ全体でゼロ）。SDK 契約では error 系 result（subtype: error_max_turns / error_during_execution）に result フィールドは無く errors も常在しないため、その場合 result_error_text は fallback の "Claude turn failed"（convert.rs:543）になり、この汎用文言がそのまま MessagePart::Error としてチャットに表示され TurnResult::Failed の error にも入る。ただし Releash は claude CLI に --max-turns を渡していない（process.rs の引数構築に存在しない）ため、error_max_turns は実際にはほぼ発生せず、現実に汎用文言へ潰れるのは error_during_execution（CLI 内部エラー）や errors/result を欠く is_error result のケース。加えて total_cost_usd / duration_ms / duration_api_ms / num_turns / permission_denials は全て破棄され（バックエンド・フロントエンド全体で参照ゼロ）、token_usage は modelUsage の inputTokens/outputTokens/cache*/contextWindow のみ抽出し costUSD も読まない（convert.rs:494-530）。結果として、turn 失敗時に原因種別（実行時エラー等）が表示できず、コスト・所要時間・turn 数・permission 拒否履歴も UI に一切出せない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:205-234`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:532-544`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:494-517`

### CL-4: stop_reason を運ぶ message_delta を捨て TurnResult の stop_reason を常に None にするため、workflow の ModelRefusal failure_signal 経路が死んでいる

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: モデルが refusal や max_tokens で打ち切っても通常完了に見える。workflow は失敗シグナルを受け取れず、拒否されたままの成果物で次の goal に進む。

**詳細**:

stream_event_parts は content_block_delta 以外の stream_event（message_start / message_delta / message_stop / content_block_start / content_block_stop）を無言で捨てる（claude/convert.rs:236-240）。stop_reason（refusal 等）は message_delta の delta に載るが、それに加えて stream-json の assistant message にも message.stop_reason が載るものの、assistant_parts（convert.rs:264以降）は content ブロックしか読まない。convert_result も TurnResult::Completed { stop_reason: None } をハードコード（convert.rs:228-231）し、Claude 変換全体で stop_reason を読む箇所が存在しない。Codex 側も codex/convert.rs:210 で常に None。一方ドメインには TurnStopReason::Refusal が存在し（domain/agent_session/entities/turn.rs:21-25、#[allow(dead_code)] コメントで fixture のみの emit を開発側も認識）、projector は stop_reason==Refusal を AgentTurnFailureSignal::ModelRefusal に射影（projector.rs:891, 906-911）、workflow 側は turn_complete.rs:224 経由で failure_policy.rs:142 が ModelRefusal を特別処置する機構をフル実装・テスト済み。さらに event_log/tests.rs:78 はテキスト走査による refusal 検知を行わないことを仕様として固定しており、stop_reason がこの検知の唯一の設計上の入力である。本番経路で Some(TurnStopReason::Refusal) を生成する箇所は皆無（唯一の出現 claude/session.rs:731 は #[cfg(test)] 内のフィクスチャ）のため、workflow の ModelRefusal 失敗検知は本番で一切発火しない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:238-241`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:228-231`
- `src-tauri/src/usecase/agent_session/event_log/projector.rs:906-911`
- `src-tauri/src/domain/agent_session/entities/turn.rs:21-25`

### CL-5: system/init から session_id と slash_commands 以外を全破棄し、MCP サーバの接続失敗（mcp_servers.status）が不可視

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: MCP サーバが落ちている/認証切れでも UI に何も出ず、agent が突然 MCP ツールを使わなくなった・「そのツールは無い」と言い出す、という形でしか観測できない。

**詳細**:

convert_system_message の init 分岐（convert.rs:89-109）は session_id を SessionEstablished に、commands/slash_commands を SlashCommandsUpdated に変換するのみで、init に含まれる mcp_servers（name/status: failed・needs_auth 等）、tools 一覧、実効 model、permissionMode、output_style を全て破棄する。AgentRuntimeEvent（gateway.rs:55-75）にこれらを運ぶ語彙はなく、session.rs の read_loop も convert 経由のみのため別経路も存在しない。durable event・frontend にも MCP サーバ状態の表現はない（ActivityLog.tsx の mcp__* は tool 名分類のみ）。補足: permissionMode は system/status メッセージ経由（convert.rs:111-119）でのみ反映されるため、init 時点の実効値は失われる。MCP サーバ起動失敗・認証切れの backend からの通知経路が存在せず、ユーザーには「agent が MCP ツールを使わなくなった/無いと言い出す」という形でしか観測できない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:91-109`
- `src-tauri/src/domain/agent_session/gateway.rs:55-75`

### CL-6: tool_result の content 配列から text 項目だけを連結し、image 等の非 text 項目を無言破棄する（image のみの結果は空文字になる）

- 種別: 捨てている（dropped） / 重大度: **low**

**ユーザー可視の症状**: 画像を返すツール（画像ファイルの Read、スクリーンショット系 MCP ツール等）の結果ブロックが空欄で表示され、ツールが失敗したように見える。

**詳細**:

tool_result_content は content が配列の場合 item.get("text")（または文字列 item）だけを filter_map で残して join する（convert.rs:464-479）。{type:"image", source:...} などの非 text 項目は数も痕跡も残らず、画像のみの tool_result は content=="" の ToolResult part になる（convert.rs:322-332）。is_error は保持される。MessagePart には Image バリアントが存在する（message_part.rs:54-59）が #[allow(dead_code)] で、この経路では使われない。変換経路は claude/session.rs:364 の convert_claude_message 一本のみで raw JSON の別保存経路はなく、durable event log にも変換済み parts しか残らないため画像データは変換時点で恒久的に失われる。UI 上は is_error=false のため成功アイコン（CheckCircle2, ActivityLog.tsx:186）付きで出力が空欄のブロックとして表示される（明示的なエラー表示にはならないが、ユーザーには結果が空に見える）。なお修正時は frontend 側も要対応: ChatSessionView.tsx:592-594 は image/image_ref part を null レンダリングしている。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:464-479`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:322-332`
- `src-tauri/src/domain/agent_session/entities/message_part.rs:54-59`

### CL-7: wire mode "plan" が PermissionModeChanged に変換できず（None で破棄）、CLI 主導の plan mode 遷移が UI・内部状態に同期されない

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: EnterPlanMode を承認した後も UI の plan トグルは OFF のままで、agent が「計画だけして編集しない」状態が続く。モード表示と実挙動が食い違う。

**詳細**:

permission_mode_from_wire は "plan" を None にする（wire.rs:71-79）ため、system/status の permissionMode:"plan" 通知は convert.rs:111-119 でイベント化されず無言に落ちる。EnterPlanMode ツールは interactive 扱いで承認ダイアログが出る（permission.rs:25-30、Full/BypassPermissions では permission.rs:17-23 で auto-allow）が、承認を plan_mode に反映する処理は backend にも frontend にも存在せず（AgentRuntimeEvent に plan 遷移を表す variant もない: gateway.rs:65）、CLI が plan mode に入っても Releash の state.plan_mode は false のまま。次の start_turn では prepare_start_turn_state が Releash 側 state 由来の stale な wire mode 同士を比較して set_permission_mode を送らない（claude/session.rs:570-595）ため、不整合が turn を跨いで継続する。なお Releash は resync_permission_mode（runtime/usecase.rs:4011-4050）で CLI 報告モードを保存済みモードへ押し戻す revert 設計を持つが、"plan" はイベント自体が破棄されるため revert も発火せず、CLI 主導遷移が同期も是正もされない唯一のモードになっている。回復にはユーザーが plan トグルを手動 ON→OFF するか permission mode を変更する必要がある。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/wire.rs:71-79`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:112-119`
- `src-tauri/src/infrastructure/agent_session/claude/permission.rs:17-30`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:575-595`

## CX: Codex 入力側の「捨て」

codex app-server (JSON-RPC) が送ってくる情報のうち、Releash が無視・部分変換・情報を落として変換しているもの。

### CX-1: requestUserInput の question id を捨てるため、ユーザーの回答が Codex 側で全部破棄される

- 種別: 扱いが違う（divergent） / 重大度: **high**

**ユーザー可視の症状**: Codex が質問ダイアログを出し、ユーザーが選択肢を選んで回答しても、agent は「無回答」として扱われる。ユーザーの指示（選択）が黙って無視され、agent が勝手な前提で進む・再質問する。

**詳細**:

codex 0.139.0 の item/tool/requestUserInput params は questions[].{id, header, question, isOther, isSecret, options} で、応答は { answers: { <question.id>: { answers: [String] } } } を要求する（openai/codex rust-v0.139.0 app-server-protocol/src/protocol/v2/item.rs: ToolRequestUserInputQuestion に pub id: String、ToolRequestUserInputResponse.answers は HashMap<String, ToolRequestUserInputAnswer>、ToolRequestUserInputAnswer { answers: Vec<String> }）。Releash の question_from_value（codex/convert.rs:615-656）は question/prompt・header・options・multiSelect しか読まず id・isOther・isSecret を捨てる。domain の PermissionQuestion（domain/agent_session/entities/permission_request.rs:41-46）にも id フィールドがなく復元不能。フロントは answers を「質問文」キーの Record<string,string> で構築し（PermissionDialog.tsx:733-743、multi-select は ", " join で単一文字列化）、ChatSessionView.tsx:420-425 → respond_agent_permission → split_updated_input_and_answers（adaptor/controller/command/agent_session/permission.rs:338-351）→ runtime（usecase.rs:497-500）→ codex/session.rs:253 → codex/permission.rs:106-114 まで無加工パススルーで result.answers として返す。codex 側は serde_json::from_value::<ToolRequestUserInputResponse> 失敗時に answers: HashMap::new() へフォールバックする（bespoke_event_handling.rs の on_request_user_input_response）。値が文字列で {answers:[String]} オブジェクトでないため deserialize は必ず失敗し（キーの質問文/id 不一致は deserialize 自体は通すが、値形式不一致で全体が失敗）、ユーザーの回答は常に空として Codex core に渡る。仮に値形式を直してもキーが id でないため id lookup が全 miss する二重不一致。加えて isSecret 落ちにより秘匿入力指定が UI に伝わらず平文入力表示になり、multi-select の ", " join は Vec<String> 形式の複数回答表現とも非互換。なお codex/permission.rs:218-226 のテストは誤った wire 形式（フラット文字列 map の素通し）を現行挙動として固定しており、wire.rs の 0.139.0 検証ノート（21-22行）にレスポンス形式の検証記録はない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:615-656 (question_from_value: id/isOther/isSecret を読まない)`
- `src-tauri/src/infrastructure/agent_session/codex/permission.rs:106-114 (answers を素通しで result.answers に)`
- `src/components/panels/AgentChatPanel/ChatSessionView.tsx:420-425 / PermissionDialog.tsx:724 (質問文キーの Record<string,string>)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/item.rs:1418-1454 (id 必須・answers は HashMap<id,{answers:[String]}>)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server/src/bespoke_event_handling.rs:1673-1678 (deserialize 失敗時は空 answers にフォールバック)`

### CX-2: mcpServer/elicitation/request を無視かつ無応答のため、MCP elicitation 発生時に turn が不可視のままハングする

- 種別: 捨てている（dropped） / 重大度: **high**

**ユーザー可視の症状**: codex の設定に elicitation を使う MCP server がある場合、その tool 呼び出しで turn が永久に止まる。UI には何のダイアログも出ず、agent が理由なく無限に「実行中」に見える（stale 検知が出るだけ）。

**詳細**:

codex 0.139.0 の server request には mcpServer/elicitation/request がある（app-server-protocol/src/protocol/common.rs の server_request_definitions!、~1400行付近）。MCP server が elicitation を発行すると app-server は capability gate なしで無条件にこの request をクライアントへ送り、spawn したタスクが oneshot receiver.await で応答を待ち、応答受領後に Op::ResolveElicitation を core へ submit する（app-server/src/bespoke_event_handling.rs 送信 ~1150-1180、await と fallback ~1238-1282。timeout なし、デフォルト decline は sender drop 時のみ）。Releash は permission_request_from_server_request の match（convert.rs:547-575）が 4 メソッド（commandExecution/fileChange/permissions の requestApproval と item/tool/requestUserInput）のみで、それ以外は _ => return None（575行）となり convert_server_request（convert.rs:230-235）はイベントを一切出さない。JSON-RPC 応答の書き込み経路は session.rs:239-258 の respond_permission（UI の permission 解決経由）のみで、app_server.rs にも method-not-found 等の汎用自動応答は存在しない。session.rs:355-358 で pending_methods には積まれるが誰も応答せず、codex 側の receiver.await が永久に解決しないため ResolveElicitation が submit されず、MCP server の elicitation が未解決のまま tool call がブロックし turn が完了しない。既知問題1（permission 不可視停止）と同型だが、こちらは表示経路が最初から存在しない完全なハングで、ユーザーの脱出手段は手動 interrupt のみ。なお同じ _ => None により item/tool/call（DynamicToolCall）等の他の server request も無応答だが、これらはクライアント側が dynamic tool 登録等をしない限り送信されないため、実害が生じる現実的な経路は elicitation。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:547-575 (4 メソッド以外の server request は None)`
- `src-tauri/src/infrastructure/agent_session/codex/wire.rs:63-66 (REQUEST_* 定数に elicitation なし)`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:355-358 (server request を pending_methods に積むだけで応答経路なし)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/common.rs:1394-1398 (McpServerElicitationRequest 定義)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server/src/bespoke_event_handling.rs:705-760 (無条件でクライアントに送信し応答を待つ)`

### CX-3: Codex の reasoning（thinking）が完全に不可視：delta を購読せず、completed item も存在しないフィールドを参照

- 種別: 捨てている（dropped） / 重大度: **high**

**ユーザー可視の症状**: Codex（特に高 reasoning effort）で agent が考えている数十秒〜数分間、チャットに何も表示されず固まって見える。事後も thinking ブロックが一切残らず、Claude セッションとの見え方が大きく食い違う。

**詳細**:

Codex 0.139.0 の reasoning item は { type:"reasoning", id, summary: Vec<String>, content: Vec<String> }（openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/item.rs、tag="type" rename_all="camelCase"、summary/content とも #[serde(default)] の文字列配列）。convert.rs:347-354 の item/completed 変換は get_string(item,["text"]) と get_string(item,["summary","text"]) を読むが、text フィールドは存在せず、summary は配列なので ["summary","text"] も常に None → MessagePart::Thinking が一度も生成されない。streaming 側も、0.139.0 が発行する item/reasoning/summaryTextDelta / item/reasoning/summaryPartAdded / item/reasoning/textDelta（common.rs 1549-1553 付近）は wire.rs:50-61 に定数すら無く、convert_notification の `_ => Vec::new()`（convert.rs:216）で無言破棄される。item/started の reasoning も item_tool_name（convert.rs:392-406）が None を返し無表示。durable event の ReasoningRecorded（event_log/part_events.rs:36）は Thinking part からのみ生成されるため、Codex セッションでは永続記録にも一切残らない。Claude 側は claude/convert.rs:254 で Thinking part を生成しており非対称。ユーザー可視症状: Codex（特に高 reasoning effort）で agent が推論している間、汎用の "Thinking..." shimmer（deriveActivityStatus.ts:17）以外の実内容が数十秒〜数分間何も表示されず、事後のトランスクリプトにも thinking ブロックが一切残らない。完全な無反応に見えるわけではない（プレースホルダは出る）が、reasoning 内容はストリーミング中も完了後も完全に不可視で、Claude セッションと見え方が大きく食い違う。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:347-354 (存在しない text / summary.text を参照)`
- `src-tauri/src/infrastructure/agent_session/codex/wire.rs:50-61 (item/reasoning/* の購読なし)`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:216 (未知 notification は無言で破棄)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/item.rs (Reasoning { summary: Vec<String>, content: Vec<String> })`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/common.rs:1573-1575 (reasoning delta notification 3種)`

### CX-4: thread/tokenUsage/updated のフィールド名不一致で Codex の token usage が常に 0 になる

- 種別: 扱いが違う（divergent） / 重大度: **medium**

**ユーザー可視の症状**: Codex セッションの token 使用量・コンテキスト残量が常に 0 と表示・記録される。workflow run のトークン集計も Codex step 分だけ常に 0 で、コスト・コンテキスト逼迫の判断材料が失われる。

**詳細**:

codex 0.139.0（wire.rs:1-2 の検証対象バージョン）の thread/tokenUsage/updated params は { threadId, turnId, tokenUsage: { total: {totalTokens, inputTokens, cachedInputTokens, outputTokens, reasoningOutputTokens}, last: {...}, modelContextWindow } }（app-server-protocol/src/protocol/v2/thread.rs:1249-1301、emit 箇所は app-server/src/bespoke_event_handling.rs:1590-1603、experimental gate なし）。token_usage_from_value（convert.rs:658-685）は params.usage（実際のキーは tokenUsage）または params 直下の inputTokens/outputTokens/totalTokens/contextWindowTokens を読むため全て miss し、input=0/output=0/total=Some(0)/context_window=None を生成する。この値が TokenUsageUpdated として emit され（convert.rs:168-171）、state.latest_usage 経由で TurnCompleted の token_usage にも入る（convert.rs:201, 211）。convert.rs:1026-1064 のテストは実プロトコルに存在しない top-level 形状を前提に挙動を固定している。runtime は usecase.rs:2499-2508 で read model（latest_token_usage）へ、usecase.rs:3183-3194 と event_log/projector.rs:883-893 経由で usecase.rs:4181-4184 の workflow turn record へゼロ値を伝播する。ユーザー可視症状の正確な範囲: 現行フロントエンドには token usage を描画するコンポーネントがまだ存在しない（useAgentChat.ts:1545 の getSessionLatestTokenUsage は未消費、types/workflow.ts:217 の totalTokenUsage も未レンダリング）ため「UI に 0 と表示される」は現時点では観測されない。実害は、Codex セッションの token 使用量が durable event log（TurnCompleted token_usage）・session read model・workflow turn record / workflow run の total_token_usage 集計・WS/presenter 配信（adaptor/presenter/agent_session.rs:171）に恒久的に 0/0/Some(0) として記録・配信されることで、コスト・コンテキスト逼迫の判断材料が Codex 分だけ欠落し、将来 UI や remote client が表示を実装しても過去データ含め常に 0 になる。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:658-685 (usage / top-level 参照でフィールド不一致)`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:1029-1035 (テストが実在しない wire 形状を前提)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/thread.rs:1249-1290 (tokenUsage.total/last/modelContextWindow 形状)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2499-2508, 4181-4184 (ゼロ値が read model と workflow turn 記録に伝播)`

### CX-5: turn/plan/updated（および plan item）を捨てるため Codex の plan/todo リストが一切表示されない

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: Codex が内部で立てた plan/todo（何をどの順でやるか、今どのステップか）がチャットに全く出ない。Claude セッションでは todo が見えるのに Codex では進行状況を観測できず、承認判断の材料が欠ける。

**詳細**:

codex app-server は update_plan（todo/checklist ツール）の実行を turn/plan/updated notification（{ threadId, turnId, explanation, plan: [{step, status}] }）で通知する。upstream rust-v0.139.0 では common.rs:1539 で experimental 属性なしの第一級 notification として定義され、bespoke_event_handling.rs:1278-1294 で EventMsg::PlanUpdate から無条件送信される（upstream コメント「update_plan is a todo/checklist tool」）。しかし Releash の wire.rs には対応定数がなく、convert.rs:216 の convert_notification の `_ => Vec::new()` で破棄される。convert_jsonrpc_message（codex/session.rs:367）が唯一の変換経路のため、別経路での処理・durable 記録もない。experimental な ThreadItem::Plan（v2/item.rs:238-240）と item/plan/delta も item_started_parts/item_completed_parts（convert.rs:336/388）の match 外で破棄される（Releash は experimentalApi:true で initialize するため届き得る）。これは refactor #1301（e115f565a）で導入されたリグレッション: 削除された旧 codex_app_server.rs は todoList item を todo_list_snapshot message に変換しており（旧ファイル 263/290/525 行）、旧統合では Codex の todo が表示されていた。wire.rs:16-20 の 0.139.0 検証ノートは「TurnItem から todo_list が消えた」ことのみ記録し、代替チャネルが turn/plan/updated notification であることを見落としている。domain には MessagePart::TodoListSnapshot（message_part.rs:44-46）があり、Claude の TodoWrite は claude/convert.rs:279-286 で変換され ChatSessionView.tsx:207/1324 で backend 非依存に描画されるため、backend 間の非対称がそのまま UI に出る。修正は wire.rs に NOTIFY_TURN_PLAN_UPDATED 定数を追加し、convert_notification で plan 配列を MessagePart::TodoListSnapshot に変換するだけで、既存の merge（message_part.rs:169-176）・durable 記録（part_events.rs:168-171 の TodoListSnapshotRecorded）・frontend 描画がそのまま機能する。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:216 (未購読 notification の破棄点)`
- `src-tauri/src/infrastructure/agent_session/codex/wire.rs:50-61 (turn/plan/updated・item/plan/delta の定数なし)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/turn.rs:392-414 (TurnPlanUpdatedNotification)`
- `src-tauri/src/domain/agent_session/entities/message_part.rs:44-46 (TodoListSnapshot は存在し Claude では使用)`

### CX-6: turn/start 以外の JSON-RPC error response を warn ログだけで握り潰す（permission mode 変更の失敗が UI 上「成功」のまま）

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: permission mode を厳しく切り替えたつもりでも codex 側で失敗していると旧設定（例: full auto 相当）のままコマンドが自動実行される。逆に Stop（turn/interrupt）が失敗しても無反応で「止まらないのに何も言わない」状態になる。

**詳細**:

convert_response は client_response_methods に載っていない id の error response を log::warn のみで捨てる（convert.rs:83-87）。pending_client_methods に登録されるのは turn/start だけ（session.rs:192-194、read_loop はこの map 経由でしか client_response_methods を埋めない: session.rs:360-363）。そのため thread/settings/update（permission mode / plan mode 切替、session.rs:260-296）、thread/name/set（session.rs:303-319）、turn/interrupt（session.rs:214-237）の JSON-RPC error response は全て不可視。convert.rs:1111 のテストがこの無イベント挙動を意図的に固定している。さらに runtime usecase は set_permission_mode で store 更新と permission_mode_changed 通知を codex への同期より先に行い、同期の write 失敗も warn-only（usecase.rs:715-740）。ただし影響範囲は限定的: build_turn_start_request が毎 turn/start で store 由来の permission settings を再送する（session.rs:518-537）ため、settings/update 失敗による「UI は新モード表示・codex は旧 approval policy / sandbox」の不一致は現在実行中の turn の残り区間のみで、次 turn 開始時に解消される。それでも「実行中の agent を見て mode を厳格化した」まさにその turn 中に silent fail し、旧設定（例: full auto 相当）でコマンドが自動実行され続ける。turn/interrupt は write 失敗こそ usecase.rs:474 で伝播するが、codex が error response を返した場合は無反応のまま turn が続行し「Stop を押したのに止まらず何も言わない」状態になる。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:83-87 (untracked error response は warn のみ)`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:192-194 (追跡対象は turn/start のみ)`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:260-296 (thread/settings/update を無追跡で送信)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:715-740 (同期前に永続化・UI通知する楽観更新)`

### CX-7: warning / configWarning / guardianWarning / deprecationNotice / model/rerouted を全て破棄し、codex がユーザー向けに明示した警告が届かない

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: モデルが黙って別モデルに差し替えられても、config の不備で MCP server が読み込めなくても、チャットには何も出ない。「なぜか挙動が違う／tool が見えない」原因をユーザーが知る手段がない。

**詳細**:

codex app-server 0.139.0 は ServerNotification として warning（"Concise warning message for the user"）、guardianWarning、deprecationNotice、configWarning、model/rerouted を定義する（common.rs の ServerNotification enum、v2/notification.rs:8-29 に payload 定義）。Releash は experimentalApi: true で v2 プロトコルを使用しており、これらは実際に受信しうる wire contract の一部。しかし wire.rs:50-61 にはこれらの定数がなく、convert.rs の convert_notification は末尾の `_ => Vec::new()`（convert.rs:216）で破棄する。この破棄は log::warn すら出ない完全サイレントで（未処理エラーレスポンスは convert.rs:83 で log される対比）、session.rs:367 の read_loop は convert 結果が空だと events_tx に何も送らないため、runtime・durable event log・frontend のいずれにも痕跡が残らない。兄弟 notification の "error" は MessagePart::Error に変換される（convert.rs:173-178）ため、warning 系のみの欠落。特に model/rerouted はユーザー指定モデルと別モデルで実行された通知、configWarning は ~/.codex/config.toml の不備（MCP server 設定ミス等）の通知であり、破棄されるとユーザーは挙動差の原因を知る手段がない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/wire.rs:50-61 (warning 系 notification の定数なし)`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:216 (破棄点)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/common.rs:1578-1586 (Warning/GuardianWarning/DeprecationNotice/ConfigWarning/ModelRerouted 定義)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/notification.rs:18-36 ("Concise warning message for the user")`

### CX-8: error notification の willRetry を無視し、自動リトライされる一時エラーを恒久エラーとして transcript に刻む

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: 一時的なネットワーク断などで codex が自動リトライして正常完了した turn でも、チャット途中に赤エラーが恒久的に残り、「失敗したのか成功したのか分からない」不安定な見え方になる。

**詳細**:

ErrorNotification は params トップレベルに will_retry: bool（camelCase 直列化で willRetry）を持ち、「true なら transient で app-server が自動リトライし turn は中断されない」と upstream doc に明記される（openai/codex rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/notification.rs）。convert.rs:173-178 は willRetry を読まず error.message のみで常に MessagePart::Error を merge する。Error part は merge 時の重複排除のみで除去手段がなく（message_part.rs:213-232、projector.rs:572-592 の push_unique_error も同様）、part_events.rs:43-53 で ErrorRecorded として durable event にも記録されるため、リトライ成功後も AgentErrorBlock（ChatSessionView.tsx:542-547）として赤エラーが transcript に恒久的に残る。convert.rs:967 のテストフィクスチャは willRetry: false を含むが変換側はフィールドを読んでおらず、willRetry=true の挙動を意図的仕様として固定するテストは存在しない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:173-178 (willRetry 無視で常に Error part 化)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/notification.rs:41-48 (will_retry=true は turn を中断しない transient)`
- `src-tauri/src/domain/agent_session/entities/message_part.rs:213-232 (Error part は取り消し不能)`

### CX-9: slash command 変換パスが 0.139.0 では dead code：initialize response に commands フィールドは存在せず、Codex のコマンド補完が常に空

- 種別: その他（other） / 重大度: **low**

**ユーザー可視の症状**: Codex セッションで "/" を打っても codex 側のコマンド（custom prompts 等）が一切補完に出ない。Claude セッションではコマンドが並ぶため、Codex だけ壊れているように見える。

**詳細**:

convert_response は result.commands / result.slashCommands から SlashCommandsUpdated を作る（convert.rs:97-100, 687-715、テスト convert.rs:1151-1175）が、codex rust-v0.139.0 の app-server protocol には commands/slashCommands を含む response が一切存在しない（v1::InitializeResponse は userAgent/codexHome/platformFamily/platformOs のみ、v2 の ThreadStartResponse/ThreadResumeResponse/TurnStartResponse/ThreadSettingsUpdateResponse/ThreadSetNameResponse/TurnInterruptResponse にも commands なし、protocol crate 全体で SlashCommand 系フィールドはゼロ件）。Releash がセッション内で送る request は initialize・thread/start・thread/resume・turn/start・turn/interrupt・thread/settings/update・thread/name/set のみ（session.rs:121-157 ほか）であり、このイベントは Codex では一度も発火しない。補足: thread/start・resume の response は convert.rs:90-96 の result.thread.id early return で SessionEstablished に変換されるため、仮に commands が同居しても抽出パスに到達しない。また 0.139.0 プロトコルには custom prompts / slash command を列挙する method 自体が存在しないため、修正は response 待ちでは成立せず、skills/list + local scan（models.rs:125-151 の skill_catalog と同様）のような別ソースが必要。frontend は runtimeSlashCommands のみで built-in fallback を持たない（MessageInput.tsx:137-146）ため、Codex セッションの "/" 補完は常に空になる。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:97-100, 687-715 (commands/slashCommands 抽出パス)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v1.rs:61-71 (InitializeResponse に commands なし)`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:121-157 (セッション内で送る request の一覧)`

### CX-10: imageGeneration / imageView / collabAgentToolCall / enteredReviewMode / exitedReviewMode / hookPrompt の item が started/completed とも無表示

- 種別: 捨てている（dropped） / 重大度: **low**

**ユーザー可視の症状**: agent が画像を生成・閲覧したり、review mode に入ったり、collab sub-agent を起動しても、その間チャットは無反応（沈黙）に見える。何をしていたのかが event log にも残らない。

**詳細**:

0.139.0 の ThreadItem には userMessage/hookPrompt/agentMessage/plan/reasoning/commandExecution/fileChange/mcpToolCall/dynamicToolCall/collabAgentToolCall/webSearch/imageView/imageGeneration/enteredReviewMode/exitedReviewMode/contextCompaction の16 variant がある（v2/item.rs:212-364）。item_started_parts（convert.rs:305-339）は fileChange を 310 で処理し、それ以外は item_tool_name（convert.rs:392-407: commandExecution/webSearch/mcpToolCall/dynamicToolCall のみ Some）が None を返す型を 317-318 の early return で捨てる。item_completed_parts（convert.rs:341-390）は reasoning/webSearch/commandExecution/fileChange/mcpToolCall/dynamicToolCall 以外を 388 の `_ => Vec::new()` で捨てる。contextCompaction のみ convert.rs:135-153 で SystemNotification 化され、agentMessage の本文は item/agentMessage/delta（convert.rs:125-132）で別途届く。よって hookPrompt/collabAgentToolCall/imageView/imageGeneration/enteredReviewMode/exitedReviewMode（および plan）は started/completed とも MessagePart を生成せず、session.rs:367 の read_loop は変換結果のみ forward するため transcript にも durable event log にも痕跡が残らない。特に /review は SlashCommandsUpdated 経由でユーザーに露出している（convert.rs:1151-1175 のテストが固定）ため、exitedReviewMode の review ペイロード（review 結果テキスト）が drop される経路はユーザーが実際に踏める。review 結果が agentMessage 側でも重複配信されない場合、/review の出力自体が不可視になり severity は medium 相当に上がる（この重複有無は未確認）。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:305-339 (item_started_parts の対応表と 336 のフォールバック)`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:341-390 (item_completed_parts の対応表と 388 のフォールバック)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/item.rs:212-364 (ThreadItem 全 variant)`

### CX-11: webSearch の完了 item から query 更新と結果（action）を捨て、固定文字列 "Web search completed." に置換

- 種別: 捨てている（dropped） / 重大度: **low**

**ユーザー可視の症状**: Codex が何を検索してどんな結果を得たのかチャットから全く分からず、tool 行に「Web search completed.」だけが並ぶ。検索根拠の監査ができない。

**詳細**:

Codex app-server v2 の WebSearch item は item/started 時点では query が空文字（upstream テストで確認: codex-rs/app-server/tests/suite/v2/web_search.rs @ rust-v0.139.0）、item/completed で確定 query と action（WebSearchAction::Search { query, queries } 等のクエリ群/種別）を持つ。しかし item_completed_parts（convert.rs:355-359）は tool_result_part(item_id, "Web search completed.", false) を返すだけで query/action を両方捨て、item_started_parts（convert.rs:331）が作った空 query の ToolUse input が更新されないまま残る。item/updated 通知は購読しておらず（wire.rs:50-61 に定数なし、convert.rs:216 で未知 method を破棄）、completed 時の ToolUse 再 emit もないため、確定クエリが UI に届く経路は存在しない。tool presentation（tool_activity.rs:48,70）は空 query を filter するため tool 行は "Explored (WebSearch)" と表示される。結果、ユーザーは Codex が何を検索したのかチャットから一切分からない。Claude 統合では tool_result content が汎用パススルー（claude/convert.rs:464-479）で検索クエリ・結果が表示されるのと非対称。なお action が持つのはクエリ群であり検索結果の内容は protocol 上元々含まれないため、失われるのは「実行された検索クエリ」の監査可能性である。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:355-359 (固定プレースホルダ)`
- `openai/codex@rust-v0.139.0 codex-rs/app-server-protocol/src/protocol/v2/item.rs:337-345 (WebSearch { query, action })`

## SD: Claude / Codex で同じ概念の扱いが違う

同じ意味の事象が backend によって異なるイベント・タイミング・信頼性で扱われるもの。

### SD-1: resume 失敗の回復経路: Claude は自動復旧、Codex は恒久的にセッションが死ぬ

- 種別: 扱いが違う（divergent） / 重大度: **high**

**ユーザー可視の症状**: Codex セッションの backend thread が消えた場合（codex home 変更・thread GC 等）、以後メッセージを送るたびにセッションが Error 状態になり、チャットにエラー説明も出ず（log warn のみ）、二度と復活しない。同じ状況の Claude セッションは何事もなかったかのように続行する（ただし文脈は静かに消える）。

**詳細**:

resume 失敗の回復経路が backend 間で非対称。Claude は resume 失敗（CLI が別 session_id で init を返す）を ResumeOutcome::Mismatch（claude/convert.rs:94-100）として event pump に流し、runtime 側 handle_resume_mismatch（runtime/usecase.rs:2083-2133）が runtime を閉じ、実行中 turn を pending_queue に戻し、resume メタデータを消去して新規 backend セッションで自動再開する（Claude の open は init を待たず即座に返るため、Mismatch は必ず take_events 後の pump に届く。claude/session.rs:75-97）。Codex は thread/resume のエラー応答を BackendSessionCleared + Fatal に変換する（codex/convert.rs:58-67）が、これは open() が wait_for_thread_id（codex/session.rs:441-464）の startup_error で失敗する起動フェーズでのみ発生し、events receiver は open 成功後の take_events（runtime/usecase.rs:2048）でしか読まれないため、BackendSessionCleared は受信者のいないチャネルごと drop される。これを処理して agent_session_id を消去するはずの runtime/usecase.rs:2420-2438 には決して届かない（gateway.rs:60-62 の #[allow(dead_code)] コメント自体が配線未完了を明記）。open 失敗の呼び出し元は 2 経路とも agent_session_id を消去しない: 送信起点の ensure_runtime 経路（runtime/usecase.rs:1462-1515）は TurnInterrupted{error} を event log に追記して SessionState::Error にし、queued turn 再オープン経路（runtime/usecase.rs:3362-3389）は log warn + SessionState::Error のみ。update_resume_metadata_if_changed の production 呼び出しは usecase.rs:2101/2402/2421 の 3 箇所だけで、いずれもこのシナリオでは到達不能。その結果、Codex は死んだ thread id（usecase.rs:2040 の session.agent_session_id）への resume を送信のたびに繰り返して恒久的に失敗し続ける。ユーザーに見える症状: Codex backend の thread が消えた場合（codex home 変更・rollout ファイル削除等）、以後メッセージを送るたびにセッションが Error 状態になり、チャットには Codex の生 JSON-RPC エラー文言（例: not found）が error part として毎回表示されるだけで（projector.rs:389-399）、復旧手段の提示も自動回復もなく二度と会話を継続できない。queued turn の再オープン経路ではエラー表示すらなく log warn のみ。同じ状況の Claude セッションは文脈を失いつつも自動的に新規セッションで続行する。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:59-67`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:60-78`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:441-464`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2047`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3362-3389`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2420-2438`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:94-104`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2083-2133`
- `src-tauri/src/domain/agent_session/gateway.rs:60-62`

### SD-2: Stop（interrupt）の信頼性: Codex は turn/started 前の Stop を無言で握りつぶし、フォールバックも無い

- 種別: 扱いが違う（divergent） / 重大度: **medium**

**ユーザー可視の症状**: Codex で送信直後（誤送信に気づいた直後など）に Stop を押しても agent は止まらずストリーミングを続け、UI は「停止中」のまま turn が自然終了するまで固まる。Claude では同じ操作が確実に（最悪 10 秒で）turn を終端する。

**詳細**:

Claude では interrupt()（claude/session.rs:214-231）が常に interrupt control request を CLI に書き込み、さらに 10 秒の ABORT_SYNTHESIS_DELAY タイマー（claude/session.rs:29-32, 493-532）が backend 無応答時に TurnCompleted(Interrupted) を合成するため、Stop は最悪 10 秒で turn を終端する。Codex では interrupt()（codex/session.rs:214-237）が state.thread_id と state.turn_id の両方を要求し、どちらかが未設定だと何も送らずに Ok(()) を返す。turn_id は app-server の turn/started 通知でのみ設定される（codex/convert.rs:119-124）ため、「Releash が turn/start を書き込んでから turn/started 通知を処理するまで」の窓で Stop が無言で握りつぶされる。なお TurnPhase::Streaming は start_turn 成功後にのみ emit される（runtime/usecase.rs:1535-1558, 3574-3590）ため、起動待ち中（thread_id 未設定）は Stop ボタン自体が表示されず UI からは到達不能。実質的な窓は turn/start 送信後〜turn/started 受信処理までで、通常はサブ秒だが app-server の負荷や resume 時には伸びうる。さらに interrupt() は request id を pending_client_methods に登録しない（start_turn は session.rs:192-195 で登録する）ため、turn/interrupt のエラー応答は convert_response の未追跡分岐（codex/convert.rs:83-87）で log のみで捨てられ、拒否と成功を区別できない。合成 abort タイマーも存在しない（プロセス死亡時の read_loop 合成のみ、session.rs:393-425）。usecase 層（runtime/usecase.rs:466-477）にもフォールバックは無い。frontend は Stop 押下時に楽観的に interrupting=true を立て（useAgentChat.ts:761-773）、invoke は backend が Ok を返すため成功扱いとなり、reducer は turnPhase が idle になるまでフラグを保持（agentChatReducer.ts:469-484）、Stop ボタンは disabled={isInterrupting}（MessageInput.tsx:805）になるため、一度握りつぶされると turn が自然終了するまで Stop を再送する手段が UI に存在しない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/session.rs:214-237`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:119-124`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:83-87`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:214-231`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:29-32`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:493-532`
- `src/hooks/useAgentChat.ts:761-771`

### SD-3: backend stdout の頑健性: Claude は非 JSON 行 skip・8MB 超破棄、Codex は非 JSON 行 1 行でセッション即死・サイズ上限なし

- 種別: 扱いが違う（divergent） / 重大度: **medium**

**ユーザー可視の症状**: codex CLI（や注入された環境）が stdout に警告等の非 JSON 行を 1 行でも出すと、Codex チャットが「invalid app-server JSON-RPC」の Fatal で突然死し turn は Crash 扱いになる。巨大なツール出力ではメモリ肥大・UI フリーズの恐れ。Claude では同じ事象は無害または 1 件破棄の通知で済む。

**詳細**:

Claude では stdout の非 JSON 行を warn ログで skip して読み続け（claude/process.rs:174-179）、8MB 超の行はバッファに保持せず読み捨てて OversizeDropped とし（claude/process.rs:22, 204-266）、チャットに Error part「backend からの応答 1 件がサイズ上限（8MB）を超えたため破棄しました」を出した上でセッションを継続する（claude/session.rs:345-353, 400-407）。この skip/oversize 挙動はテストで意図的仕様として固定されている（claude/process.rs:451-503）。Codex では decode_jsonrpc_line が非 JSON 行を Err「invalid app-server JSON-RPC」にし（codex/app_server.rs:130-132）、next_json がそのまま伝播（codex/app_server.rs:107-117）、read_loop の Err 分岐が turn 実行中なら TurnCompleted(Interrupted{Crash}) を送り、常に Fatal を送って break → process.shutdown() で app-server を kill する（codex/session.rs:411-428）。runtime 側は Fatal で turn を Crash として complete し runtime を close するためセッション実体が終了する（runtime/usecase.rs:2536-2564）。また Codex の stdout 読み取りは BufReader::lines()（codex/app_server.rs:99）でサイズ上限がなく、巨大な item/completed（aggregatedOutput 等）の行を丸ごとメモリに蓄積する。結果として、同じ「プロトコル外の 1 行」という事象が Claude では無害な警告または 1 件破棄の通知、Codex では致命的クラッシュ（turn=Crash、セッション終了）になる非対称がある。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/app_server.rs:107-117`
- `src-tauri/src/infrastructure/agent_session/codex/app_server.rs:130-132`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:411-424`
- `src-tauri/src/infrastructure/agent_session/claude/process.rs:22`
- `src-tauri/src/infrastructure/agent_session/claude/process.rs:174-179`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:345-353`

### SD-4: 生存シグナル: Claude は thinking delta + keep_alive で進捗を刻むが、Codex は reasoning 中に一切イベントを出さず stall 誤検知する

- 種別: 扱いが違う（divergent） / 重大度: **medium**

**ユーザー可視の症状**: Codex では思考中の内容が一切ライブ表示されず（完了時に一括出現）、3 分を超える長考で「stall（無応答）」警告が正常動作中に表示される。Claude では同じ長考でも thinking がストリーム表示され stall 警告は出ない。

**詳細**:

Claude では thinking_delta が逐次 MessagePart::Thinking（PartsMerged=進捗）になり（claude/convert.rs:251-259、--include-partial-messages を process.rs:291 で常時付与）、CLI の keep_alive 行も KeepAlive イベントとして stale 監視の progress を更新する（convert.rs:83 → runtime/usecase.rs:2514）。Codex では reasoning は item/completed 時に item 単位で 1 個の Thinking になるだけで（codex/convert.rs:341-354。item/started は reasoning に対し無変換、reasoning delta 通知や keep-alive 相当は wire.rs に登録すら無く convert_notification の既定分岐で破棄）、単一の reasoning item が続く間は progress を更新する runtime イベントが完全にゼロになる。TokenUsageUpdated が届いても record_progress を呼ばない（usecase.rs:2499-2507）ため代替の progress 経路も無い。その結果、stale watchdog（既定 180 秒 stale.rs:16、tool in-flight でなければ延長なし stale.rs:57-63、全 turn で spawn usecase.rs:1537）が、単一 reasoning 区間が 180 秒を超える正常な長考中に stall を観測し、agent-stall-observed 通知（frontend で stall ラベル表示 ChatSessionView.tsx:742-749）と reconnect 試行（Codex は trait デフォルトの Unavailable、gateway.rs:168-171）を発火する。なお reasoning が複数 item に分割される場合は各 item/completed が progress を刻むため、誤検知の正確な成立条件は「単一 reasoning item（または無イベント区間）が 180 秒以上継続」である。stall は非終端シグナルで turn 自体は落ちない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:251-259`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:83`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:347-354`
- `src-tauri/src/infrastructure/agent_session/codex/wire.rs:50-61`
- `src-tauri/src/usecase/agent_session/runtime/stale.rs:16`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1895-1922`
- `src/hooks/useAgentSdkListeners.ts:326-368`

### SD-5: tool 出力 delta の写像差: Codex は outputDelta を ToolResult に変換するため in-flight 判定が壊れ、fileChange は開始時点で結果 part を出す

- 種別: 扱いが違う（divergent） / 重大度: **medium**

**ユーザー可視の症状**: Codex で長時間コマンドが最初に何か出力した後沈黙すると 180 秒で stall 警告が出る（Claude は同条件で 1800 秒まで待つ）。また Codex のファイル編集は実行・承認前から diff が完了済み結果として描画される。

**詳細**:

Claude では ToolResult は user message の tool_result ブロック（ツール完了時）のみで生成される（claude/convert.rs:300-335、生成箇所は 322 の 1 箇所）ため、実行中は ToolUse だけが残り has_in_flight_tool_use（stale.rs:39-53）が真になり stale timeout が 1800 秒へ延長される（stale.rs:57-63）。Codex では item/commandExecution/outputDelta・item/fileChange/outputDelta（convert.rs:155-159）が command_output_delta_part（convert.rs:479-483）で同じ tool_use_id を持つ ToolResult part に変換され、apply_parts→merge_part（usecase.rs:2657-2662）で domain_streaming_parts に蓄積されるため、最初の出力 delta の瞬間から in-flight 判定が偽になり timeout が基準値（既定 180 秒）に戻る（usecase.rs:1845-1847, 1895-1898）。各 delta は record_progress で progress を更新するため、症状が出るのは「序盤に出力した後 180 秒以上沈黙する」コマンド（cargo build 等）に限られ、その場合 Codex だけ stall 誤検知となる（Claude は同条件で 1800 秒まで待つ）。stall はチャット上部の「No agent output for X. Session remains active.」バナー（ChatSessionView.tsx:1769-1777）として最大 3 回表示され、workflow-step セッションでは WorkflowStallObserved 介入シグナルも dispatch される（usecase.rs:1955-1967）。stall recovery の runtime.reconnect() は Claude/Codex とも未実装（gateway.rs:168-171 の既定 Unavailable）のため破壊的動作はなく誤シグナルに留まる。さらに fileChange は item/started（convert.rs:310-316→file_change_tool_parts:409-438）および item/fileChange/patchUpdated（convert.rs:160, 485-494）の時点で ToolUse と is_error=false の ToolResult のペアを emit するため常に in-flight にならず、frontend は結果ペアリング済みツールを実行スピナーなし・CheckCircle2 付きの完了状態で描画する（ActivityLog.tsx:965 の executing = isRunning && !results?.length、同 186 の ToolStatusIcon）。その結果、Codex のファイル編集は適用完了（item/completed での status 判定）前から diff が完了済み結果として表示され、適用失敗時は後から is_error=true のペアで上書きされる。stale.rs:38 と usecase.rs:1873-1875 のコメントは「ToolResult 到着＝ツール完了」を設計不変条件として明記しており、Codex の outputDelta→ToolResult 写像はこの不変条件を破る emergent な不整合であり、テストで意図的仕様として固定されてはいない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:479-483`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:155-159`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:310-316`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:409-438`
- `src-tauri/src/usecase/agent_session/runtime/stale.rs:38-63`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1845-1847`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:300-335`

### SD-6: permission 要求の写像情報差: Codex は生の JSON-RPC params と合成 tool 名で、allow 時の入力編集も破棄される

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: Codex の承認ダイアログには threadId/turnId 等の内部フィールドを含む生 JSON が表示され、Claude のような整形済みの command/diff 表示・入力編集が効かない。transcript 上は tool 名が Edit なのにダイアログでは CodexFileChange と表示される不一致もある。

**詳細**:

Claude では can_use_tool から tool 本来の名前と input（{command: ...} 等）・title/description/decision_reason を写し（claude/permission.rs:58-96）、allow 応答は updatedInput / answers を CLI に返す（同 :222-243）。Codex では requestApproval の params 全体（itemId 込み。プロトコル上 threadId/turnId も含む）を ToolApproval の input として詰め（codex/convert.rs:537-589、input: payload(params.clone())）、tool_name は CodexCommand/CodexFileChange 等の合成名になる。allow 応答は {decision: accept} 固定で updated_input は破棄され、deny の理由 message も command/fileChange では送られない（codex/permission.rs:87-129）。ただし accept/decline 固定応答自体は Codex app-server プロトコルの応答スキーマ制約であり設計仕様（docs/specs/feat-issues-1301/design.md:640）にも明記されている。一方、同設計仕様 :624-625 は input を「params 由来の {command, cwd, reason}」「{itemId 対応の changes/diff}」に絞る意図を示しており、生 params 詰めは実装の仕様乖離。presentation 層（present_agent_permission_request_inner、adaptor .../permission.rs:186-190）は tool 名ベースで整形するため、Codex の command/fileChange/permissions 承認は汎用 'tool' 扱いになる（item/tool/requestUserInput のみ AskUserQuestion/Question として整形される）。is_edit_preview_tool は Edit/MultiEdit/Write のみ一致（同 :48-50）のため CodexFileChange には Claude の Edit/Write で効く diff プレビュー・内容編集 UI が出ず、ダイアログ本文は request.input の生 JSON（内部フィールド混じり）表示になる（PermissionDialog.tsx:292-297, 940-945）。なおダイアログのヘッダは title "Codex approval requested" が出る（toolLabel = title || displayName || toolName、PermissionDialog.tsx:908）ため合成名はヘッダには出ず、transcript の tool 名 "Edit"（codex/convert.rs:395,433）との不一致は ActivityLog の解決済みエントリ「✓ CodexFileChange: allowed」（ActivityLog.tsx:778、session/mod.rs:1190-1205）として可視化される。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:537-589`
- `src-tauri/src/infrastructure/agent_session/codex/permission.rs:115-118`
- `src-tauri/src/infrastructure/agent_session/codex/permission.rs:121-129`
- `src-tauri/src/infrastructure/agent_session/claude/permission.rs:58-96`
- `src-tauri/src/infrastructure/agent_session/claude/permission.rs:222-243`
- `src-tauri/src/adaptor/controller/command/agent_session/permission.rs:182-218`

### SD-7: compaction 失敗の閉じ方: Codex は failed で banner を閉じるが、Claude は失敗経路が無く in_progress のまま残る

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: Claude チャットで compaction 中にエラーや Stop が起きると、「Compacting conversation」の進行中表示が閉じられずに残り続け、あたかも圧縮が永遠に走っているように見える。Codex では同じ失敗が「Compaction failed」として明示的に閉じられる。

**詳細**:

Claude では compaction 通知は system status(compacting)→in_progress（claude/convert.rs:120-127）、compact_boundary→completed（claude/convert.rs:133-139）の2状態のみで、ClaudeConvertState は compaction を追跡せず、turn がエラー（convert_result は Error part + TurnCompleted(Failed) のみ生成、convert.rs:205-234）や中断で終わった場合に in_progress を閉じる経路が存在しない（finalize_turn は tool call と permission しか閉じない: finalization.rs:19-37。projector の TurnCompleted/TurnInterrupted も SystemNotification に触れない: projector.rs:374-407）。Codex では compaction_in_progress を state で追跡し（codex/convert.rs:31,136,147,162）、turn が failed/errored で終わった場合に status=failed の SystemNotification を合成して閉じる（codex/convert.rs:181-196、テスト convert.rs:953 で仕様固定）。その結果、Claude で compaction 中に turn が失敗・中断すると「Compacting conversation」の in_progress part（frontend では animate-pulse 付き⏳表示: ChatSessionView.tsx:194-205）が transcript に恒久的に残る。in_progress の置換は turn 単位の assistant_parts に閉じているため（projector.rs:779-797）、後続 turn の compaction でも閉じられない。補足: Codex も interrupted（Stop）経路では flag をリセットするだけで failed 通知を合成しないため（codex/convert.rs:205-208）、Stop 時の stale banner は両 agent 共通の未処理ギャップであり、Claude だけが劣る乖離は failed/errored 経路に限られる。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:120-139`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:186-196`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:161-167`
- `src-tauri/src/usecase/agent_session/event_log/finalization.rs:7-45`

## OB: 送信側（ユーザー → agent）の差・喪失

ユーザー入力が backend に届くまでの経路での喪失・非対称。

### OB-1: Codex の interrupt は turn_id 未取得ウィンドウで無言 no-op になり、以後の停止操作も frontend が握りつぶす

- 種別: 扱いが違う（divergent） / 重大度: **high**

**ユーザー可視の症状**: Codex セッションで turn 開始直後（または backend ハング時）に停止ボタンを押すと、何も起きないまま agent が実行を続ける。ボタンは「停止中」表示のまま再押下も効かず、turn が自然終了するまでセッションを止められない。Claude では同じ操作が確実に（最悪 10 秒で）止まるため、backend によって停止の信頼性が大きく違って見える。

**詳細**:

CodexSessionRuntime::interrupt は thread_id または turn_id が None のとき何も送信せず Ok(()) を返す（codex/session.rs:214-224）。turn_id は turn/started 通知が届いて初めてセットされ（codex/convert.rs:119-124、session.rs:369 で runtime state へ mirror）、turn/start の成功レスポンスからは取得しない。turn 完了（convert.rs:180）と turn/start エラー応答（convert.rs:71）で None に戻る。一方 usecase 層は start_turn 送信前に reset_for_turn で phase=Streaming にする（runtime/usecase.rs:1446、session_state.rs:127-128）ため、app-server spawn + wait_for_thread_id（最大15秒, codex/session.rs:184）+ turn/started 受信までのウィンドウ全体で停止ボタンが有効になり、この間の停止要求はどこにも伝わらない。エラーも返らないため usecase 層（runtime/usecase.rs:466-477）にリトライや pending-interrupt の仕組みはなく、Tauri command（adaptor/controller/command/agent_session/session.rs:165-176）経由で frontend の invoke も成功として resolve する。frontend は停止押下時に楽観的に interrupting フラグを立てて再押下を握りつぶし（useAgentChat.ts:761-773）、フラグは turnPhase が idle になるまで解除されず（agentChatReducer.ts:469-476）、さらに停止ボタン自体が disabled={isInterrupting} で「Stopping…」表示のまま無効化される（AgentChatPanel/MessageInput.tsx:805-811）。Claude は interrupt を常に送信し、10秒（ABORT_SYNTHESIS_DELAY, claude/session.rs:30）で TurnCompleted(Interrupted) を合成する fallback timer を持つ（claude/session.rs:214-231, 493-532）が、Codex に相当する仕組みはない。Codex で合成 TurnCompleted(Interrupted) が出るのはプロセス crash 時のみ（codex/session.rs:396-404, 413-421）で、それも turn_id.is_some() が前提のため当該ウィンドウでは crash してすら turn が終端しない。stale watchdog も #1374 以降は非破壊な stall signal + reconnect のみ（runtime/usecase.rs:1972-1983）で turn を終端しないため、app-server ハング時はセッションが無期限にロックされる。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/session.rs:214-224`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:119-124`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:493-532`
- `src/hooks/useAgentChat.ts:761-773`
- `src/hooks/agentChatReducer.ts:469-476`

### OB-2: stalled turn 中の送信は両 backend とも steer 不可で即エラーになり、入力テキスト・画像が完全に失われる

- 種別: 捨てている（dropped） / 重大度: **high**

**ユーザー可視の症状**: agent が 3 分以上無反応（まさにユーザーが追加指示や催促を打ちたくなる状況）でメッセージを送ると、「メッセージ送信に失敗: active-turn steering is not available for backend 'claude'」という意味不明なエラーが出て、長文の入力・添付画像が入力欄ごと消えてしまう。通常の実行中なら queue に積まれるのに、止まっている時だけ入力が捨てられる。

**詳細**:

send_message は stall 観測中（デフォルト 180 秒無出力、stale.rs:16 / usecase.rs:1909。ただし tool 実行中は stale.rs:57-63 により 1800 秒へ延長）の turn に対し、backend が steering 非対応なら queue せずエラーを返す（runtime/usecase.rs:292-297）。この時点では add_human_message_internal 前なのでメッセージは永続化されない。Claude/Codex とも capabilities().steering=false（claude/models.rs:58, codex/models.rs:65）で、どちらの SessionRuntime も steer をオーバーライドしていない（gateway.rs:160-165 のデフォルトは Unavailable）ため、steer 分岐（usecase.rs:299-336）は本番で到達不能であり、stalled turn への送信は常にこのエラーになる。なお backend がエラーを返すこと自体は usecase.rs:7568-7592 のテストで「stalled retry/continue must not be silently queued」という意図的仕様として固定されている（issues-1301 D16/F-2）ので、欠陥の本体は frontend 側にある: (a) MessageInput.tsx:447-456 は onSend を await せず入力欄と添付画像を即クリアする。(b) useAgentChat.ts:920-925 の catch は SET_ERROR でバナー表示するのみでエラーを swallow するため、MessageInput 側が await しても失敗を検知できず、入力を復元する経路が存在しない。(c) しかも stall 中は ChatSessionView.tsx:1769-1781 が「No agent output for X. Session remains active.」バナーを表示しつつ MessageInput を無効化しない（同 1808-1828）ため、まさにユーザーの介入入力を誘発した上で、送信された本文と画像を復元不能に破棄する。既知問題 1 と同型の『成功前提の楽観処理 + 失敗経路での取りこぼし』が入力側にも存在する。修正は frontend の失敗時入力復元（または送信失敗をエラーとして伝播させる API 変更）か、backend エラーをユーザー向け文言に変換した上での入力保全が必要。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:292-297`
- `src-tauri/src/infrastructure/agent_session/claude/models.rs:58`
- `src-tauri/src/infrastructure/agent_session/codex/models.rs:65`
- `src-tauri/src/domain/agent_session/gateway.rs:160-165`
- `src/components/panels/AgentChatPanel/MessageInput.tsx:447-456`
- `src/hooks/useAgentChat.ts:920-925`

### OB-3: pending queue はメモリのみで、session close / backend 切替 / アプリ再起動で queue 済みメッセージの turn が無言で消滅する

- 種別: ライフサイクルで失われる（lossy-lifecycle） / 重大度: **medium**

**ユーザー可視の症状**: 実行中にメッセージを 2〜3 件先送りしてからアプリを再起動（またはタブを閉じる・backend を切り替える）と、再表示された transcript には送ったメッセージが並んでいるのに agent は永遠に応答せず、エラーも queue チップも出ない。「送ったのに無視された」ように見える。

**詳細**:

turn 実行中に送ったメッセージは human message として即永続化された上で（runtime/usecase.rs:337-344）、in-memory の RuntimeSessionState.pending_queue に積まれる（session_state.rs:50、usecase.rs:345-364）。QueuedTurnInput はどこにも永続化されない。close_session は state ごと map から remove し（usecase.rs:839-848）、close_all（アプリ終了、usecase.rs:850-861）や set_session_backend（usecase.rs:820）も同様。frontend の closeSession（useSessionStore.ts:326 → close_session コマンド）でも発生する。復旧経路 recover_queued_turn_if_idle_without_runtime（usecase.rs:703-713）もメモリ上の queue しか見ないため、消えた turn を再構築する手段はない。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/session_state.rs:50`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:337-364`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:839-861`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:820`
- `src/hooks/useSessionStore.ts:326`

### OB-4: cancel_queued_turn は永続化済み human message を残すため、キャンセルしたはずのメッセージがリロード後に transcript へ復活する

- 種別: ライフサイクルで失われる（lossy-lifecycle） / 重大度: **medium**

**ユーザー可視の症状**: queue 済みメッセージを×で取り消したのに、後でセッションを開き直すとそのメッセージが返信なしで履歴に居座っている。さらにセッション復元時にはキャンセルしたはずの指示が復元コンテキストとして agent に再提示され、取り消した指示に agent が言及することがある。

**詳細**:

queue 投入時に human message は store へ永続化され existing_human_message_id として queue entry に紐づく（runtime/usecase.rs:337-355）が、queue 中は frontend が transcript に表示しない（useAgentChat.ts:885-898 で queuedTurn 時は ADD_MESSAGE しない。queue は ChatSessionView.tsx:1782 の chip 表示のみ）。cancel_queued_turn はメモリ上の pending_queue から entry を除去するだけで（usecase.rs:922-949）、永続化済み human message を削除しない（backend に message 削除 API 自体が存在しない）。結果、ライブ UI ではキャンセルでメッセージが完全に消えるが、get_session は store のページをそのまま返し frontend も pending queue との突合せをしないため、セッションをリロードするとそのメッセージが返信のない通常発言として transcript に現れる。さらに restore 時の reinjection 経路（native agent_session_id がなく Reinject plan になる場合。context_restore.rs:125-139 は非空の human message を全て転送対象にする）では、キャンセルしたはずの指示が過去文脈として agent に再提示される。なお native session を Resume できる場合は reinjection は走らないため、混入は Reinject 経路に限られる。同型の問題として、queue 中にアプリを再起動した場合も pending_queue（in-memory）だけが消えて永続 message が orphan 化する。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:922-949`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:337-344`
- `src/hooks/useAgentChat.ts:885-898`
- `src-tauri/src/usecase/agent_session/runtime/context_restore.rs:125-139`

### OB-5: ユーザーの interrupt 直後に pending queue が無条件 drain され、次の queue メッセージが即座に実行開始される

- 種別: その他（other） / 重大度: **medium**

**ユーザー可視の症状**: agent の暴走に気づいて停止を押しても、先送りしてあったメッセージが即座に走り出して作業が続行される。完全に止めるには「停止 → queue を個別キャンセル → また停止」を素早く繰り返す必要があり、Codex では新 turn 直後の停止が無効になることもある。

**詳細**:

apply_runtime_event は TurnCompleted を結果種別（Completed / Failed / Interrupted(Abort) / Crash）に関わらず actions.drain() する（runtime/usecase.rs:2530-2534、Fatal 経路も :2598）。complete_turn（:3139）は pending_queue に触れず、drain は run_runtime_event_post_actions（:2246-2249）で TurnCompleted 適用直後に同期的に start_next_queued_turn（:3305）を呼ぶため、ユーザーが停止ボタンで turn を abort しても（Claude: claude/session.rs:474-475、Codex: codex/convert.rs:205-206 が abort を TurnCompleted(Interrupted{Abort}) に変換）、queue に残っているメッセージが即座に次の turn として開始される。停止と queue 取消は全層で別操作であり、backend interrupt（usecase.rs:466-477）も frontend interrupt（useAgentChat.ts:761-773）も queue を触らない。queue クリアは cancel_queued_turn（usecase.rs:922-934、id=None で全クリア）のみだが、UI が公開するのは個別 ID キャンセルだけ（BoundSessionChat.tsx:227-229）。Codex では drain で開始された新 turn の turn_id が turn-started 受信（codex/session.rs:369）まで None のため、その間の interrupt は silent no-op（codex/session.rs:222-224）となり、停止直後の再停止が一時的に無効化される。なお Completed 後 drain と Fatal 後の queue 温存＋再開はテストで意図仕様として固定されている（usecase.rs:5973）が、ユーザー abort 後に drain する挙動を固定するテストは存在しない。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2530-2534`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2598`
- `src/hooks/useAgentChat.ts:761-773`

### OB-6: queue 済み turn の起動失敗後は自動リトライ・自動 drain がなく、queue がユーザーの次操作まで沈黙したまま停止する

- 種別: ライフサイクルで失われる（lossy-lifecycle） / 重大度: **low**

**ユーザー可視の症状**: queue に積んだメッセージの起動が一度失敗すると（CLI 起動失敗・プロセス死亡など）、セッションがエラー表示のまま queue チップが残り続け、待っていても何も起きない。もう 1 通ダミーメッセージを送って初めて詰まっていた turn が動き出す。

**詳細**:

start_next_queued_turn の失敗経路はいずれも queue front を残したまま return し、自動リトライ・自動 drain の再スケジュールがない。turn_id 採番失敗（runtime/usecase.rs:3392-3397）は状態変更もイベント発行もなく無言。runtime.start_turn 失敗（:3513-3548）は TurnInterrupted(Crash)+SessionState::Error を出すが queue front は pop されない（pop は成功分岐 :3549-3559 のみ）。runtime 再 open 失敗（:3362-3389）も同様に queue 残置。drain の再トリガは event pump（:2078→run_runtime_event_post_actions :2246-2249）経由の live runtime イベント（TurnCompleted :2533、Fatal :2598、resume mismatch :2131）か、ユーザーが新規メッセージを送った時の recover_queued_turn_if_idle_without_runtime（:290, :703-713）のみ。再 open 失敗時は runtime 不在でイベントは永遠に来ず、turn_id 失敗時も turn 未開始のため completion イベントは来ない。テスト（queued_turn_start_turn_failure_preserves_queue_and_retries 等）は「queue を失わない」ことを意図的仕様として固定しているが、リトライは手動 drain 呼び出しでしか検証されておらず、失敗後に自動 drain を予約する機構が存在しないのが実際のギャップ。frontend の queue チップ（agentChatReducer.ts:48 pendingQueues）は成功時の pending_message_consumed（usecase.rs:3560）でしか消えないため、失敗時はチップが残り続ける。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3392-3397`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3513-3556`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:703-713`

### OB-7: 画像のみ送信時、Claude は空の text block を必ず付与し、Codex は明示的に省略する（wire 生成の非対称）

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: スクリーンショットだけ添付してテキストなしで送ると、Codex では普通に処理されるのに、Claude では turn が API エラーで失敗し得る（空 text block 拒否）。同じ操作が backend によって成否が分かれる。

**詳細**:

Claude の user_message は prompt が空文字でも常に {"type":"text","text":""} を content 先頭へ push する（claude/wire.rs:144-147、関数全体は 140-167）。Codex 側は codex_user_input（codex/session.rs:574-586）で「prompt が非空、または images が空」の場合のみ text item を積み、画像のみ送信時は空 text を送らないよう明示的にガードしている。frontend は画像のみ（本文空）の送信を許可し（MessageInput.tsx:436-439 の handleSubmit、useAgentChat.ts:808 の sendMessage 双方が images 有りなら空文字を通過させる）、backend の usecase/adaptor 層にも user prompt の空バリデーションはなく、claude/session.rs:209 で input.prompt が無加工で user_message に渡る。結果、画像のみ送信時に Claude backend だけが空 text block を含む stream-json を CLI に書き込む。Anthropic Messages API は空 text block を invalid_request_error で拒否する既知の挙動があり（claude CLI がこれをサニタイズするかはリポジトリ外の挙動のため未検証）、Codex 側にのみガードが実装されている非対称から、Claude 側に同等の回避策（空 prompt 時は text block を省略）が欠けている。修正は wire.rs の user_message に Codex と同じ条件（prompt 非空 or images 空のときのみ text を push）を入れるのが対称的。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/wire.rs:144-157`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:574-586`
- `src/components/panels/AgentChatPanel/MessageInput.tsx:437-439`

### OB-8: resume mismatch の requeue で editor_context が脱落する（current_turn_input 構築時に None 固定）

- 種別: 捨てている（dropped） / 重大度: **low**

**ユーザー可視の症状**: Codex セッションの復元が resume mismatch で作り直しになった直後の 1 turn だけ、「今開いているこのファイルの選択範囲を見て」系の依頼で agent がエディタ状態を受け取れず、見当違いのファイルを参照する。

**詳細**:

start_turn_for_session は current_turn_input を構築する際、QueuedTurnInput::new の editor_context 引数に None を渡す（runtime/usecase.rs:1447-1456。payload.editor_context: Option<EditorContext> はスコープ内に存在するが、QueuedTurnInput 側は Option<AgentEditorContext> 型で逆変換 From<EditorContext> for AgentEditorContext が未実装のため使われていない）。resume mismatch 時は handle_resume_mismatch がこの current_turn_input を queue 先頭へ戻し（:2091-2094）、start_next_queued_turn（:3305）が queued.editor_context のみから TurnInput を構築する（:3509）ため、リトライされた turn の editor_context は None になる。影響は2経路: (a) Codex は editor_context を additionalContext としてワイヤ送信するため（codex/session.rs:545-547）、リトライではエディタ状態（アクティブファイル・選択範囲）が送られない。(b) リトライ turn の system prompt 再構築（build_queued_system_prompt、usecase.rs:3346→3670-3672 の system_context_editor_input）からも editor context が消えるため、Claude セッションでも system prompt 経由のエディタ状態が脱落する。mentions（:1454）や images（:1451）は保持されるのに editor_context だけ落ちる非対称。修正は元の AgentEditorContext を start_turn_for_session まで引き回すか、逆変換を追加して current_turn_input 構築時に渡す。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1447-1456`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2091-2094`
- `src-tauri/src/infrastructure/agent_session/codex/session.rs:545-547`

## RT: runtime 〜 event log 〜 read model 経路の喪失・変質

正規化後のイベントが永続化・投影される過程で失われる・変質するもの。

### RT-1: close_session / backend切替 / アプリ終了がターン進行中でも finalize も flush もせず、backend の終了イベントも捨てる

- 種別: ライフサイクルで失われる（lossy-lifecycle） / 重大度: **high**

**ユーザー可視の症状**: ストリーミング中にチャットタブを閉じる・backend を切り替える・アプリを終了すると、再オープン時に返答が文の途中で切れ、ツール実行が永久にスピナーのまま、permission カードが永久に「確認待ち」のまま残る。中断されたという表示は一切出ない。

**詳細**:

AgentSessionRuntimeUsecase::close_session (runtime/usecase.rs:839-848) は sessions map から state を remove してから runtime.close() するだけで、complete_turn / finalize_turn / flush_streaming_update(force_persist=true) を一切呼ばない。state を先に消すため、close と競合して in-flight のランタイムイベントは apply_runtime_event の guard (usecase.rs:2364-2376, sessions.get→None) で破棄され、さらに Claude/Codex の infrastructure 層は closed フラグにより shutdown 起因の終了イベント（TurnCompleted(Interrupted)/Crash）の emit 自体を抑止する (claude/session.rs:286-290, 346, 537-541 emit_crash_if_unexpected / codex/session.rs:321-324)。どちらの層でも turn の terminal event は適用されない。Tauri command 側 (adaptor/controller/command/agent_session/stored_session.rs:191-201) は runtime close 後に SessionClosed を append するだけ (lifecycle_controller.rs:43-53)。close_all (usecase.rs:850-861, アプリ終了: application_lifecycle.rs:9) と set_session_backend (usecase.rs:801-824) も同経路。frontend の tab close (useAgentChat.ts:954-959) も interrupt せず直接 close_session を呼ぶ。結果: (1) 最後の streaming persist (1秒間隔: streaming.rs:6, usecase.rs:2794-2811) 以降のストリーミング本文と未 persist の pending parts がメッセージストアに書かれず消える（closeはforce persistしない）。(2) event log には TurnStarted と durable part event（ToolCallStarted/PermissionRequested 等: usecase.rs:2727-2736→3860-3898）と SessionClosed が残るが、その turn の terminal event（TurnCompleted/TurnInterrupted）は永久に記録されない。finalize_turn の本番呼び出しは complete_turn 経由の usecase.rs:4089 のみで（log.rs:108 の finalize は cfg(test)、usecase.rs:1483/1577/3525 の TurnInterrupted は新 turn の起動失敗用）、閉じられた turn を後から修復する経路は存在しない。(3) pending Permission part は Pending のまま永続化され、再オープン時に actionable な permission ダイアログとして描画されるが (ChatSessionView.tsx:574-587, PermissionDialog.tsx:588)、respond_permission は live runtime を必須とするため必ず "No active agent runtime" で失敗し (usecase.rs:488-496)、新 turn 開始後は event-log fallback (finalization.rs:57-74 latest_unresolved_permission_request) が最新 turn しか見ないため、このカードは永久に解決不能。(4) ツール実行の見え方: session status 自体は SessionClosed→projector (projector.rs:807-817) で Closed/Idle に修復されるため、再オープン後の通常 ToolUse はスピナーではなく「結果なしの静的表示」になる (ChatSessionView.tsx:550-551 の isExecuting は isLastAgentStreaming 依存)。ただし background Task group のみ isRunning=!group.isCompleted (ActivityLog.tsx:900-901) のため永久スピナーになる。ユーザーに見える症状: ストリーミング中にチャットタブを閉じる・アプリを終了する（UI 上 backend 切替は空セッション限定のため主経路はこの2つ）と、再オープン時に返答が文の途中で切れ、中断されたという表示（TurnInterrupted 由来のエラー part）が一切出ず、ツール実行は結果不明のまま残り（background Task は永久スピナー）、permission カードは永久に確認待ちのまま残って Allow/Deny を押すとエラーになる。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:839-848 (close_session: remove→close のみ)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2364-2376 (state 不在で全イベント破棄)`
- `src-tauri/src/adaptor/controller/command/agent_session/stored_session.rs:191-200 (close command は SessionClosed append のみ)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:4089 (finalize_turn の本番呼び出しは complete_turn 経由のこの1箇所のみ)`
- `src/components/panels/AgentChatPanel/ActivityLog.tsx:965 (ToolResult 無しの ToolUse は executing スピナー表示)`

### RT-2: クラッシュ/強制終了後の再起動時に dangling turn を回収する経路がなく、ストリーミング本文は durable event にも残らない

- 種別: ライフサイクルで失われる（lossy-lifecycle） / 重大度: **medium**

**ユーザー可視の症状**: アプリやbackendがクラッシュした後にセッションを開き直すと、直前約1秒分の返答テキストが消え、ツール実行スピナーと「確認待ち」permission カードが永久に残る。permission に応答しようとすると『No active agent runtime』エラーになり、どの操作でもこの残骸は解消されない。

**詳細**:

ターン中に durable event 化されるのは ToolUse/ToolResult/Permission/TaskStatus/Todo/SystemNotification/Image/ImageRef のみで（usecase.rs:3900-3912 part_records_durable_event）、Text/Thinking/Error を event 化する PartEventMode::FinalLiveBlocks（event_log/part_events.rs:21-52）は production から一度も呼ばれない。本文の durable 記録は complete_turn 時の FinalPartsRecorded（usecase.rs:4052-4069）だけで、ターン中の永続化はメッセージストアへの1秒間隔スナップショット（streaming.rs:6、usecase.rs:2794-2814。force persist は complete_turn の usecase.rs:3158 のみ）に限られる。プロセスがクラッシュすると complete_turn が走らないため FinalPartsRecorded / TurnInterrupted は永久に欠落し、再起動後に dangling turn を finalize する経路は存在しない（restore_session_state は projection で state を補正後 Idle に直すだけ: session/lifecycle_controller.rs:55-69。TurnInterrupted の他の生成箇所 usecase.rs:1483/1577/3525 はすべて現行 turn の起動失敗処理）。event log の PermissionRequested は未解決のまま残り、finalize_turn の Cancelled 畳み込み（finalization.rs:29-37）は turn_id スコープのため後続 turn でも解消されない。再起動後の get_session は runtime.is_some() が条件（usecase.rs:980-983）でダイアログを復元せず、persist 済み parts の pending permission カード（ChatSessionView.tsx:574-587 で操作可能なまま描画）に応答すると、#1379 で追加された event log fallback（usecase.rs:686-700）が pending を見つけても直後の runtime lookup で「No active agent runtime」エラーになる（usecase.rs:488-496。さらに新 turn 開始後は latest_unresolved_permission_request が最終 turn しか見ないため「No pending permission request」エラーに変わる）。既知問題2（finalize 時に Cancelled へ畳む）の補集合で、畳む処理自体が走らないケース。ユーザー可視の症状: クラッシュ後にセッションを開き直すと直前約1秒分の返答テキストが消え、「確認待ち」permission カードが永久に pending のまま残り、応答すると「No active agent runtime」エラーになる。永久スピナーは background Task group のみ（ActivityLog.tsx:900-902 で isBackground は !isCompleted で回転継続）で、通常の ToolUse は isLastAgentStreaming が false になるためスピナーは出ず結果無しの行として残る。セッション自体は Idle に復元され新規メッセージ送信は可能だが、残骸を解消する操作は存在しない。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3900-3912 (Text/Thinking/Error は durable event 対象外)`
- `src-tauri/src/usecase/agent_session/event_log/part_events.rs:21-52 (FinalLiveBlocks モード限定)`
- `src-tauri/src/usecase/agent_session/runtime/streaming.rs:6 (STREAMING_PERSIST_INTERVAL=1s)`
- `src-tauri/src/usecase/agent_session/session/lifecycle_controller.rs:55-69 (restore は state 補正のみ、parts/event を修復しない)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:488-496 (runtime 不在時の respond_permission はエラー)`

### RT-3: キュー済みターン (pending_queue) がメモリのみで、再起動・close で「送信済みだが永久に応答されないメッセージ」が残る

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: エージェント実行中に続けて送ったメッセージ（キュー表示されていたもの）が、アプリ再起動やタブclose後に黙って実行されなくなる。チャット上には自分のメッセージが残っているのに、エージェントは永久に応答せず、エラーも出ない。

**詳細**:

ターン実行中に送ったメッセージは human message として session_store.append_message で即永続化された上で (usecase.rs:337-344 → session/mod.rs:1415-1442)、実行キューは RuntimeSessionState.pending_queue (runtime/session_state.rs:50) にのみ積まれる (usecase.rs:345-364)。QueuedTurnInput (runtime/queue.rs:6-19) は serde 非対応でどこにも永続化されず、durable event log (event_log/events.rs) にも TurnQueued 相当のイベントが存在しないため、アプリ再起動 (close_all: usecase.rs:850-861、application_lifecycle.rs:9 から呼出)・close_session (usecase.rs:839-848、frontend の closeSession → Tauri command 経由)・set_session_backend (usecase.rs:801-824、820行で close_session 呼出) で state ごと消える。human message は chat 履歴に残るが event log には対応する TurnStarted が無く、agent 返答は永久に生成されない。復旧手段の recover_queued_turn_if_idle_without_runtime (usecase.rs:703-713) もメモリ上のキューしか見ず、起動時に未応答 human message を検出して再キューする reconcile 処理も存在しない。さらに resume plan (context_restore.rs) は CLI 側 session を resume するため、CLI に送信されなかった孤立メッセージは以後のターンのコンテキストにも含まれない。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:337-364 (human message 永続化＋メモリキュー)`
- `src-tauri/src/usecase/agent_session/runtime/session_state.rs:50 (pending_queue: VecDeque、永続化なし)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:850-861 (close_all で drain)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:703-713 (復旧はメモリキュー前提)`

### RT-4: event log への append がクラッシュで欠けた ']' を自己修復せず、以後そのセッションの全イベント記録（送信含む）が恒久的に失敗する

- 種別: その他（other） / 重大度: **medium**

**ユーザー可視の症状**: タイミング悪くクラッシュした後、その特定のチャットセッションでだけメッセージ送信が毎回エラーになり、二度と会話を続けられなくなる（新規セッションでは動くため原因が分かりにくい）。

**詳細**:

append_session_event_to_dir は追記時にファイル末尾の非空白バイトが ']' であることを要求し、違えば『event log does not end with a JSON array』でエラーにする (adaptor/gateway/agent_session/session_storage/event_store.rs:105-110)。追記自体は set_len で ']' を削ってから本体と ']' を書き直す方式 (event_store.rs:113-123) のため、この間にクラッシュすると ']' の無いファイルが残る。読み取り側には recover_unclosed_session_events (event_store.rs:130-163) があるが、append 側に修復が無いので、以後 TurnStarted の append (usecase.rs:1427-1440 で `?` により send_message ごと失敗)・SessionClosed・PermissionResolved など全 append が恒久的に失敗し続ける。

**証拠**:

- `src-tauri/src/adaptor/gateway/agent_session/session_storage/event_store.rs:105-114 (']' 必須チェックと set_len 方式の追記)`
- `src-tauri/src/adaptor/gateway/agent_session/session_storage/event_store.rs:130-163 (修復は読み取り側のみ)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1427-1440 (TurnStarted append 失敗で send_message がエラー)`

### RT-5: workflow への turn 完了通知が失敗理由を運ばない（Error part 除外＋TurnCompleted に error フィールドなし）

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: workflow ステップとして走らせた agent turn が API エラー等で失敗すると、workflow 側の判定・履歴には『exit 1・出力なし』としか残らず、失敗理由が判断材料に使えない。ユーザーは該当セッションのチャットを開いて Error part を探すしかない。

**詳細**:

terminal_projection は TurnResult::Failed { error, .. } の error を捨てて TerminalEventProjection::Completed(exit_code=1, stop_reason=None) に写像し (runtime/usecase.rs:4119-4127)、durable event TurnCompleted 自体に error フィールドが無い (event_log/events.rs:249-256。TurnInterrupted には error があるが Failed はそこに写像されない)。projector の project_workflow_turn_complete は final_text_parts に MessagePart::Text だけを集め、Error part を除外する (event_log/projector.rs:874-881)。backend は失敗時に Error part + TurnCompleted(Failed) を流す (claude/convert.rs:217-226, codex/convert.rs:70-81) ため、失敗理由はチャット表示にだけ残る。WorkflowTurnCompleteNotification (runtime/usecase.rs:4168-4187) には exit_code=1 と（モデルがテキストを出していなければ）空の final_text_parts しか届かず、failure_signal も stop_reason 由来の ModelRefusal のみで Failed 時は常に None。下流でも workflow engine の failure_reason は「AgentSession error at step '{node}' (exit_code: N)」の汎用文字列のみ (domain/workflow/services/transition.rs:238-241) で、SessionError 経路では final_parts を消費すらしないため、RetryPolicy の失敗種別判定・step 履歴・ユーザー向け失敗表示のいずれにも失敗理由が届かない。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:4119-4127 (Failed の error 破棄)`
- `src-tauri/src/usecase/agent_session/event_log/events.rs:249-256 (TurnCompleted に error なし)`
- `src-tauri/src/usecase/agent_session/event_log/projector.rs:874-881 (final_text_parts は Text のみ)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:4168-4187 (workflow 通知の内容)`

### RT-6: Idle 中の Fatal はエラーメッセージがログ以外のどこにも残らず、セッションが理由なしに Error 表示になる

- 種別: 捨てている（dropped） / 重大度: **low**

**ユーザー可視の症状**: 待機中に backend プロセスが死ぬと、セッションバッジが突然 Error になるがチャットには何のメッセージも出ず、reload すると Error 表示自体も消えて何が起きたのか一切分からない。

**詳細**:

AgentRuntimeEvent::Fatal はターン進行中なら complete_turn(Interrupted{Crash, error}) 経由で TurnInterrupted event → projector が Error part を合成するが、phase==Idle の場合 (should_complete_crash=false) は log::warn と set_session_state(Error) と transient な state change emit のみで (usecase.rs:2536-2599、特に 2572-2597)、durable event もチャットへの Error part も一切残らない。state change payload (AgentSessionStateChangedPayload) に message フィールドがないため、エラーメッセージはライブの UI 通知にも含まれない。message はどのストアにも記録されないため（codex の startup_error は in-memory で、Fatal 処理の runtime close で消滅）、Error 状態の理由を後から知る手段がない。SessionState::Error 自体は session meta に永続化されるので reload 後も Error バッジは表示され続けるが、理由は不明のまま。さらに次に何か event が append されると append_session_event_and_project_state (session/store.rs:466-485) が event log projection で state を上書きし、Error だった痕跡自体が消える。ユーザー可視の症状: 待機中に backend プロセス（特に常駐する Codex app-server）が死ぬと、セッションバッジが突然 Error になるがチャットには何のメッセージも出ず、理由は log ファイル以外に残らない。次にメッセージを送る等で event が append されると Error 表示自体も消え、何が起きたのか一切分からなくなる。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2536-2599 (Fatal 処理、idle 時は log のみ)`
- `src-tauri/src/usecase/agent_session/session/store.rs:466-485 (次の append で state が projection により上書き)`

### RT-7: キュー済みターンの runtime 再オープン失敗時、キューが黙って停止し persist 失敗も握りつぶされる

- 種別: ライフサイクルで失われる（lossy-lifecycle） / 重大度: **low**

**ユーザー可視の症状**: キューに積んだメッセージがあるのにセッションが Error 表示になり、そのまま何も起こらない。新しいメッセージを送るとなぜか古いキューが動き出す、という不可解な挙動に見える。

**詳細**:

start_next_queued_turn で runtime 再オープンに失敗すると、`let _ = ctx.session_store.set_session_state(Error)` で永続化失敗を握りつぶしつつ (usecase.rs:3366-3370)、interrupted:true / SessionState::Error の state change を emit して return する (usecase.rs:3371-3388)。このときキュー先頭は pop されず（pop は start_turn 成功分岐 3549-3559 のみ）、durable event も残らず、再試行のスケジュールも無い。start_next_queued_turn の production トリガーは (a) runtime イベント後の post-action drain (usecase.rs:2246-2249) と (b) send_message 冒頭の recover_queued_turn_if_idle_without_runtime (usecase.rs:290-291, 703-713) の 2 つだが、再オープン失敗後は runtime が None のまま runtime イベントが発生し得ないため (a) は発火せず、ユーザーが追加送信しない限りキューは永久に停止する（キューは in-memory のみで、アプリ再起動では消失する）。既存テスト（queued_turn_started_event_failure_preserves_queue_and_retries 等）は「失敗時にキューを保持し次の drain で再試行できる」ことを固定しているが、その drain はテスト専用ヘルパ drain_next_queued_turn_for_test で人工的に発火させており、この失敗ケースの自動再試行トリガー欠如は仕様として固定されていない。同様に start_turn 失敗時の TurnInterrupted append も `let _ =` で握りつぶされ (usecase.rs:3522-3531)、TurnStarted は 3470-3483 で append 済みのため、この append が失敗すると event log 上は turn が進行中のまま残る（in-memory は rollback_started_turn で復旧し UI は Idle になるが、durable log と不整合になる）。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3360-3390 (再オープン失敗: 3366 の let _ = とキュー未 pop)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3522-3531 (TurnInterrupted append の握りつぶし)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:703-713 (回収は send_message 契機のみ)`

### RT-8: FinalPartsRecorded の append 失敗時、projection 由来の text 欠落 parts が persist 済みの本文を上書きし得る

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: ターン完了の瞬間に、直前までストリーミング表示されていた返答テキストがツール実行履歴だけのメッセージに置き換わり、reload しても本文が戻らない。

**詳細**:

complete_turn は append_final_turn_events が失敗しても warn のみで続行し（usecase.rs:3213-3224）、load_session_events→project の結果 agent_parts_for_message(&message_id) が非空ならそれを persist する（usecase.rs:3234-3248）。append_final_turn_events は FinalPartsRecorded を最初に append して失敗時に短絡するため（usecase.rs:4052-4069）、この append だけが失敗すると projection は durable event（ToolCallStarted/Succeeded/Failed・Permission・TaskStatus・Todo・SystemNotification・Image、ストリーミング中に usecase.rs:2727-2736 で即時 append 済み）のみから組み立てられ、Text/Thinking は durable event として記録されない（TextRecorded/ReasoningRecorded は FinalLiveBlocks モード専用で production 未使用: part_events.rs:21-42、呼び出しは DurableOnly のみ: usecase.rs:3888）ため含まれない。TurnStarted が assistant_message_id = streaming_message_id を記録する（usecase.rs:1432-1440, session_state.rs:127-130）ため message id は一致し、tool-only の再構成が .filter(!is_empty) を通過して、1秒間隔 persist（usecase.rs:2794-2814）と complete_turn 冒頭の force flush（usecase.rs:3158）で保存済みだった本文入り parts を完全置換で上書きする（message_store.rs:286-291）。live parts は直後にクリアされ（usecase.rs:3250-3259）修復経路はない。発生条件: (a) 当該ターンが tool 使用等の durable part を含むこと（純テキストターンは projection が空になり live parts にフォールバックして無傷）、(b) TurnStarted と最低1つの durable event の append 成功後、mid-turn で event append が失敗し始めること（turn 開始前から壊れている場合は TurnStarted 自体が落ちてターン開始失敗またはフォールバックで無傷）。具体的トリガ: mid-turn の append I/O 失敗（disk full 等）が event log を閉じていない状態にすると、以後 append は「does not end with a JSON array」で失敗する一方（event_store.rs:105-110）、read は recover_unclosed_session_events で成功し（event_store.rs:130-163）、divergence が成立する。症状の正確な見え方: turn 完了の瞬間は frontend が transcript を再取得しない（refreshSessions は session 一覧のみ更新: useAgentChat.ts:573-632）ため live view にはテキストが残り続けるが、reload・セッション切替・別クライアントでの読み込み時に当該メッセージが tool 実行履歴だけになり、本文は永続的に失われる。発生時は警告ログのみ。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3213-3224 (append 失敗でも続行し projection を採用)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3234-3248 (projection 非空なら live parts でなく projection を persist)`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3900-3912 (projection に Text が含まれない根拠)`

## FE: frontend の見せ方

backend が保持している情報が UI に出ない・live と reload 後で違って見えるもの。

### FE-1: Interrupt 中の pending permission part が UI 上で永久に「操作可能な dialog」のまま残る（durable では Cancelled 済み）

- 種別: 扱いが違う（divergent） / 重大度: **high**

**ユーザー可視の症状**: permission 待ちで停止ボタンを押すと、チャット内の許可 dialog が Allow/Deny 付きのまま残り続ける。Allow を押しても実行されず赤い『パーミッション応答に失敗』が出るだけ。reload すると同じ dialog が突然『— cancelled』チップに変わる。

**詳細**:

waiting_permission 中の permission part は AgentRuntimeEvent::PermissionRequested → apply_parts(StreamingApplyMode::Immediate)（runtime/usecase.rs:2442-2450）の streaming delta で status=pending として frontend の message parts に入る。ユーザーが停止すると TurnResult::Interrupted が complete_turn（usecase.rs:3139）に届き、flush_streaming_update（usecase.rs:3158、この時点では permission はまだ pending のまま最終 delta が出る）→ append_final_turn_events（usecase.rs:3214→4052）→ Interrupted 経路で finalize_turn（usecase.rs:4089→event_log/finalization.rs:29-37）が PermissionResolved{Cancelled} を durable log に追記し、投影済み parts（status=cancelled）を persist する（usecase.rs:3239-3248）。しかしこの patch 済み parts を frontend に届ける通知は存在しない: 以後は emit_session_state_change（usecase.rs:3285、pending_permission_request: None）のみで、AgentSessionEventNotifier trait（runtime/ports.rs:50-100）自体に final parts を運ぶメソッドがない。frontend 側は agent-session-state-changed で SET_PENDING_PERMISSION null（useAgentSdkListeners.ts:471-476）により pendingPermissions map をクリアするだけで message part の status は書き換えず（agentChatReducer.ts:551-577）、turn 完了時の refreshSessions()（useAgentSdkListeners.ts:507）も listSessions→SET_SESSIONS で summary のみ置換し messages に触れない（agentChatReducer.ts:397-408）。MARK_AGENT_TURN_COMPLETED は interrupted 時は dispatch されず（useAgentSdkListeners.ts:488）、されても timestamp 更新のみ。結果、ChatSessionView.tsx:574-586 が part.status="pending" をそのまま PermissionDialog に渡し、PermissionDialog.tsx:588 の分岐で Allow/Deny ボタン付きフル dialog（AllowDenyButtons: 899, 1036）が session 再読込まで残り続ける。さらに PermissionDialog.tsx:354-416 の visibility heartbeat が backend 上ではもう存在しない request を report_agent_permission_request_observed で報告し続ける。stale な Allow を押すと respond_agent_permission が pending_permission_for_response（usecase.rs:658-701）で失敗する（runtime state の pending は complete_turn:3177 でクリア済み、event log は TurnInterrupted 終端のため latest_unresolved_permission_request が None: finalization.rs:64-66）→「パーミッション応答に失敗」の error banner のみ表示（useAgentChat.ts:1255-1278）。session を再選択/再読込すると投影 read model（projector.rs:256-271）から status=cancelled で読み直され、同じ dialog が突然「— cancelled」chip に変わる。permission 待ち中の停止は日常的な操作であり、毎回確実に再現する。

**証拠**:

- `src/components/panels/AgentChatPanel/ChatSessionView.tsx:574`
- `src/components/panels/AgentChatPanel/PermissionDialog.tsx:588`
- `src/hooks/useAgentSdkListeners.ts:471`
- `src-tauri/src/usecase/agent_session/event_log/finalization.rs:29`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3158`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3285`

### FE-2: turn の crash/timeout エラーは live UI に一切出ず、reload 後にだけ Error block が出現する（無言 Idle 化）

- 種別: 扱いが違う（divergent） / 重大度: **high**

**ユーザー可視の症状**: 回答生成中に CLI プロセスが死ぬと、spinner とアクティビティ表示が消えて『agent が勝手にやめた』ように見えるだけで、エラーは何も出ない。アプリを reload すると同じ turn にオレンジの Error block と『…により中断』の tool 結果が突然現れる。

**詳細**:

Claude process の予期しない死（stdout EOF / read error）は PartsMerged(Error) を伴わず TurnCompleted(Interrupted{Crash, error: Some(msg)}) + Fatal のみを送る（claude/session.rs:534-568、呼び出し元 :381-395。Codex 側も codex/session.rs:400,417 で同様）。complete_turn（runtime/usecase.rs:3139）は append_final_turn_events → finalize_turn で未完了 tool への ToolCallFailed(『…により中断』) と TurnInterrupted{error} を durable log に追記し（finalization.rs:19-44）、projector が error part を合成する（projector.rs:395-398）が、これらは persist_message_parts（usecase.rs:3239-3248）で永続化されるだけで live へは emit されない（flush_streaming_update は error part 合成前 :3158 に実行済み。live emit は agent-session-state-changed のみ）。frontend の agent-session-state-changed handler（useAgentSdkListeners.ts:445-510）は exit_code!=0 / interrupted / session_state=error に対して SET_ERROR 等の表示 dispatch を行わず、chat panel に session.state を描画するコードも存在しない（ChatSessionView.tsx:1748 の error バナーは invoke 失敗用 reducer error、:1755 は contextCarry failed バナー）。Fatal が idle 中に起きた場合（Codex thread start/resume の JSON-RPC error codex/convert.rs:58-67。start_session が idle 中に ensure_runtime するため実際に発生しうる）は SessionState::Error を永続化して state change を出すだけで（runtime/usecase.rs:2572-2597）、chat panel には何も表示されない。訂正2点: (1) 原指摘タイトルの「timeout」は現コードでは発生しない — InterruptReason::Timeout の生成元は存在せず（#1381 で無出力 timeout は非破壊 signal 化、usecase.rs:4133-4134 のマッピングは残骸）、本問題の live 経路は crash と Fatal のみ。(2) 完全に無表示ではなく、workspace sidebar の AgentStateIcon（WorkspaceList.tsx:210、status.rs:270-280 の derive_agent_state 経由）だけは error 色に変わる。ただし chat panel 本体には spinner 停止以外の変化がなく、reload（または session 再選択による getSession 再読込）後にのみ AgentErrorBlock（ChatSessionView.tsx:542-547）と失敗 tool 結果が出現する、という核心症状は正確。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/session.rs:557`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2536`
- `src-tauri/src/usecase/agent_session/event_log/projector.rs:395`
- `src/hooks/useAgentSdkListeners.ts:486`
- `src/components/panels/AgentChatPanel/ChatSessionView.tsx:1748`

### FE-3: streaming 中の再 hydration（session 切替復帰 / reload）で最大 ~1 秒分の streamed text が欠落し、seq 無視のため自己修復しない

- 種別: ライフサイクルで失われる（lossy-lifecycle） / 重大度: **medium**

**ユーザー可視の症状**: streaming 中に別 session を見て戻ると、生成中のメッセージが文の途中で欠けて（単語が飛んで）繋がる。tool を使わない純テキスト回答だと turn が終わっても欠けたまま表示され続け、reload するとなぜか直る。

**詳細**:

get_session は永続化済み page のみを返し in-memory の streaming_parts を merge しない（runtime/usecase.rs:951-1014）。streaming parts の persist は 1 秒間隔（streaming.rs:6 STREAMING_PERSIST_INTERVAL、usecase.rs:2794-2799）、emit は 33ms 間隔の append-only delta で、text/thinking のみの間は snapshot が発生しない（usecase.rs:2664-2683, streaming.rs:77-86）。snapshot 送出は seq==0（turn 開始）・非 text/thinking part 到着・PermissionRequested（Immediate mode, usecase.rs:2444-2448）・emit 連続失敗 fallback（usecase.rs:2916-2929）のみ（usecase.rs:2779）。session 切替復帰（selectSession は常に getSession→UPSERT_SESSION、useAgentChat.ts:646-669）では upsertSession が incoming messages 非空時に既存 messages を置換する（agentChatReducer.ts:289-293）ため、EVICT_SESSION_BODY 済みか否かに関わらず最大 ~1 秒＋fetch レイテンシ分 stale な persisted parts が土台になり、以後の append delta 継ぎ足しで最終 persist〜UPSERT 間の text が文中欠落として固定される。webview reload 後も同様。reducer は delta の seq を完全に無視し（agentChatReducer.ts:667-682 で action.seq 未使用、useAgentSdkListeners.ts:392-425 も gap 検出なし。この seq 無視は useAgentSdkListeners.test.ts:598 のテストで意図的に固定されている）、backend にも再購読時の snapshot 再送機構がない。tool を使う turn では次の非 text part 到着時の snapshot で自己修復するが、text/thinking のみの turn では turn 終端でも修復されない: complete_turn の flush_streaming_update(ctx, session_id, true)（usecase.rs:3158）の force は persist のみを強制し snapshot emit を行わず、frontend の turn 完了処理（useAgentSdkListeners.ts:486-510）は MARK_AGENT_TURN_COMPLETED（timestamp 更新のみ）と refreshSessions()（listSessions の summary 取得のみで body を refetch しない、useAgentChat.ts:573-632）のため、次に get_session が走る操作（reload、別 session への切替→再復帰）まで欠落が表示され続ける。permanent なデータ損失ではない（complete_turn が最終 parts を persist する、usecase.rs:3239-3248）が、表示上の文中欠落が turn を跨いで残存する。

**証拠**:

- `src/hooks/agentChatReducer.ts:667`
- `src/hooks/useAgentSdkListeners.ts:392`
- `src-tauri/src/usecase/agent_session/runtime/streaming.rs:6`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2665`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2779`

### FE-4: token usage / context window が backend から完全に配管済みなのに UI のどこにも表示されない

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: トークン消費・コスト・context window の逼迫具合がどこにも見えない。compaction の system notification が出るまで context が埋まっていることに気づけない。

**詳細**:

backend は turn ごとに TokenUsage（inputTokens/outputTokens/totalTokens?/contextWindowTokens?, types/session.ts:338-343）を算出し、runtime/usecase.rs:2507 の notifier.token_usage_updated 経由で presenter（src-tauri/src/adaptor/presenter/agent_session.rs:171-178）が agent-turn-usage-updated を emit する。get_session 応答にも latestTokenUsage が含まれる（useSessionStore.ts:123,249 / useAgentChat.ts:274,330）。frontend は listener で受信し（useAgentSdkListeners.ts:163-186）、reducer に保存し（agentChatReducer.ts:621-628）、getSessionLatestTokenUsage として公開する（useAgentChat.ts:1545-1549, 1627）が、これを消費する component が一つも存在しない（src 全体 grep で consumer は AgentChatPanel.test.tsx:279 の context mock のみ。BoundSessionChat.tsx の useAgentChatContext destructure にも含まれず ChatSessionView へ渡していない。types/workflow.ts:217 の totalTokenUsage も同様に型定義・テストフィクスチャ以外で未消費）。backend から reducer までの配管全体にテストが存在する一方、表示だけが欠けており、意図的な非表示を示す仕様・テストはない。

**証拠**:

- `src/hooks/useAgentSdkListeners.ts:163`
- `src/hooks/useAgentChat.ts:1545`
- `src/hooks/agentChatReducer.ts:621`
- `src/components/panels/AgentChatPanel/BoundSessionChat.tsx:63`

### FE-5: error banner が session 横断のグローバル値で、無関係な UPSERT_SESSION（他 session の turn-prepared 等）で無言クリアされる

- 種別: 見せ方（presentation） / 重大度: **medium**

**ユーザー可視の症状**: 送信失敗の赤 banner が、裏で動いている別 session が次の turn を始めた瞬間に勝手に消える。逆に別 session のエラーが今見ている session のパネルに出る。

**詳細**:

AgentChatState.error は単一のグローバルフィールドで（agentChatReducer.ts:40）、BoundSessionChat は context の error をそのまま表示対象 session の ChatSessionView に渡す（BoundSessionChat.tsx:79, 209）。AgentChatPanel と WorkflowView は MainLayout の center panel で排他だが、WorkflowView は workflow node ごとの pane grid で複数の BoundSessionChat を同時 mount するため（WorkflowView.tsx:389-431）、session A の送信失敗 banner が同時表示中の session B の pane にも表示される。さらに reducer の upsertSession は無条件に error:null を返し（agentChatReducer.ts:315）、useAgentSdkListeners.ts:193-200 の agent-turn-prepared listener は worktreePath 一致のみで UPSERT_SESSION を dispatch するため、同一 worktree 内の別 session の turn 開始（backend の queued turn drain は runtime/usecase.rs で turn_prepared を emit するのでユーザー操作なしに発火）や loadSession 成功（useAgentChat.ts:677 等）が起きた瞬間、表示中の『メッセージ送信に失敗』等の banner がユーザー操作なしに消える。agentChatReducer.test.ts:272 の「UPSERT_SESSION clears error」テストは reducer 単体の clear 挙動を pin しているが、cross-session の無言クリアを仕様として固定するものではない。

**証拠**:

- `src/hooks/agentChatReducer.ts:315`
- `src/hooks/useAgentSdkListeners.ts:197`
- `src/components/panels/AgentChatPanel/BoundSessionChat.tsx:210`
- `src/components/panels/AgentChatPanel/ChatSessionView.tsx:1748`

### FE-6: Task（subagent）配下の thinking part と未 pair の tool_result が展開しても描画されない

- 種別: 見せ方（presentation） / 重大度: **low**

**ユーザー可視の症状**: Task を展開しても subagent の思考過程が全く見えず、一部の tool 結果も表示されない。subagent のテキスト報告は 200 文字で切れ、全文を見る方法がない。

**詳細**:

Claude の subagent 出力は parent_tool_use_id 付きの thinking/text/tool_use/tool_result として届く（thinking/text は stream delta 経由 claude/convert.rs:243-259、tool_use は :288-293、tool_result は :322-332。CLI は --include-partial-messages で起動 process.rs:291）。これらは streaming（runtime/usecase.rs:2991-3024）・durable replay（part_events.rs / projector.rs:103-126）の両経路で parentToolUseId（camelCase serialize、usecase/agent_session/session/mod.rs:171-177）を保持したまま frontend の msg.parts に到達し、toolPairing.ts:144-151 で task child に分類され、ChatSessionView.tsx:501 で通常レンダリングから除外される。TaskToolActivity の child switch（ActivityLog.tsx:951-988）は text / tool_use / error のみ処理し default で null を返すため、subagent の thinking part と、同一 message 内に対応 tool_use がない tool_result（例: subagent 内 TodoWrite は convert.rs:279-286 で ToolUse part に変換されないため、その tool_result は必ず未 pair になる）は展開表示でも無言で消える。text child は 200 文字 truncate + max-h-24 overflow-hidden で展開手段がない（ActivityLog.tsx:953-961）。さらに TaskGroup.resultIndex（Task 自体の tool_result = subagent 最終報告）は toolPairing.ts:170 で計算されるが、どのコンポーネントからも参照されておらず描画されない。

**証拠**:

- `src/components/panels/AgentChatPanel/ActivityLog.tsx:986`
- `src/components/panels/AgentChatPanel/toolPairing.ts:144`
- `src/components/panels/AgentChatPanel/ChatSessionView.tsx:501`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:282`

### FE-7: 解決済み permission チップが description / decisionReason を捨て、拒否理由が見えない

- 種別: 捨てている（dropped） / 重大度: **low**

**ユーザー可視の症状**: 『Bash — denied』のようなチップだけが残り、なぜ拒否されたのか（ルール名や CLI の説明文）が一切表示されない。

**詳細**:

backend は permission に description・decision_reason を載せる（claude/permission.rs:71-74、Claude の permission_denied は CLI の説明文を description に格納 claude/convert.rs:140-159、input は空 json!({}))。値は event_apply.rs:41-42 の permission_request_msg 経由で PermissionRequestMsg に引き継がれ camelCase で frontend に届き、durable event log（part_events.rs:133-143）にも丸ごと保存されるため replay 後も存在する。frontend の PermissionRequest 型には description/decisionReason が定義されている（types/session.ts:117-118）が、decisionReason はフロント全体で一度も描画されず（参照は型宣言のみ）、description は pending の generic 分岐（PermissionDialog.tsx:918-921）と resolved ask_user_question の質問文 fallback（PermissionDialog.tsx:606）でしか使われない。resolved の tool 分岐（PermissionDialog.tsx:588-696）は title/displayName/toolName + status のラベルのみで、詳細展開は presentation.hasResolvedDetail に依存する。permission_denied part は resolved-denied で到着し input が空のため has_resolved_detail=false（adaptor/controller/command/agent_session/permission.rs:198-202 の input_has_object_fields）となり展開ボタン自体が disabled になる。さらに仮に展開できても ResolvedDetail の generic 分岐（PermissionDialog.tsx:291-298）は input JSON しか描画しないため、拒否理由には構造上到達不能。ActivityLog の permission_result summary（session/mod.rs:1188-1206）も answers か status 文字列のみで理由を含まず、代替表示経路は存在しない。

**証拠**:

- `src/components/panels/AgentChatPanel/PermissionDialog.tsx:588`
- `src/components/panels/AgentChatPanel/PermissionDialog.tsx:918`
- `src/types/session.ts:118`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:140`
- `src-tauri/src/infrastructure/agent_session/claude/permission.rs:74`

## RG: 参照実装（Vibe Kanban / ACP）との語彙ギャップ

参照実装が一級で扱うのに Releash の語彙（AgentRuntimeEvent / MessagePart）に存在しない・弱い概念。

### RG-1: Codex の reasoning/thinking が全経路で捨てられる（delta 未処理＋完了時の抽出パスがスキーマ不一致）

- 種別: 捨てている（dropped） / 重大度: **high**

**ユーザー可視の症状**: Codex セッションでは thinking が一切チャットに表示されない。長い推論フェーズ中はどのイベントも変換されないため（KeepAlive も出ない）、ユーザーには「エージェントが固まって何もしていない」ように見え、最終回答だけが突然現れる。Claude では thinking がストリーム表示されるため、backend を切り替えると挙動が不揃いになる。

**詳細**:

指摘の detail は正確。検証で得た補強点のみ追記: (a) スキーマ不一致は Releash 自身が wire 契約を検証したバージョン（codex-cli 0.139.0、wire.rs:1-2 の検証ノート）の openai/codex rust-v0.139.0 タグで直接確認済み — v2 ThreadItem::Reasoning は { id: String, summary: Vec<String>, content: Vec<String> }（serde tag "reasoning"、text フィールドなし）であり、convert.rs:348 の get_string(item, &["summary","text"]) は summary が JSON 配列のため常に None。(b) wire.rs:17 の検証ノート自身が 0.139.0 の TurnItem に Reasoning variant が存在することを記録しており、既知の契約要素を取りこぼしている。(c) item/started の reasoning item も item_started_parts（convert.rs:305-338）で item_tool_name が None を返し何も生成しない。(d) この no-op 挙動を固定するテストは convert.rs に存在しない。(e) 修正は二点: item/reasoning/* delta 通知を NOTIFY 定数と convert_notification に追加して MessagePart::Thinking としてストリームすること、および item_completed_parts の reasoning 分岐を summary/content の Vec<String> を join して読むよう修正すること（delta 対応済みなら completed 側は dedup 考慮が必要）。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:347`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:216`
- `src-tauri/src/infrastructure/agent_session/codex/wire.rs:50`
- `openai/codex codex-rs/app-server-protocol/src/protocol/v2/item.rs (Reasoning { summary: Vec<String>, content: Vec<String> }, rust-v0.63.0〜rust-v0.143.0-alpha.38 で確認)`
- `openai/codex codex-rs/app-server-protocol/src/protocol/common.rs (ReasoningTextDelta => "item/reasoning/textDelta" ほか)`

### RG-2: Codex の plan/todo（turn/plan/updated・item/plan/delta・plan item）が全て破棄され、TodoListSnapshot が Claude 専用になっている

- 種別: 捨てている（dropped） / 重大度: **high**

**ユーザー可視の症状**: Codex がプラン（update_plan）を作成・更新しても、チャットにも todo フッターにも何も出ない。Claude セッションでは todo チェックリストが表示されるのに Codex では機能ごと黙って消えるため、「Codex は計画を立てていない」ように見える。human checkpoint でプランを見て承認するという workflow 判断材料が Codex では欠落する。

**詳細**:

Codex app-server（Releash が検証対象とする codex-cli 0.139.0、wire.rs:1-2）は plan を3経路で一級配信する: (a) `turn/plan/updated`（TurnPlanUpdatedNotification { thread_id, turn_id, explanation: Option<String>, plan: Vec<TurnPlanStep{step, status: pending/inProgress/completed}> }、v2/turn.rs）— `update_plan` ツール（TODO/checklist ツール、core/src/tools/spec_plan.rs:646 で無条件登録）実行時に app-server/src/bespoke_event_handling.rs:1239-1292 が必ず発信、(b) experimental な `item/plan/delta`（PlanDeltaNotification）、(c) experimental な ThreadItem::Plan（v2/item.rs:240、collaborationMode "plan" の提案プラン用。update_plan は Plan mode では禁止: handlers/plan.rs:81）。Releash は experimentalApi: true で initialize し（wire.rs:7）plan mode も使用する（session.rs:564）が、codex/wire.rs:50-61 に plan 系 NOTIFY 定数がなく、convert.rs の convert_notification（104-218）は catch-all（216）で (a)(b) を破棄、item_started_parts（317-319, 336）/item_completed_parts（388）は (c) を破棄する。全 notification は session.rs:367 の convert_jsonrpc_message 経由のみで、別の処理経路は存在しない。MessagePart::TodoListSnapshot（usecase/agent_session/session/mod.rs:250-252）の生成元は claude/convert.rs:284（TodoWrite）のみで、UI の TodoListFooter（ChatSessionView.tsx:1807、1324 の latestTodoListSnapshot 供給）は part が来ないと表示されない。なお wire.rs:16-20 の 0.139.0 検証メモは「legacy todo_list item は契約外」と TurnItem variant のみを確認しており、notification 経路の存在（0.139.0 で確実に発信される）が検証から漏れている。結果: Codex が update_plan でプランを作成・更新しても turn/plan/updated が黙って捨てられ、チャットにも todo フッターにも何も出ない。Claude セッションでは同機能が動作するため、Codex だけプラン可視性が機能ごと欠落し、human checkpoint でプランを見て判断する workflow の判断材料が失われる。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:216`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:284`
- `src/components/panels/AgentChatPanel/ChatSessionView.tsx:1807`
- `openai/codex codex-rs/app-server-protocol/src/protocol/v2/turn.rs (TurnPlanUpdatedNotification / TurnPlanStep / TurnPlanStepStatus)`
- `ACP schema/v1 SessionUpdate::plan + PlanEntry{content, priority, status}`

### RG-3: turn 終了理由（stop reason）の語彙が実質未配線 — max turns / refusal / cancelled が区別されず workflow failure signal も発火しない

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: Claude が max turns 到達や実行時エラーで止まっても、is_error 経由の汎用テキスト（result 本文がなければ「Claude turn failed」）しか出ず、「上限に達しただけなので続行すればよい」のか「本当に失敗した」のかをユーザーが判別できない。refusal も普通の完了と同じに見え、workflow 層の失敗シグナル（ModelRefusal）も一切上がらないため自動リトライ/承認判断の材料にならない。

**詳細**:

Releash の TurnStopReason は Refusal 1 変種のみ（domain/agent_session/entities/turn.rs:20-25、dead_code allow 付き。コメント自体が「conversion fixtures and workflow failure projection, not every production path」と fixture 限定を自認）で、production で設定する箇所がゼロ。claude/convert.rs の convert_result（205-234行）は Claude CLI result イベントの subtype（success / error_max_turns / error_during_execution）を読まず、is_error bool のみで分岐し、成功時は常に stop_reason: None（229行）、エラー時は result_error_text（532-543行）の汎用テキスト（errors 配列→result 文字列→「Claude turn failed」フォールバック）に畳む。codex/convert.rs も completed 時は常に stop_reason: None（210行）。Refusal を構築するのはテストのみ（claude/session.rs:731 は #[cfg(test)] mod tests 内、event_log/tests.rs:61、runtime_engine_impl/tests.rs:7184）。その結果、projector.rs:906-910 の workflow_failure_signal_from_stop_reason → AgentTurnFailureSignal::ModelRefusal は production で決して発火しない死んだ経路であり、下流の failure_policy.rs:142 が ModelRefusal を partial failure として特別扱いする配線が実在するのに到達不能。frontend にも stopReason を扱うコードは存在せず、UI 上 max turns 到達・実行時エラー・refusal が区別されない。参照実装比: ACP は StopReason = end_turn / max_tokens / max_turn_requests / refusal / cancelled を一級で規定し、Claude CLI result にも subtype がある。ただし cancelled 相当は Releash では TurnResult::Interrupted（claude は abort 中 Completed→Interrupted 正規化、codex は "interrupted" status→InterruptReason::Abort、codex/convert.rs:205-208）として別途一級で区別されており、未配線なのは completed/failed turn 内の max_turns / error_during_execution / refusal の区別である。

**証拠**:

- `src-tauri/src/domain/agent_session/entities/turn.rs:20`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:228`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:209`
- `src-tauri/src/usecase/agent_session/event_log/projector.rs:906`
- `src-tauri/src/infrastructure/agent_session/claude/session.rs:731`
- `ACP schema/v1 StopReason = [end_turn, max_tokens, max_turn_requests, refusal, cancelled]`

### RG-4: ツール実行結果の状態語彙が is_error bool に潰れ、denied / timed-out / interrupted が「失敗」と同一表示になる

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: ユーザーが自分で拒否したツール、中断で打ち切られたツール、実際にエラーで失敗したツールが、履歴上すべて同じ「エラー（赤）」表示になる。後からセッションを見返したとき「このコマンドは壊れていたのか、自分が止めたのか」が本文テキストを読まないと分からず、実行履歴の監査性が参照実装より低い。

**詳細**:

Releash のツール実行結果の状態語彙は is_error: bool に潰れており、denied / interrupted / failed がツールエントリの status レベルでは区別されない。確認済みの経路: (a) MessagePart::ToolResult は domain（src-tauri/src/domain/agent_session/entities/message_part.rs:23-30）・usecase（src-tauri/src/usecase/agent_session/session/mod.rs:199-219）両定義とも is_error: bool のみ。(b) durable event は ToolCallSucceeded/ToolCallFailed の二値（src-tauri/src/usecase/agent_session/event_log/events.rs:142-167）。(c) turn 中断時の未完了ツールは finalization.rs:19-27 で一律 ToolCallFailed（content「{reason} により中断」）。(d) Codex の status "declined"（ユーザー拒否。codex 0.142.5 バイナリで inProgress/completed/failed/declined の status enum を確認）は codex/convert.rs:373-380（mcpToolCall）・381-387（dynamicToolCall）・508-516（command_is_error）で is_error=true に畳まれ、fileChange も 365-371 で status≠completed を一律 error 扱い。(e) codex 0.142.5 の protocol に実在する通知 `item/mcpToolCall/progress` は wire.rs に定数がなく convert_notification の catch-all（convert.rs:216）で破棄され、長時間 MCP ツールの進行状況が表示されない。(f) frontend の ToolStatusIcon（src/components/panels/AgentChatPanel/ActivityLog.tsx:183-187）は isError の二値で赤 XCircle/チェックのみ。参照実装（vibe-kanban ToolStatus::Denied/TimedOut、ACP ToolCallStatus）と比べ status 語彙が一級でない点は正確。ただし緩和要素あり: permission 経由の拒否は durable に PermissionResolved{decision: Denied} が残り（projector.rs:710-711）、frontend は解決済み permission card を「{tool} — denied」ラベルで履歴に表示する（PermissionDialog.tsx:588-597、ChatSessionView.tsx:574）ため、拒否は隣接カードから本文を読まずに判別可能。中断ツールも content テキスト「…により中断」で判別可能。ToolCallFailed を根拠に動作する自動化経路は現状なく（消費先は projector の表示投影のみ）、実害はツールエントリのアイコン/機械可読 status の監査性ギャップに留まる。

**証拠**:

- `src-tauri/src/usecase/agent_session/event_log/events.rs:142`
- `src-tauri/src/usecase/agent_session/event_log/finalization.rs:19`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:508`
- `BloopAI/vibe-kanban crates/executors/src/logs/mod.rs:125 (ToolStatus::Denied{reason}/TimedOut/PendingApproval)`
- `ACP schema/v1 ToolCallStatus = [pending, in_progress, completed, failed]`

### RG-5: todo リストの in_progress / priority が completed:bool に潰れ、「今どの作業をしているか」が消える

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: todo フッター/チェックリストで「完了 or 未完了」しか見えず、エージェントが今まさに取り組んでいる項目（in_progress、Claude Code CLI ならスピナー付きで出る項目）がハイライトされない。長い turn の進捗監視で「残り項目のどれをやっているのか」が分からない。

**詳細**:

Releash 側: TodoListItem は text + completed: bool のみ（domain/agent_session/value_objects/todo_list_item.rs:2-5、usecase/agent_session/session/mod.rs:55-58 に同形の二重定義）。claude/convert.rs:440-462 の todo_items_from_input が status == "completed" だけを見て bool に変換するため、in_progress と pending の区別・activeForm・priority が変換時点で不可逆に失われる。さらに convert.rs:279-286 で TodoWrite の tool_use は ToolUse part として emit されず Text("Updated todo list") + TodoListSnapshot に置換されるため、raw input が tool call 詳細としても残らず、durable event（event_log/events.rs:217-220 TodoListSnapshotRecorded）にも flattened 形でしか記録されない。frontend 型（src/types/session.ts:193-195）と TodoListFooter（ChatSessionView.tsx:229-295）も completed 2値のみを扱い、in_progress 表示手段が存在しない。なお Codex 側は plan/todo イベントの変換処理自体が存在しない（codex/convert.rs に plan/todo ハンドリングなし）。

**証拠**:

- `src-tauri/src/domain/agent_session/value_objects/todo_list_item.rs:2`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:440`
- `BloopAI/vibe-kanban crates/executors/src/logs/mod.rs:159 (TodoItem{content, status, priority})`
- `ACP schema/v1 PlanEntry{content, priority: high|medium|low, status: pending|in_progress|completed}`

### RG-6: Codex の運用系通知（warning / configWarning / deprecationNotice / model/rerouted / account/rateLimits/updated）が無音破棄され、受け皿となる語彙も無い

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: Codex がモデルを別モデルへ reroute した・設定に問題がある・レート制限に近づいている、といった状況でチャットに何も出ない。ユーザーには「なぜ急に応答品質が変わったのか」「なぜ遅い/失敗するのか」が説明されず、原因不明の不安定さとして体験される。

**詳細**:

参照実装側: Codex app-server は Warning => "warning"、ConfigWarning => "configWarning"、DeprecationNotice => "deprecationNotice"、ModelRerouted => "model/rerouted"、GuardianWarning => "guardianWarning"、AccountRateLimitsUpdated => "account/rateLimits/updated" を server notification として定義する（openai/codex codex-rs/app-server-protocol/src/protocol/common.rs の server_notification_definitions マクロ、fetch で確認済み）。Releash は同じ v2 app-server プロトコル（wire.rs:106-122 で initialize + experimentalApi: true、thread/start / turn/start を使用）で通信しており、これらの notification は同一 stdout ストリームで到着しうる。Releash 側: codex/convert.rs の convert_notification（104-218行）が処理するのは wire.rs:50-61 の 12 メソッドのみで、上記 6 種は catch-all（convert.rs:216 `_ => Vec::new()`）で破棄される。unhandled error response には log::warn がある（convert.rs:83-86）のに対し、未知 notification はログすら出ない完全な無音破棄。session.rs の read_loop（349-391行）が唯一の消費経路で、notification は message_kind 分岐（session.rs:365 `_ => {}`）でも無処理のため、AgentRuntimeEvent が生成されず durable event log にも frontend にも一切届かない。受け皿の語彙も無い: SystemNotificationType は domain（domain/agent_session/value_objects/system_notification_type.rs:2-4）・usecase（usecase/agent_session/session/mod.rs:153-155）両定義とも Compaction 1 変種のみで、MessagePart（domain/agent_session/entities/message_part.rs:8）にも警告・運用通知を表現する変種が存在しない（Error に流すことすらしていない）。なお Releash が処理する thread/tokenUsage/updated は token 使用量通知であり、account/rateLimits/updated（レート制限枠）の代替にはならない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:216`
- `src-tauri/src/usecase/agent_session/session/mod.rs:151`
- `openai/codex codex-rs/app-server-protocol/src/protocol/common.rs (Warning/ConfigWarning/DeprecationNotice/ModelRerouted/AccountRateLimitsUpdated)`
- `BloopAI/vibe-kanban crates/executors/src/logs/mod.rs:84 (SystemMessage / ErrorMessage{SetupRequired})`

### RG-7: tool_result 内の image ブロックが text 抽出で捨てられ、画像を返すツール結果が空欄になる

- 種別: 捨てている（dropped） / 重大度: **medium**

**ユーザー可視の症状**: スクリーンショットや図を返す MCP ツール（browser 系、Read での画像読込等）の結果がチャット上で空のツール結果として表示される。エージェント本体は画像を見て判断しているのに、ユーザーは同じ判断材料を見られず、承認判断（この UI で合っているか等）ができない。

**詳細**:

tool_result 内の image ブロックが text 抽出で捨てられ、画像を返すツール結果がユーザーに見えない。MessagePart::Image / ImageRef の語彙自体は存在する（usecase/agent_session/session/mod.rs:263-272、domain/agent_session/entities/message_part.rs:62）が、生成元はユーザー添付経路のみ（送信時の human_parts: runtime/usecase.rs:3722-3738、保存時の外部化/ハイドレーション: attachment_blob.rs:78-129、およびそれら起点の durable event 再生: part_events.rs:192-201 / projector.rs:346 / event_apply.rs:190）で、backend converter からは一切生成されない。claude/convert.rs:464-479 の tool_result_content は content 配列から text フィールド（または文字列要素）しか抽出せず image ブロックを黙って落とす。画像のみの結果は空文字列の ToolResult になる（convert.rs:315-332）。codex/convert.rs:518-535 の mcp_result_content も text 系のみ抽出し、画像のみの MCP 結果は None となり convert.rs:373-380 で "Codex MCP tool call completed." という generic 文言に置換される（空欄ではなく無内容の定型文）。text+image 混在の場合は text だけが残り、画像が存在した痕跡すら表示されない。ユーザー可視症状: スクリーンショットや図を返す MCP ツール（browser 系等）や画像ファイルの Read の結果が、Claude ではチャット上で空のツール結果、Codex では定型完了文として表示される。エージェント本体は画像を見て判断しているのに、ユーザーは同じ判断材料を見られず承認判断ができない。この挙動を意図仕様として固定するテストは存在しない。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:464`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:518`
- `src-tauri/src/usecase/agent_session/session/mod.rs:263`
- `ACP schema/v1 ToolCallContent/ContentBlock (image を含む) + ToolCallUpdate.rawOutput`

### RG-8: コマンドの exit code が構造化されず is_error bool と本文文字列に潰れる

- 種別: 扱いが違う（divergent） / 重大度: **low**

**ユーザー可視の症状**: 失敗したコマンドが「なぜ・どの exit code で」失敗したかを UI がバッジ等で示せず、ユーザーは長い出力テキストを目視で追うしかない。出力が空のまま失敗した場合（timeout kill 等）は手掛かりがほぼゼロになる。

**詳細**:

Codex: codex/convert.rs の command_is_error（508-516）は item.exitCode を is_error 判定にだけ使って数値を捨て、command_result_content（496-506）は aggregatedOutput 文字列のみ（空なら「Codex command `X` finished with status Y.」の合成文で exit code を含まない）。ToolResult part（usecase/agent_session/session/mod.rs:199）にも durable event ToolResultRecorded（event_log/events.rs:168）にも ToolOutputSummary にも exit code フィールドはなく、ToolUse input（convert.rs:321-325）は started 時の command/cwd/status のみで completion 時に更新されない。Claude 側 Bash も tool_result の text にしか残らない（claude/convert.rs:315-332, 464-478）。frontend は ActivityLog.tsx:677-684 で「exit N」バッジを描画するが、その値は extractExitCode（ActivityLog.tsx:157-159）が結果テキストを正規表現 /exit code\s+(-?\d+)/i でスクレイピングして得るもの。Claude Bash 失敗時は CLI が埋め込む "Exit code N" テキストに依存して表示され、Codex では生のコマンド出力にも合成文にもその句が無いためバッジは事実上表示されない。ユーザー影響: Codex の失敗コマンドは exit code がどこにも表示されず、出力が空のまま失敗した場合（timeout kill 等）は status 名だけの合成文しか手掛かりがない。Claude 側もバッジが CLI のテキスト形式変更で壊れうる脆弱な経路。参照実装（vibe-kanban の CommandRunResult/CommandExitStatus、ACP の rawOutput）は exit code を構造化して保持する。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:508`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs:496`
- `src-tauri/src/usecase/agent_session/session/mod.rs:199`
- `BloopAI/vibe-kanban crates/executors/src/logs/mod.rs:43 (CommandExitStatus / CommandRunResult)`

### RG-9: コスト（total_cost_usd）が TokenUsage 語彙に存在せず破棄される

- 種別: 捨てている（dropped） / 重大度: **low**

**ユーザー可視の症状**: セッション・turn ごとの金額コストを UI に出せない。API キー（従量課金）で使うユーザーは「この workflow 実行にいくらかかったか」を Releash 内で確認できず、CLI 側の表示に頼ることになる。

**詳細**:

Claude CLI の result イベントは total_cost_usd（および modelUsage 内の per-model costUSD、duration_ms、num_turns）を返すが、Releash はこれらを完全に破棄する。TokenUsage は input/output/total/context_window のみ（domain/agent_session/entities/turn.rs:36-42）で、claude/convert.rs の token_usage()（494-530行）は costUSD を読まず、convert_result()（205-234行）も total_cost_usd/duration_ms/num_turns を参照しない。テストフィクスチャ（convert.rs:708 "costUSD": 0）に実データの痕跡がある。durable event の TurnTokenUsage（event_log/events.rs:97-100）と frontend の TokenUsage 型（src/types/session.ts:338、src/types/workflow.ts:11）にも cost はなく、全経路で欠落。補足: duration_ms は TurnStarted/TurnCompleted の at タイムスタンプから間接導出可能だが、cost は完全に復元不能。

**証拠**:

- `src-tauri/src/domain/agent_session/entities/turn.rs:36`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:494`
- `ACP schema/v1 UsageUpdate{used, size, cost}`

## ST: 構造・基盤上の問題（前段の構造調査より）

CL〜RG の監査（意味的・観測可能な問題に限定）の対象外だが、それらの問題を生み、再発の検出を不能にしている構造要因。特定の単一症状ではなく問題クラス全体の原因であるため、種別は `structural` とし、関連する問題 ID を併記する。milestone 84 の解消対象に含める。

### ST-1: Codex wire 層が非型付け（手書き JSON-RPC decode、公式 protocol クレート未使用）

- 種別: 構造要因（structural） / 重大度: **high**

**影響**: 未処理 notification・フィールド名不一致が全て「無言破棄」になる。CX 群 11 件の大半（CX-1 の question id 欠落、CX-4 の tokenUsage フィールド名不一致、CX-3/CX-5/CX-7 の未購読 notification 等）の構造的原因。

**詳細**:

`app_server.rs` の `decode_jsonrpc_line`（app_server.rs:130-131）は `serde_json::Value` を返すのみで、convert.rs（1,176 行）が Value からの手動抽出で全 notification / item を解釈している。openai/codex は公式の `codex-app-server-protocol` / `codex-protocol` クレートを提供しており（参照実装 Vibe Kanban はタグ固定 git 依存で利用: crates/executors/Cargo.toml:39-40）、これを使えばフィールド名不一致は deserialize 失敗として顕在化し、新規 notification の取りこぼしは enum の非網羅 match としてコンパイル時に検出できる。公式クレートのタグ固定導入は合意済み。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/codex/app_server.rs:130-131`
- `src-tauri/src/infrastructure/agent_session/codex/convert.rs（全域が Value 手動抽出）`
- `BloopAI/vibe-kanban crates/executors/Cargo.toml:39-40（公式クレート利用例）`

### ST-2: Claude wire 層も非型付け（stream-json の手動 Value 解釈）

- 種別: 構造要因（structural） / 重大度: **high**

**影響**: 未知メッセージ type の無言破棄（CL-1 の control_cancel_request 等）、content block 種別の取りこぼし（CL-6 の image、redacted_thinking 等）、フィールドの読み忘れ（CL-3/CL-4/CL-5）が検出不能。CL 群 7 件の構造的原因。

**詳細**:

claude/wire.rs は文字列定数の列挙のみで（wire.rs:13-27）、convert_claude_message は `serde_json::Value` を手動で辿り、catch-all `_ => ClaudeConversion::none()`（convert.rs:84-85）が未知 type を無言で握りつぶす。契約元である Claude Agent SDK の型定義（sdk.d.ts の StdoutMessage union）に対応する typed model が Rust 側に存在せず、SDK 更新時の差分検出手段が無い。

**証拠**:

- `src-tauri/src/infrastructure/agent_session/claude/wire.rs:13-27`
- `src-tauri/src/infrastructure/agent_session/claude/convert.rs:84-85`

### ST-3: runtime/usecase.rs が 8,380 行の god module で、RuntimeSessionPhase 遷移が暗黙的

- 種別: 構造要因（structural） / 重大度: **high**

**影響**: turn lifecycle / permission / queue / stale 監視 / lock / resume が単一ファイルに同居し、状態遷移が巨大 match の分岐に散在。RT 群・OB 群のライフサイクル問題（close 時の finalize 漏れ RT-1、queue 非永続 RT-3/OB-3、interrupt 直後の drain OB-5）を個別修正しても、遷移の全体像が明示されていないため回帰・考慮漏れが検出できない。

**詳細**:

runtime/ 配下 10,298 行のうち usecase.rs が 8,380 行を占める。RuntimeSessionPhase（Idle / Streaming / WaitingPermission 等）の遷移条件・許可される操作・turn 終端時の後始末（finalize・queue drain・permission 畳み込み）が一箇所の遷移表として存在せず、イベント処理の各分岐に埋め込まれている。直近の安定性修正（#1352, #1379, #1381）が全てこのファイルへの追記になっており、複雑性が単調増加している。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs（8,380 行）`
- `src-tauri/src/usecase/agent_session/runtime/（合計 10,298 行）`

### ST-4: 永続化失敗の握りつぶし（let _ =）

- 種別: 構造要因（structural） / 重大度: **medium**

**影響**: 永続化・投影の失敗が caller に伝搬せず、in-memory 状態と durable 状態が乖離したまま処理が続行する。RT-7（queue persist 失敗の握りつぶし）・RT-8（FinalPartsRecorded append 失敗時の上書き）として症状が確認済みのパターンの残存箇所。

**詳細**:

runtime/usecase.rs に少なくとも 3 箇所: L3366 `set_session_state` の失敗無視、L3522 `append_session_event_and_project_state` の失敗無視、L4755 `complete_pending_queue_item` の失敗無視。いずれもログには出るが、呼び出し側は成功と区別できない。全箇所の棚卸しと、失敗時の方針（リトライ / 明示エラー / 診断イベント発行）の統一が必要。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3366`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:3522`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:4755`

### ST-5: session lock の二段取得と prune skip

- 種別: 構造要因（structural） / 重大度: **medium**

**影響**: lock 保持中に別 session の操作を await する経路が追加されると deadlock し得る構造。「チャットが応答しなくなる」系の将来バグの温床（現時点で deadlock の実績確認は無し）。

**詳細**:

`acquire_session_runtime_lock`（usecase.rs:2141）は locks map の Mutex → per-session の Mutex（lock_owned、usecase.rs:2152）の二段取得。guard の Drop 時に `tokio::runtime::Handle::try_current()` が失敗すると lock エントリの prune を skip する（usecase.rs:2165 付近）ため、エントリが蓄積し得る。lock 規約（保持中に許される await の範囲）が文書化・検査されていない。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2141-2152`
- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:2165`

### ST-6: MessagePart の domain / usecase 二重定義と issues-1301 残債

- 種別: 構造要因（structural） / 重大度: **medium**

**影響**: 語彙の拡張（RG 群の解消: thinking 一級化、todo の in_progress、stop reason、exit code、cost 等）を 2 つの enum と変換層に二重実装する必要があり、片側だけ直す事故と実装コスト増の温床。

**詳細**:

`MessagePart` が domain（domain/agent_session/entities/message_part.rs:8）と usecase（usecase/agent_session/session/mod.rs:169）に二重定義され、間に写像がある。issues-1301 の残債として `#[allow(dead_code)]` が多数残存: G-1（legacy session DTO projection の折りたたみ待ちで domain entity を保持）、D-2/D-7（`BackendSessionCleared` イベントが未配線 — resume mismatch 回復経路 SD-1 の実装に必要）、F-2（fork の production coverage）等。

**証拠**:

- `src-tauri/src/domain/agent_session/entities/message_part.rs:8`
- `src-tauri/src/usecase/agent_session/session/mod.rs:169`
- `src-tauri/src/domain/agent_session/gateway.rs（D-2/D-7: BackendSessionCleared）`

### ST-7: 再発検出基盤の欠落（wire fixture replay / E2E turn / cross-backend parity テストが無い）

- 種別: 構造要因（structural） / 重大度: **high**

**影響**: 本ドキュメントの 57 件を修正しても、CLI のバージョンアップや将来の変更で同種の「無言破棄・扱いの差」が再発したとき検出する仕組みが無い。監査で発見された問題の多く（CX-1 の wire 形式誤り、CX-4 のフィールド名不一致、CX-9 の dead code 化）は、実 wire ログを流すテストがあれば機械的に検出できた。

**詳細**:

convert のテストは inline `json!` literal の単体テストのみ（claude 12 件・codex 15 件）。(a) 実セッションの wire ログ（stream-json / JSON-RPC）を fixture として `convert → AgentRuntimeEvent → projector → read model` を通す replay テスト、(b) 送信 → streaming → permission → 完了の E2E turn ライフサイクルテスト、(c) 同一概念の入力で Claude / Codex が同等のイベント列・read model になることを検証する parity テスト、のいずれも存在しない。`pnpm test:integration`（playwright）は settings / statusbar / workspace-manager のみで agent chat を扱っていない。なお codex/permission.rs:218-226 のように**誤った挙動を現行仕様として固定しているテスト**も確認されている（CX-1 参照）。

**証拠**:

- `tests/（settings.spec.ts / statusbar.spec.ts / workspace-manager.spec.ts のみ）`
- `src-tauri/src/infrastructure/agent_session/（fixture ファイル 0 件）`
- `src-tauri/src/infrastructure/agent_session/codex/permission.rs:218-226（誤挙動を固定するテスト）`

### ST-8: frontend の agent chat 状態管理の肥大（mirror 原則からの逸脱）

- 種別: 構造要因（structural） / 重大度: **medium**

**影響**: FE 群 7 件（error banner のグローバル値 FE-5、seq 無視 FE-3、hydrate 差 FE-2 等）の温床。backend read model と frontend 表示の対応が 1:1 でないため、「live と reload 後で見え方が違う」問題クラスが繰り返し発生する。

**詳細**:

useAgentChat.ts 1,631 行・agentChatReducer.ts 868 行・useAgentSdkListeners.ts 627 行（計 3,218 行）。reducer が transient event の適用順・欠落に依存する独自の状態機械を持ち、CLAUDE.md の「frontend state は backend-owned state の mirror に留める」原則に対して、get_session（durable）と streaming delta（transient）の 2 経路を frontend 側で合成している。合成規則が backend に無いため、FE-1/FE-2/FE-3 のような経路差の問題が構造的に生じる。

**証拠**:

- `src/hooks/useAgentChat.ts（1,631 行）`
- `src/hooks/agentChatReducer.ts（868 行）`
- `src/hooks/useAgentSdkListeners.ts（627 行）`

### ST-9: 不可視停止の診断が遅い（permission wait 診断閾値 60 秒）

- 種別: 構造要因（structural） / 重大度: **low**

**影響**: 「backend は待っているが UI に出ていない」系の問題（FE-1、既知問題 1 と同型）が発生したとき、診断イベントが出るまで 60 秒かかり、それ以前の停止はテレメトリに残らない。

**詳細**:

`PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD` は本番 60 秒（usecase.rs:1762-1765、test cfg は 50ms）で、チェックは usecase.rs:1785。permission 以外の待ち状態（WaitingPermission 以外の phase での無応答）には同種の診断自体が無い。不可視停止クラスの問題を修正した後も、検出手段がこの 1 つでは再発時の観測が困難。

**証拠**:

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs:1762-1785`


## 付録 A: 入力カバレッジ表

各領域の調査エージェントが作成した「入力の全体像 × 処理有無」の対応表。個別問題に挙がっていない破棄項目もここに記録されている。

### CL: Claude 入力側の「捨て」

```text
Claude stream-json wire type × 処理有無（convert.rs / session.rs / process.rs 精読結果）:
| wire type / subtype | 処理 | 備考 |
| system/init | 部分変換 | session_id・slash_commands のみ。mcp_servers(status)・tools・model・permissionMode・output_style 破棄 |
| system/status | 部分変換 | permissionMode→PermissionModeChanged（ただし "plan" は None で破棄）、status==compacting→Compaction notification。それ以外は message/content 文字列を MessagePart::Error として表示（情報系も赤エラー扱い）、文字列なしは無言破棄 |
| system/compact_boundary | 変換 | Compaction completed notification |
| system/permission_denied | 変換 | Resolved(Denied) の PermissionRequested |
| system/task_started・task_progress・task_updated・task_notification | 変換 | TaskStatus part（status/description/patch/summary） |
| system/その他 subtype | 部分変換 | message/content があれば Error part、なければ無言破棄・ログなし |
| stream_event/content_block_delta(text_delta) | 変換 | Text part |
| stream_event/content_block_delta(thinking_delta) | 変換 | Thinking part |
| stream_event/content_block_delta(input_json_delta・signature_delta・citations_delta) | 破棄 | 無言 |
| stream_event/message_start・message_delta・message_stop・content_block_start・content_block_stop | 破棄 | 無言。message_delta の stop_reason と usage、content_block_start の redacted_thinking を含む |
| assistant | 部分変換 | tool_use のみ（TodoWrite は Text+TodoListSnapshot に特別扱い）。text・thinking・redacted_thinking・image ブロックと message.usage を破棄（本文は delta 単独依存） |
| user | 部分変換 | tool_result のみ。content 配列は text 項目のみ連結（image 等破棄）、is_error は保持。user text ブロック（local command stdout・CLI 合成テキスト）破棄 |
| result | 部分変換 | is_error・errors[]/result（エラー文言）・usage/modelUsage（cache 込み input/output/contextWindow）のみ。subtype・total_cost_usd・duration_ms・duration_api_ms・num_turns・permission_denials・costUSD 破棄。stop_reason 常に None |
| control_request/can_use_tool | 変換 | auto-allow 応答 or PermissionRequested。tool_name 欠落は deny 応答 |
| control_request/その他 subtype | 破棄 | 無言・応答も返さない（CLI が応答待ちならハング要因） |
| control_response/request_id==releash-initialize | 部分変換 | commands のみ。subtype:error 不検査 |
| control_response/その他（set_model・set_permission_mode・interrupt への応答） | 破棄 | 成功もエラーも無言破棄 |
| control_cancel_request | 破棄 | 型未定義、catch-all で無言破棄・ログなし |
| keep_alive | 変換 | KeepAlive（stale 監視の progress 更新） |
| 未知 type / type フィールドなし | 破棄 | 無言・ログなし（非JSON行のみ process.rs:177 で warn ログ） |
| 8MB 超過行 | 可視化付き破棄 | OversizeDropped→Error part とカウント。ただしどの type の行を失ったかは不明のまま（result 行なら turn 終端喪失の可能性） |
既知問題1〜3は再報告せず、その同型（本文テキストの transient 依存、permission 取り下げの取りこぼし、control 応答の無言破棄）を中心に報告した。
```

### CX: Codex 入力側の「捨て」

```text
codex app-server 0.139.0（openai/codex tag rust-v0.139.0 の app-server-protocol を取得し照合。wire.rs 自身が 0.139.0 検証を明記）× Releash codex/{wire,convert,session,app_server}.rs の突き合わせ結果。

【notification × 処理有無】
error=部分変換(willRetry無視→Finding8) / thread/started=✓ / thread/status/changed=無視 / thread/archived・unarchived・closed=無視(未使用APIで概ね妥当) / skills/changed=無視 / thread/name/updated=無視 / thread/goal/updated・cleared=無視 / thread/settings/updated=無視 / thread/tokenUsage/updated=変換するが形状不一致で常に0(Finding4) / turn/started=turn_id捕捉のみ / hook/started・completed=無視 / turn/completed=✓(status・error.messageのみ。durationやerror.additionalDetails/codexErrorInfoは破棄) / turn/diff/updated=無視(Releash自前diffで代替、妥当寄り) / turn/plan/updated=無視(Finding5) / item/started=対応type限定(下表) / item/autoApprovalReview/started・completed=無視 / item/completed=対応type限定 / rawResponseItem/completed=無視(internal-only、妥当) / item/agentMessage/delta=✓ / item/plan/delta=無視(Finding5) / command/exec/outputDelta・process/outputDelta・process/exited=無視(未使用API、妥当) / item/commandExecution/outputDelta=✓(0.139.0はplain textでbase64問題なしを確認) / item/commandExecution/terminalInteraction=無視 / item/fileChange/outputDelta=✓(deprecated) / item/fileChange/patchUpdated=✓ / serverRequest/resolved=無視(Finding12) / item/mcpToolCall/progress=無視(長いMCP呼び出しが無進捗表示) / mcpServer/oauthLogin/completed・startupStatus/updated=無視 / account/updated・rateLimits/updated・login/completed=無視(rate limit逼迫が見えない) / app/list/updated・remoteControl/*・externalAgentConfig/*・fs/changed=無視(未使用API、妥当) / item/reasoning/summaryTextDelta・summaryPartAdded・textDelta=無視(Finding3) / thread/compacted=✓(deprecated) / model/rerouted=無視(Finding7) / model/verification=無視 / turn/moderationMetadata=無視 / warning・guardianWarning・deprecationNotice・configWarning=無視(Finding7) / fuzzyFileSearch/sessionUpdated・sessionCompleted=無視(session API未使用のため妥当) / thread/realtime/*=無視(未使用、妥当) / windows/*=無視。

【server request × 処理有無】
item/commandExecution/requestApproval=✓ / item/fileChange/requestApproval=✓ / item/tool/requestUserInput=変換するが question id・isOther・isSecret 喪失＋応答shape不一致で回答が破棄される(Finding1) / mcpServer/elicitation/request=無視・無応答→ハング(Finding2) / item/permissions/requestApproval=✓ / item/tool/call=無視・無応答(ただしReleashはdynamicTools未登録のため実質不発) / account/chatgptAuthTokens/refresh=無視・無応答(発生条件は外部auth管理時のみ) / attestation/generate=無視(initializeでrequestAttestation:false宣言のため妥当) / ApplyPatchApproval・ExecCommandApproval(legacy)=無視(turn/start経由では不発、妥当)。

【item type × 処理有無】
userMessage=無視(自前echoで妥当) / hookPrompt=無視(Finding10) / agentMessage=started/completedとも無視でdelta通知のみに依存(完了textとの照合なし。deltaは常時送出されるため許容と判断し非報告) / plan=無視(Finding5) / reasoning=誤フィールド参照で実質無視(Finding3) / commandExecution=✓(aggregatedOutput/exitCode/status反映。durationMs・commandActions・processIdは破棄=軽微) / fileChange=✓ / mcpToolCall=✓(text系contentのみ抽出、image/resource contentは破棄=軽微) / dynamicToolCall=✓(結果はitem全体のJSON dump) / collabAgentToolCall=無視(Finding10) / webSearch=開始✓・結果はプレースホルダ(Finding11) / imageView・imageGeneration=無視(Finding10) / enteredReviewMode・exitedReviewMode=無視(Finding10) / contextCompaction=✓(SystemNotification)。

【response 処理】result.thread.id=✓(SessionEstablished) / result.commands=dead path(0.139.0のinitialize responseに存在しない→Finding9) / turn/start error=✓(Error part+TurnFailed) / それ以外のerror response=warnログのみ(Finding6) / thread/resume responseのturns(履歴)=破棄(Releashは自前event logで再構築するため設計上妥当と判断)。

【スコープ外の隣接観察（報告件数には含めず）】models.rs の fuzzyFileSearch 呼び出しは 0.139.0 の FuzzyFileSearchParams が必須フィールド roots: Vec<String> を要求するのに {root, cwd, query, limit} を送っており invalid params になる可能性が高い（codex セッションのファイル検索が backend 経由で失敗）。担当ファイル外のため findings から除外した。

既知問題1〜3は再報告していない。全 finding は codex 側プロトコル定義（tag rust-v0.139.0 の実ソース）と Releash 実コードの両方で裏取り済み。
```

### SD: Claude / Codex で同じ概念の扱いが違う

```text
claude/{convert,session,permission,process,wire}.rs と codex/{convert,session,permission,app_server,wire}.rs を全読し、指定 10 概念（テキスト delta、thinking、tool ライフサイクル、token usage、エラー、turn 完了と interrupt、permission 写像、compaction、establish/resume、slash/skill）を突き合わせた。裏取りとして domain/gateway.rs、entities/message_part.rs（merge_part）、runtime/usecase.rs の apply_runtime_event / handle_resume_mismatch / stall watchdog / interrupt、runtime/stale.rs、event_log/finalization.rs、frontend（useAgentChat.ts、useAgentSdkListeners.ts、PermissionDialog.tsx、agentChatReducer.ts）を確認し、各 divergence の到達経路とユーザー可視性を検証した。対称だったため報告しなかった項目: 最終テキストは両者とも delta のみに依存（Claude assistant text block / Codex agentMessage completed を双方無視）、slash command の取得と SlashCommandsUpdated 変換、skills のファイルスキャン実装、turn 失敗時の Error part + Failed の順序、未応答 control/JSON-RPC への無応答リスク（両者同型）。Claude の AcceptEdits/Plan での client 側 auto-allow はテストが design §6.3 準拠を明記しているため意図的設計と判断し除外。Codex outputDelta と aggregatedOutput の重複表示は merge_part の contains ヒューリスティックで通常吸収されるため確信が持てず除外。
```

### OB: 送信側（ユーザー → agent）の差・喪失

```text
監査対象: ユーザー入力→backend 到達経路。runtime/usecase.rs の send_message（steer 分岐 292-336 / queue 分岐 337-373 / 直接 start 375-431）、start_turn_for_session（1375-1611）、start_next_queued_turn（3305-3592）と全エラー経路、handle_resume_mismatch（2083-2133）、interrupt（466-477）、cancel_queued_turn / close_session / close_all、stale watchdog と stall→steer 判定（1640-1674, 1840-1993）、apply_runtime_event の TurnCompleted/Fatal/SessionEstablished 処理。gateway.rs の TurnInput/steer/interrupt 定義と BackendCapabilities（Claude/Codex とも steering:false、steer 実装なしを確認）。claude/session.rs の start_turn/interrupt/replace_process/合成 abort、wire.rs の user_message/画像形式。codex/session.rs の start_turn/interrupt/turn_id ライフサイクル（convert.rs 119-124, 180）、permission settings、editor_context の additionalContext 送信。context/builder.rs と context_restore.rs（system prompt 構成・reinjection・prefix 二重適用の有無 — 二重適用は経路上不可能と確認）。frontend は useAgentChat.ts の sendMessage/interrupt エラー処理、agentChatReducer の interrupting、MessageInput の入力クリアを確認。permission profile の Claude 無視 / Codex plan-mode 上書きは実コードで確認したが、production で permission_profile_id を Some にする書き込み経路が現存しないため（機能休眠中）報告から除外。Claude の system prompt 変更ごとのプロセス replace→resume 挙動は CLI 側の resume 時 session id 保持仕様を確認できず未報告。
```

### RT: runtime 〜 event log 〜 read model 経路の喪失・変質

```text
runtime→event log→read model 経路を監査した。対象: runtime/usecase.rs(8380行) の send/queue/complete_turn/close/Fatal/stale watchdog/epoch guard、event_apply.rs、streaming.rs(coalesce/persist間隔)、stale.rs、event_log/(events.rs の durable event 全種別・projector.rs の fold・finalization.rs・part_events.rs の DurableOnly/FinalLiveBlocks 振り分け)、session/(store.rs・read_model.rs・lifecycle_controller.rs・stored_lifecycle.rs・message_window.rs)、adaptor/gateway の event_store.rs・tool_output_blob.rs。検証内容: durable 化されない AgentRuntimeEvent 種別の列挙(TokenUsageUpdated/SlashCommandsUpdated/Fatal(idle時)/KeepAlive は transient のみ、PartsMerged の Text/Thinking/Error はターン完了時 FinalPartsRecorded のみ)、append_session_event_without_projection の本番経路(append_durable_part_events のみ)、finalize_turn の呼び出し元が complete_turn 経由の1箇所のみであること、close/クラッシュ経路に finalize・flush が無いこと、tool output 外部化(30KB/1000行 preview+blob 参照で損失なし)、projector の orphan event 破棄と FinalPartsRecorded 上書き、epoch guard による drop、KeepAlive⇄watchdog(現行 watchdog は turn を畳まないため誤畳み経路なし)を確認。既知3件(transient permission / finalize時Cancelled / stream_emit_suppressed)と重複する報告は除外した。
```

### FE: frontend の見せ方

```text
担当範囲（frontend の見せ方）を全ファイル精読した: useAgentChat.ts(1631行), agentChatReducer.ts(868行), useAgentSdkListeners.ts(627行), useSessionStore.ts(541行), ChatSessionView.tsx(1832行), AgentChatPanel.tsx, BoundSessionChat.tsx, ActivityLog.tsx(994行), StreamMessage.tsx, PermissionDialog.tsx(1080行), toolPairing.ts, deriveActivityStatus.ts。backend 側は突合のため MessagePart enum(session/mod.rs:169)、TurnPhase(status.rs)、projector.rs / finalization.rs / part_events.rs、runtime/usecase.rs の complete_turn / Fatal / apply_parts / get_session、streaming.rs、claude/codex の convert・permission・session を検証した。検証して問題なしと判断した点: MessagePart 全 variant のレンダラ有無（todo_list_snapshot は footer、system_notification は inline、image/image_ref は human 側で表示、agent message への image は現状 producer が存在せず未達経路）、TurnPhase 3 値は全て UI 反映、Codex の tool 名は Rust 側で Claude 名（Bash/Edit/WebSearch）へ正規化済みで表示差なし、stall observation と contextCarry failed はバナー表示あり、permission revision guard は既知修正 #1379 と整合。当初疑った「Task 配下 permission part の不可視化（projector が parent=tool_use_id を書く問題）」は、FinalPartsRecorded が live parts で置換するため stored messages に到達しないことを確認し、誤報として除外した。報告 7 件は全て file:line まで裏取り済み。
```

### RG: 参照実装（Vibe Kanban / ACP）との語彙ギャップ

```text
対応表（vibe-kanban 正規化語彙 / ACP session/update 語彙 / Releash 語彙）:
[メッセージ] UserMessage / user_message_chunk / TurnStarted.prompt(durable) → 同等。AssistantMessage / agent_message_chunk / MessagePart::Text → 同等（ただし Claude は完全版 assistant メッセージの text を捨て delta のみ＝finding 8）。
[思考] Thinking / agent_thought_chunk / MessagePart::Thinking → 語彙はあるが実配線は Claude のみ。Codex は reasoning delta 未処理＋完了時抽出パス不一致で全損（finding 1）。
[ツール] ToolUse{tool_name, action_type: FileRead|FileEdit{FileChange}|CommandRun{exit_status,category}|Search|WebFetch|TaskCreate|PlanPresentation|TodoManagement|AskUserQuestion|Other, status: Created|Success|Failed|Denied{reason}|PendingApproval|TimedOut} / tool_call+tool_call_update{kind: read|edit|delete|move|search|execute|think|fetch|switch_mode|other, status: pending|in_progress|completed|failed, locations, rawInput, rawOutput} / MessagePart::ToolUse{tool名+生input}+ToolResult{is_error: bool} → Releash は種別(kind)を語彙に持たず、表示時に tool 名文字列で事後分類（adaptor/controller/command/agent_session/tool_activity.rs: read|write|command|mcp|other の5値。Codex は "Bash"/"Edit"/"WebSearch" へ名前偽装で相乗り）。状態は bool のみで denied/timed-out が失敗と同一（finding 4）。exit code 非構造化（finding 9）。画像結果破棄（finding 7）。
[plan/todo] TodoManagement{todos: status+priority}+PlanPresentation / plan{PlanEntry: status+priority} / TodoListSnapshot{text, completed:bool} → Claude 専用（Codex の turn/plan/updated・item/plan/delta・plan item は全破棄＝finding 2）、in_progress/priority 喪失（finding 5）。
[turn 終了理由] NextAction{failed}+ErrorMessage{SetupRequired|Other} / StopReason{end_turn|max_tokens|max_turn_requests|refusal|cancelled} / TurnResult{stop_reason: 実質常に None、TurnStopReason は Refusal のみでテスト以外に生成元なし} → 死語彙（finding 3）。
[使用量] TokenUsageInfo{total_tokens, context_window} / usage_update{used, size, cost} / TokenUsageUpdated{input,output,total,context_window} → tokens は同等、cost 欠落（finding 10）。
[システム通知] SystemMessage・ErrorMessage / session_info_update / SystemNotification{Compaction のみ} → Codex の warning/configWarning/deprecationNotice/model-rerouted/rateLimits は無音破棄（finding 6）。
[コマンド/モード] （なし）/ available_commands_update・current_mode_update・config_option_update / SlashCommandsUpdated・PermissionModeChanged → 同等（config_option は Releash はモデル選択等を別経路で保持）。
[質問/承認] UserAnsweredQuestions・AskUserQuestion・UserFeedback{denied_tool} / RequestPermission{PermissionOptionKind 4値} / PermissionRequestMsg{questions, allowed_prompts, decision_reason}+PermissionPartStatus{pending|allowed|denied|cancelled} → ほぼ同等（Releash はむしろ豊か）。
[Releash 独自の強み（参照実装に無い）] ToolOutputRef/ToolOutputSummary による大出力の id-based 遅延取得（full-retention 回避）、ImageRef/AttachmentRef の blob 外部化、TaskStatus（subagent 進捗の live 更新）、ToolCallRetried（retry の durable 語彙）、SystemNotification{Compaction}（compaction の可視化。vibe-kanban/ACP には無い）、KeepAlive（生存通知）、steer（turn 途中の再指示）、ResumeOutcome{Mismatch}（resume 不一致の一級表現）。
調査ソース: BloopAI/vibe-kanban crates/executors/src/logs/mod.rs（NormalizedEntryType/ActionType/ToolStatus/CommandExitStatus/TodoItem 全変種）と logs/utils/shell_command_parsing.rs（CommandCategory: Read|Search|Edit|Fetch|Other）、ACP schema v1（zed-industries/agent-client-protocol schema/v1/schema.json: SessionUpdate 11 変種・ToolKind 10 値・ToolCallStatus 4 値・StopReason 5 値・PlanEntry・UsageUpdate）、openai/codex app-server-protocol（v2 ThreadItem 18 変種・server notification 全列挙、Reasoning スキーマは rust-v0.63.0〜0.143.0-alpha.38 で検証）、Releash 側は gateway.rs / session/mod.rs / turn.rs / claude・codex convert.rs / event_log（events.rs, finalization.rs, projector.rs）/ tool_activity.rs / AgentChatPanel を読解。既知問題 1-3 は除外し、コード上で確証できた欠落・divergence のみ報告。
```

## 付録 B: 検証で却下された指摘（再調査不要）

監査で候補に挙がったが、検証者が実コードで反証した指摘。将来同じ疑いを再調査しないための記録。

### [CL] assistant 本文（text/thinking）が transient な stream delta のみに依存し、最終 assistant message と result.result を捨てているため、delta を1つでも失うと本文が恒久欠損する

却下理由: 構造的な核（本文 text/thinking の供給源が stream delta のみで、最終 assistant message の text ブロックは assistant_parts で捨てられ（convert.rs:264-298）、成功時 result.result も読まれない（convert.rs:205-234））は実コードで確認できた。しかし恒久欠損の根拠とされた欠損経路の分析に意味の誤りがある。(1) 経路(c) stream_emit_suppressed では本文は durable に残る: apply_parts は emit 判定前に無条件で domain_streaming_parts へ merge し（runtime/usecase.rs:2657-2662）、flush_streaming_update は emit_suppressed の early-return（usecase.rs:2816-2818）より前に persist_message_parts で定期永続化し（usecase.rs:2794-2814）、complete_turn が turn 終端で force flush（usecase.rs:3158）して append_final_turn_events が FinalLiveBlocks モードで TextRecorded/ReasoningRecorded を durable event log に書く（usecase.rs:3214-3224、part_events.rs:21-42）。よって抑制時もライブ表示が止まるだけで、再読込と workflow final_text_parts は復元される。「どれかが起きると本文は durable event log にも残らず」「再読込しても戻らない」は (c) について誤り。(2) 経路(a) 8MB超過行破棄は無言ではなく可視の Error part を合成し（claude/session.rs:400-407）、text/thinking delta は小さい chunk で届くため1行8MB超の delta は事実上発生しない（8MBで落ちるのは巨大 tool result 等で、その text はそもそも未使用）。残る現実的な恒久欠損トリガーは (b) 非JSON破損行の skip（process.rs:174-179）という稀なケースのみで、high の根拠だった「既知問題3と連動した恒久欠損」は成立しない。

### [CL] user メッセージの text ブロックを全破棄するため、CLI が合成するテキスト（built-in slash command のローカル出力等）がチャットに出ない

却下理由: コード記述（convert.rs:300-335 で user メッセージの text block を全破棄、SlashCommandsUpdated が convert.rs:105-108 / usecase.rs:2495-2498 経由で補完 UI に供給）は正確だが、症状の因果メカニズムが誤り。実際の claude CLI 2.1.195 に stream-json で /cost・/context を送って実測した結果、built-in ローカルコマンドの出力は user メッセージ（<local-command-stdout>）としてではなく "model":"<synthetic>" の assistant メッセージ（full text block、stream delta なし）として emit され、user タイプのメッセージは 1 件も出力されない。したがって user_parts の text 破棄は「/cost の出力が消える」症状の原因ではない。CLI が user メッセージとして text を合成する実例は現行統合では確認できず、指摘された経路での観測可能な被害は立証できない。なお症状自体（補完に出る built-in コマンドを実行しても何も表示されない）は実在するが、原因は assistant_parts（convert.rs:264-298）が tool_use block しか変換せず synthetic assistant メッセージの text block を落とすこと、および convert_result（convert.rs:205-234）が成功時 result テキストを表示に回さないことであり、別 finding として報告すべき内容。

### [CL] token usage を result（turn 終端）でしか反映せず、assistant message の usage / stream message_delta の usage を破棄しているため turn 中はメータが凍結する

却下理由: バックエンドのデータフロー記述は全て正確（TokenUsageUpdated は convert_result のみ: convert.rs:205-211 / stream_event_parts は content_block_delta 以外を破棄: convert.rs:236-240 / assistant_parts は tool_use のみで message.usage を読まない: convert.rs:264-298 / Interrupted 時は usage=None: runtime/usecase.rs:3183-3194）。しかし user_visible_symptom が成立しない。usage の消費先を末端まで追跡した結果、token usage を表示する UI コンポーネントが frontend に一切存在しない: presenter の agent-turn-usage-updated emit（adaptor/presenter/agent_session.rs:171-179）→ useAgentSdkListeners.ts:168 → agentChatReducer.ts:624 → useAgentChat.ts:1545 の getSessionLatestTokenUsage まで届くが、この getter の消費者はテスト mock（AgentChatPanel.test.tsx:279）のみで、inputTokens/outputTokens/contextWindowTokens/totalTokenUsage をレンダリングする箇所は src 全体に 0 件。auto-compact 接近表示も存在しない。「トークン/コンテキスト使用量の表示が凍結して見える」というメータはそもそも画面に無く、指摘された「チャットの不安定として見える」症状は現状観測不能。監査基準（意味的・観測可能な問題のみ、症状の誇張不可）に照らし不成立。

### [CX] serverRequest/resolved を捨てるため、codex 側で解決済みの approval dialog が turn 終了まで生き続ける

却下理由: コードレベルの事実（wire.rs に定数なし、convert.rs:216 で破棄、codex 0.139.0 に serverRequest/resolved が実在）は全て正確だが、指摘の前提と症状が意味的に誤っている。(1) codex は「client 応答以外の経路で解決されたとき」ではなく全解決時に emit する（bespoke_event_handling.rs の on_*_response ハンドラが receiver.await 後に無条件で resolve_server_request_on_thread_listener を呼ぶ）。client 自身の応答のエコーは Releash が respond_permission（runtime/usecase.rs:479-529）で自前解決するため冗長。(2) client 応答以外の解決経路は abort_pending_server_requests の3呼出（TurnStarted:154 防御的 / TurnComplete:184 / TurnAborted:1128）と thread unload のみで、全て turn 境界。TurnComplete/TurnAborted は同一イベント処理内で turn/completed（status completed/failed/interrupted）を送出し、Releash は convert.rs:179 でこれを TurnCompleted に変換して finalize が pending permission を畳むため、ダイアログは解決とほぼ同時（turn 境界）に消える。「turn 継続中に解決済みダイアログが turn 終了まで生き続ける」には turn 継続中の server 側解決経路が必要だが、単一 stdio 接続の本統合には存在しない。よって user_visible_symptom（死んだダイアログを長時間操作させられる）は成立せず、破棄は現状観測可能な影響を持たない。

### [SD] permission mode の source of truth: Claude は backend echo で同期、Codex は楽観更新のみで失敗も無言

却下理由: 指摘の Codex 側の事実関係（PermissionModeChanged emit 経路の不在、set_permission_mode の応答非待機、thread/settings/update エラー応答が pending_client_methods 未追跡で convert.rs:83-87 の log::warn のみで捨てられる）は全て実コードで確認できたが、核心の 2 点が意味的に誤っている。(1) Claude の同期方向が逆: PermissionModeChanged の唯一の消費先 runtime/usecase.rs:2481→resync_permission_mode（usecase.rs:4011-4050）は、CLI 報告モードが store と異なる場合に store のモードを CLI へ押し戻し、UI へも store の値を再通知する。「UI のモード表示を backend 実態に同期」「agent 起点のモード遷移も反映される」は誤りで、agent 起点の遷移はむしろ巻き戻される（store が source of truth）。(2) Codex の不整合は恒常的でない: 次の turn/start で build_turn_start_request（codex/session.rs:518-537）が TurnInput.permission_mode（frontend の現在表示モード、usecase.rs:412）から approvalPolicy/sandboxPolicy を再送するため、settings/update 失敗は次 turn 開始時に自己修復する。「Full 表示なのに承認ダイアログが出続ける」という恒常的症状は成立せず、影響は「turn 実行中のモード変更が拒否された場合にその turn の残りだけ旧ポリシーで進む」に限定される。また楽観更新（persist+notify 先行、runtime 同期失敗は warn のみ）は両 backend 共通の意図的仕様で、テスト set_permission_mode_persists_and_notifies_when_runtime_sync_fails（usecase.rs:6141）で固定されている。

### [SD] token usage の更新タイミングと内訳が backend ごとに別物（同一フィールドに異なる意味）

却下理由: Claude 側の記述（result 受信時に 1 回だけ emit、cache 合算、modelUsage.contextWindow=容量値）は claude/convert.rs:82,205-234,494-530 で全て確認できた。しかし Codex 側の記述が実プロトコルと不一致。Releash は wire.rs で experimentalApi:true の v2 API を初期化しており、実際に spawn される codex-cli 0.142.5 から生成した JSON Schema（ThreadTokenUsageUpdatedNotification）では thread/tokenUsage/updated の params は { threadId, turnId, tokenUsage: { last, total, modelContextWindow } } で、数値は tokenUsage.total/.last の下にネストされる。codex/convert.rs:658-685 の token_usage_from_value は params.usage（存在しない）→ params 直下の inputTokens/outputTokens/contextWindowTokens（いずれも存在しない）を読むため、「inputTokens をそのまま使い contextWindowTokens を透過する」のではなく、実際は TokenUsage{input:0, output:0, total:Some(0), context_window:None} が毎回 emit される。さらに「cache 合算なしで Claude が桁違いに大きく見える」という機序も誤り: OpenAI 系では cachedInputTokens は inputTokens の内数であり、仮にパースが機能しても Claude 側の合算はむしろ意味を揃える方向。指摘の「更新頻度の差」自体は事実だが、値の意味論と user_visible_symptom の機序が実挙動と異なるため、この指摘のままでは不正確。

### [RT] token usage が transient のみで reload で消え、durable 側も context_window_tokens / total_tokens / 中断ターンの usage を落とす

却下理由: データフロー上の記述はすべて実コードで確認できたが、指摘の核心である user_visible_symptom が現行コードでは発生し得ないため不成立。(1) 確認できた点: TokenUsageUpdated はメモリ+emit のみ (runtime/usecase.rs:2499-2508)、durable TurnTokenUsage は input/output のみ (event_log/events.rs:97-100, usecase.rs:4159-4166)、TurnCompleted.token_usage を latest_token_usage へ読み戻す経路は皆無、SessionPage.latest_token_usage は全構築箇所で None ハードコード (message_store.rs:541, session/store.rs:426, prompt_suggestion.rs:308) で get_session:1011 のフォールバックは常に空、interrupted turn は projector.rs:895-902 で token_usage: None。(2) しかし「チャットのコンテキスト使用量/トークン表示が空にリセットされる」は誤り: トークン表示 UI は commit 5b37349d3 (#1150, 2026-06-15) で ChatSessionView.tsx から削除済みで、現 HEAD の src/ には latestTokenUsage / totalTokenUsage を render するコンポーネントが一切存在しない（getSessionLatestTokenUsage は useAgentChat.ts:1545 で公開されるが消費者ゼロ、workflow の totalTokenUsage も types/workflow.ts:217 の型定義のみ）。表示自体が無い以上「reload で表示が消える」というユーザー可視の不安定は起こらない。(3) 細部の誤り: usecase.rs:3183-3191 は「中断ターンの usage を破棄」ではなく、domain の TurnResult::Interrupted (domain/agent_session/entities/turn.rs:14-17) が構造的に token_usage フィールドを持たないため捨てるデータ自体が到達しない。また同関数は latest_token_usage を Some の時のみ上書きするため、ターン中の TokenUsageUpdated で得た値は中断後もメモリ上には残る。監査基準（意味的・観測可能な問題のみ、ユーザー可視の症状必須）に照らすと、これは観測不能な潜在的 state 層の欠損であり報告対象外。

### [RG] Claude の完全版 assistant メッセージの text/thinking ブロックを converter が捨て、本文の永続化が transient な stream delta のみに依存する（既知問題1/3 と同型）

却下理由: 構造的事実は正しいが、指摘の中核である因果連鎖（既知問題3発火時に本文が永続ログからも消える）が実コードと矛盾する。確認できた事実: (1) claude/convert.rs:264-298 の assistant_parts は type=="tool_use" のみ処理し（275行）、完全版 assistant メッセージの text/thinking ブロックを破棄する。(2) Text/Thinking part の生成元は stream_event_parts（convert.rs:236-262、content_block_delta のみ）だけで、完全版からのバックフィル経路は存在しない（session.rs:364 が唯一の変換入口、part_records_durable_event（runtime/usecase.rs:3900-3912）も Text/Thinking を durable per-part event に記録しない）。しかし反証: stream_emit_suppressed は UI emit（ctx.notifier.streaming_delta）だけを抑止し、永続化には影響しない。apply_parts（runtime/usecase.rs:2657-2662）は emit 判定より前に無条件で state.domain_streaming_parts へ delta を merge し、flush_streaming_update の snapshot 永続化（usecase.rs:2794-2814）は emit_suppressed の early return（2816行）より前に実行される。turn 終了時の complete_turn は in-memory の streaming_parts（3204行）を FinalPartsRecorded（3214-3224行、append_final_turn_events は usecase.rs:4052-4069 で全 parts を event log に記録）と persist_message_parts（3239-3248行）で永続化するため、既知問題3発火時でも本文は永続ログに残り、turn 完了後/リロード後には表示される。失われるのは turn 中のライブストリーミング表示のみ。残るのは「wire レベルで delta 自体が届かない場合」だが、コード上の現実的な欠落機構は 8MB 超行の破棄（claude/session.rs:345-353）と非JSON行スキップ（process.rs:174-179）のみで、前者は小さな delta 行には当たらず（むしろ巨大な完全版メッセージ側が破棄される）、「delta だけ欠落して完全版は届く」前提条件の現実的な発火経路は示されていない。よって user_visible_symptom（「永続ログからも消える」）は意味レベルで不正確。
