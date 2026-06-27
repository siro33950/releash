# Design — issues-1248

対象 Issue: #1248「Agent context epoch と instruction resolution policy を導入する」
対象 requirements: `docs/specs/issues-1248/requirements.md`
対象 behavior: `docs/specs/issues-1248/behavior.md`

本書は requirements の R1〜R7 / AC1〜AC5 と behavior の各 Scenario を、現行実装の調査結果に基づいて実装設計へ落とし込む。behavior が design 責務に委ねた具体化事項（epoch / revision の版管理アルゴリズム、探索範囲の階層境界、`CONTEXT.md` 等の追加対象、replacement の再構築粒度）は本書で確定する。

## 概要

現状、Agent に投入される system context（repo summary / diff / open editor / mentions / terminal log / workflow state / instructions）は、それぞれ別経路で個別に組み立てられ、「どの版か」「いつ差し替えるか」を統一管理する保持単位を持たない。調査で確認した現状は次のとおり。

1. **会話復帰と system context 鮮度が未分離**: `ContextRestorePlan`（`infrastructure/agent_session/runtime/context_restore.rs:20-49`、Resume / Reinject / NoContext）は**会話メッセージ履歴**の復帰戦略のみを扱い（#1190 の責務）、repo / diff / instruction の鮮度は対象外。
2. **workflow state が context に伝播していない**: `WorkflowStepContextDto`（`usecase/agent_session/session/mod.rs:214-224`）は型として存在するが、`ChatSession.workflow_step_context` は実質 None で、Agent context へ載っていない。
3. **instruction 解決ルールが未確立**: `AGENTS.md` / `CLAUDE.md` の探索・重複回避は未実装。system prompt は `compose_system_prompt()`（`bridge_common/shared.rs:1026-1033`）で CLI help を載せるのみ。workflow facet の instruction は `compose_from_parts()`（`adaptor/gateway/workflow/facet.rs:348-392`）で `user_message` に直結し、リポジトリ階層 instruction や read file 近傍 instruction との重複回避を持たない。
4. **共通保持単位の不在**: 各 context source を「どの epoch / revision として保持し、いつ破棄・再構築するか」を表す型が無いため、stale 文脈・残留 instruction・重複投入が構造的に防げない（failure mode 1〜4）。

本設計では、(a) context source を列挙し epoch / revision を持つ保持単位（`ContextSource` / `ContextSnapshot` / `ContextEpoch`）として Rust ドメインに定義し、(b) backend / model / worktree / instruction file 変更を契機とする replacement ルールをドメインサービスとして実装し、(c) `AGENTS.md` 相当の探索範囲・重複回避を Rust usecase に集約し、(d) これらを Agent への context 投入経路（init コマンド構築・メッセージ送信）に統合する。#1190 の会話復帰経路とは責務を分離し共存させる。

## 変更対象

### Rust（src-tauri）

新規:

- `src-tauri/src/domain/agent_session/value_objects/context_epoch.rs`（新規）
  - `ContextEpoch` / `ContextEpochId` / `ContextRevision` / `ContextSourceKind` / `ContextSnapshot` / `ContextSourceState` / `ReplacementTrigger` / `ReplacementAction` の型定義。
- `src-tauri/src/domain/agent_session/services/context_replacement.rs`（新規。`services.rs` を mod 化、または `services.rs` 内へ追加）
  - replacement ルール（trigger × source → action）の純粋判定。
  - instruction 重複回避の純粋判定（fingerprint / 正規化パスによる dedup）。
- `src-tauri/src/usecase/agent_session/context/mod.rs`（新規）
  - `SystemContextBuilder`（各 source を port 経由で取得し epoch / revision を付与、replacement を適用、投入対象を構成）。
  - `ContextEpochState`（current epoch + source kind → 最新 snapshot のマップ）。
- `src-tauri/src/usecase/agent_session/context/instruction_resolver.rs`（新規）
  - `AGENTS.md` / `CLAUDE.md` の探索（worktree 階層・read file 近傍）と収集・dedup。

変更:

- `src-tauri/src/usecase/agent_session/session/mod.rs`
  - `ChatSession` に epoch 識別情報を保持するフィールドを追加（後述 `context_epoch` メタ）。`workflow_step_context` を SystemContextBuilder の入力として実際に使う。
- `src-tauri/src/infrastructure/agent_session/resolver_ports.rs`
  - instruction file 探索・読み取り用 port（`InstructionSourcePort`）、repo summary / diff 取得 port を整理（既存 `BaseBranchResolverPort` / `MentionResolverPort` に追加）。
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/shared.rs`
  - `compose_system_prompt()` / `build_init_cmd()` を、SystemContextBuilder の出力（dedup 済み instruction を含む system context）を受け取る形へ拡張。
- `src-tauri/src/infrastructure/agent_session/runtime/mod.rs`（`AgentBackendRegistry` 周辺）
  - backend / model 切替を SystemContextBuilder へ `ReplacementTrigger` として伝える結線。
- `src-tauri/src/agent_message_dispatcher.rs`
  - メッセージ送信時に editor_context / mentions / workflow_step_context の生入力を SystemContextBuilder へ渡す。

### フロントエンド（src）

- 原則変更なし（D8 / R6 / AC4）。`send_agent_message` は既に `mentions` / `editor_context` を生のまま渡す構成（`adaptor/controller/command/agent_session/session.rs:1251-1313`、`AgentEditorContext`）。本要求では context 構築ロジックがフロントに残っていないことを確認し、必要なら生入力の受け渡し漏れ（編集中ファイル・選択範囲・mentions）のみを補う。新規の context 組み立て・stale 判定・重複回避はフロントに置かない。

## アーキテクチャと責務分割

ロジックは Rust に集約する（`.claude/rules/rust-first-logic.md`）。レイヤ責務は次のとおり。

- **domain（`domain/agent_session`）**: context の保持単位の型（`ContextSource` / `ContextSnapshot` / `ContextEpoch` 相当）と、外部 I/O に依存しない純粋判定（replacement ルール、instruction dedup）を持つ。
- **usecase（`usecase/agent_session/context`）**: port 経由で各 source の取得結果を集め、epoch / revision を付与し、replacement を適用し、投入対象 context を構成するオーケストレーション。instruction の探索・収集・dedup の集約点（R4 / R7 / AC2）。
- **infrastructure（`infrastructure/agent_session`）**: port の実装（ファイル走査、repo summary / diff 取得は既存 `CodeUsecase`、terminal log・workflow state の取得結果の受け渡し）と、Agent backend（Claude / Codex）への投入結線。
- **adaptor / frontend**: 生入力を usecase に渡すだけ（R6 / AC4）。

### epoch / revision モデル（R2 / AC1 / AC5 failure mode 1）

2 階層で鮮度を表す（behavior A5 を実装化）。

- **epoch**: context 全体の鮮度世代。session の「context を全体として無効化しうる属性」= `(backend_id, model_id, worktree_path)` の組で identity を定める。この組のいずれかが変わると新 epoch を採番する（`ContextEpochId` を単調増加）。
- **revision**: 個々の context source の版。source の入力（内容 fingerprint）が変わるたびに当該 source の `ContextRevision` を単調増加させる。

各 `ContextSnapshot` は採取時点の `(epoch_id, revision, fingerprint)` を持つ。stale 判別は behavior どおり「snapshot の epoch_id が current epoch_id と不一致」または「同一 source により新しい revision が存在する」のいずれかで成立し、stale snapshot は投入対象から除外する。

> 設計判断（epoch 採番の単位）: epoch identity を `(backend, model, worktree)` に限定する。instruction file 変更は epoch を進めず、instruction source の revision のみを進める（replacement 対象を instruction context に限定するため）。これにより「backend / model / worktree 変更＝全体世代交代」「ファイル内容変更＝該当 source のみ版更新」が一貫する。

### replacement ルール（R3 / AC5 failure mode 2、behavior「context replacement」）

`ReplacementTrigger`（`BackendChanged` / `ModelChanged` / `WorktreeChanged` / `InstructionFileChanged` / `None`（通常ターン））に対し、source ごとの `ReplacementAction`（`Discard`：投入を止め再解決まで載せない / `Rebuild`：その場で再取得して新版を作る / `Retain`：直前版を据え置き再構築しない）を純粋関数で決める。

| source | BackendChanged | ModelChanged | WorktreeChanged | InstructionFileChanged | None |
| --- | --- | --- | --- | --- | --- |
| repo summary | Retain | Retain | Rebuild | Retain | Retain |
| diff / review snapshot | Retain | Retain | Rebuild | Retain | Retain |
| open editor / selection | Retain | Retain | Rebuild（W2 基準に再スコープ） | Retain | Retain |
| mentions | Retain | Retain | Rebuild | Retain | Retain |
| terminal log summary | Retain | Retain | Retain | Retain | Retain |
| workflow run / step state | Retain | Retain | Retain | Retain | Retain |
| project instructions（AGENTS.md 相当） | Discard→Rebuild | Discard→Rebuild | Rebuild | Rebuild | Retain |
| backend/model identity | Discard→Rebuild | Discard→Rebuild | Retain | Retain | Retain |

要点:

- **backend / model 切替時**（failure mode 2 / behavior「前 backend / 前 model 向け instruction・identity payload が残留しない」）: backend/model identity（`backend_id` / `model_id` 文字列 payload）と project instructions を `Discard` してから新 backend / 新 model 向けに `Rebuild`。`Discard` は「旧版を context から外す」ことを保証する（再解決前に旧版が残らない）。runtime system prompt 本文は context epoch の保持対象ではなく、本文差し替えは既存の `system_prompt_fingerprint` 系統が担う。
- **worktree 切替時**（behavior「repo 由来の context が再構築される」）: repo summary / diff / open editor / mentions / project instructions を W2 基準で `Rebuild`、W1 基準の旧版は破棄。terminal log / workflow state は worktree 非依存として `Retain`。
- **instruction file 変更時**（behavior「変更前の instruction は据え置かれない」）: project instructions のみ `Rebuild`。epoch は進めない（revision のみ更新）。
- **通常ターン（None）**（behavior「該当しない context source は据え置かれる」）: 全 source `Retain`。直前と同一版はそのまま据え置き再構築しない。

> 設計判断（mentions の WorktreeChanged）: mentions は worktree 内パス参照が前提のため、worktree 切替で解決し直す（`Rebuild`）。ユーザ入力としての mentions テキストは次メッセージ送信時に新 worktree 基準で再解決される。

### instruction 解決と重複回避（R4 / AC2、behavior「instruction 解決と重複回避」）

`SystemContextBuilder` 内の `instruction_resolver` に集約する（R7）。

**探索範囲（R4）**:

- 対象ファイル名は **`AGENTS.md` と `CLAUDE.md`**（requirements A4）。
- **リポジトリ階層**: worktree ルートを上限境界とし、worktree ルートから対象ディレクトリへ降りる各階層の `AGENTS.md` / `CLAUDE.md` を収集する。worktree ルートより上位（親リポジトリ・ホーム等）は探索しない。
- **read file 近傍**（failure mode 4 / behavior「read した file 近傍の局所 instruction を投入する」）: Agent が read した file のディレクトリから worktree ルートまでの経路上にある `AGENTS.md` / `CLAUDE.md` を局所 instruction として収集する。

> 設計判断（CONTEXT.md / 階層境界）: OpenCode は `AGENTS.md` / `CLAUDE.md` / `CONTEXT.md` を対象とするが、requirements A4 の最小集合（`AGENTS.md` / `CLAUDE.md`）を本実装の対象とする。`ContextSourceKind` / 探索ロジックは対象ファイル名を定数リストで保持し、`CONTEXT.md` 等の追加を後日リストへ加えるだけで拡張できる構造とする（本要求では既定リストに含めない）。探索の上限境界は worktree ルートに固定する（worktree 外への走査は行わない）。

**重複回避（R4 / failure mode 3、behavior「同一 instruction は重複投入されない」）**:

- 同一 instruction が複数経路（リポジトリ instruction / workflow facet instruction / read file 近傍 instruction）から到達しても 1 回だけ投入する。
- dedup キーは 2 種で判定する: (1) **正規化済み絶対パス**（同一ファイルがリポジトリ階層経路と read file 近傍経路の双方から来た場合）、(2) **内容 fingerprint（ハッシュ）**（workflow facet instruction とリポジトリ instruction が同一内容の場合のように、パスが異なる／パスを持たない経路同士）。いずれかが一致すれば重複として除外する。
- 投入順序は OpenCode 同様「広いスコープ → 狭いスコープ」（worktree ルート → 深い階層 → read file 近傍）とし、より局所の instruction を後段に置く。workflow facet instruction は別チャネルとして合流し、内容 fingerprint で project instructions と dedup する。
- これにより `compose_from_parts()` が instruction を `user_message` に無条件直結していた現状を、dedup 済み instruction を構成する経路へ置き換える。

### #1190 との整合と責務分離（R5 / AC3、behavior「epoch / revision は会話履歴復帰と独立」）

- `ContextRestorePlan`（#1190、`context_restore.rs`）= **会話メッセージ履歴**の復帰（Resume / Reinject / NoContext）。本要求の epoch / revision = **system context** の鮮度管理。両者は同一 session 内で独立に判定され共存する。
- behavior「会話履歴は復帰しつつ system context は最新版に差し替える」を満たすため、SystemContextBuilder は会話復帰の成否（`ContextCarryState::Resumed / Reinjected / Failed`）を入力に取らず、復帰時点の current epoch（= 復帰時の backend / model / worktree）で system context を構成する。会話履歴復帰の失敗は epoch 判定を変更しない。
- 復帰時の system context は「中断時点の古い snapshot」ではなく「復帰時点の current epoch の版」を投入する（failure mode 1）。

## データモデルまたは型

`domain/agent_session/value_objects/context_epoch.rs`（新規）。domain 層は `docs/architecture/DOMAIN.md` の規約に従い、`serde` 等の外部依存を持たない pure な値オブジェクトとして定義する。永続化・転送用の serde 付き表現は後述の `usecase/agent_session/context_meta.rs` 側 DTO に分離する。

```rust
/// context 全体の鮮度世代。session 単位で単調増加。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextEpochId(pub u64);

/// 個々の context source の版。source 単位で単調増加。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextRevision(pub u64);

/// 投入しうる context source の列挙（R1 / AC1、最小 7 種）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextSourceKind {
    RepoSummary,
    DiffReviewSnapshot,
    OpenEditorSelection,
    Mentions,
    TerminalLogSummary,
    WorkflowState,
    ProjectInstructions,
    BackendModelIdentity,
}

/// BackendModelIdentity は backend/model identity payload
/// (`backend_id` / `model_id` 文字列) だけを保持する。
/// runtime system prompt 本文は context epoch の snapshot として保存せず、
/// 本文置換は `system_prompt_fingerprint` 系統で管理する。
/// 永続化 key は既存 session 互換のため `"backend_system_prompt"` を維持し、
/// Rust 上の source 名とディスク上の legacy key を分離する。

/// epoch の identity。この組が変わると新 epoch を採番する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEpoch {
    pub id: ContextEpochId,
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub worktree_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEpochIdentity {
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub worktree_path: String,
}

/// ある source の、ある時点での取得結果（保持単位の最小要素）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSnapshot {
    pub kind: ContextSourceKind,
    pub epoch_id: ContextEpochId,
    pub revision: ContextRevision,
    /// 内容の同一性判定・dedup・stale 判定に用いる指紋。
    pub fingerprint: String,
    /// Agent へ投入する本文（取得済み結果。取得アルゴリズム自体は対象外＝R7）。
    pub payload: String,
}

/// source ごとの現在状態（最新 snapshot を保持。欠落も表現）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSourceState {
    pub kind: ContextSourceKind,
    pub latest: Option<ContextSnapshot>,
    pub revision_counter: ContextRevision,
}

/// replacement の契機。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementTrigger {
    None,
    BackendChanged,
    ModelChanged,
    WorktreeChanged,
    InstructionFileChanged,
}

/// source に対する replacement の指示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementAction {
    Retain,
    Rebuild,
    Discard,
}
```

instruction の収集中間表現:

```rust
/// 1 件の instruction（解決前の収集結果）。
pub struct ResolvedInstruction {
    pub origin: InstructionOrigin,        // RepoHierarchy / FileNeighbor / WorkflowFacet
    pub source_path: Option<PathBuf>,     // ファイル由来なら正規化済み絶対パス
    pub content: String,
    pub fingerprint: String,              // 内容ハッシュ
    pub scope_depth: usize,               // 投入順序（広い→狭い）用
}
```

永続化・転送表現（`usecase/agent_session/context_meta.rs`）。`meta.json` へ保存する公開メタは epoch identity / revision / fingerprint に限定し、Agent 投入本文は serde 上 skip する。Retain 用 payload cache は `private_context.json` に分離する。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextSourceRevisionMeta {
    pub kind: String,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fingerprint: Option<String>,
    #[serde(skip, default)]
    pub payload: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextSourcePayloadCache {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fingerprint: Option<String>,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextEpochMeta {
    pub epoch_id: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model_id: Option<String>,
    pub worktree_path: String,
    #[serde(default)]
    pub source_revisions: Vec<ContextSourceRevisionMeta>,
}
```

`ChatSession`（`usecase/agent_session/session/mod.rs`）への追加（永続化）:

```rust
/// 現在の epoch identity と source 別 revision の公開メタ。
/// snapshot 本文は meta.json へは永続化しない。
#[serde(skip_serializing_if = "Option::is_none", default)]
pub context_epoch: Option<ContextEpochMeta>,
```

`ContextEpochMeta` は `epoch_id` と `(backend_id, model_id, worktree_path)`、および source kind → revision / fingerprint / runtime payload の表を持つ。これにより再起動・復帰後も「current epoch」と「各 source の最新 revision」を継続でき、stale 判定が再起動を跨いで成立する。runtime payload は `ContextSourceRevisionMeta.payload` として保持するが、serde 上は skip し、`meta.json` には本文を出さない。

snapshot payload は Retain 時に前回 snapshot を current epoch へ再タグするための cache として必要なため、`message_store.rs` が session dir の `private_context.json` に `SessionPrivateContext.context_epoch_payloads` として保存する。保存内容は source kind / fingerprint / payload に限定し、load 時は `private_context.json` から `ContextEpochMeta` へ hydrate する。したがって永続メタの責務は「`meta.json` に epoch identity・revision・fingerprint を保存すること」、private context の責務は「Agent 投入本文を公開メタから分離して payload cache として保持すること」とする。

## 処理フロー

### A. epoch の解決と更新

1. メッセージ送信／復帰起動時、現在の `(backend_id, model_id, worktree_path)` を取得。
2. `ChatSession.context_epoch` の identity と比較し、差分から `ReplacementTrigger` を導出（backend 差→`BackendChanged`、model 差→`ModelChanged`、worktree 差→`WorktreeChanged`、いずれも無→`None`）。差分があれば新 `ContextEpochId` を採番。
3. instruction file の変更検知（後述）があれば、`InstructionFileChanged` を併発トリガとして扱う（epoch は進めず instruction revision のみ更新）。

### B. SystemContextBuilder による context 構成

1. 生入力（editor_context / mentions / workflow_step_context）と、port 経由の取得結果（repo summary / diff / terminal log）、instruction_resolver の収集結果を集める。
2. source ごとに replacement ルール（trigger × kind → action）を適用:
   - `Retain`: runtime payload、または復帰時に `private_context.json` から hydrate した payload cache を使い、直前 snapshot を current epoch に再タグして据え置く（再取得しない）。
   - `Rebuild`: 取得結果から新 snapshot を作り、fingerprint が前版と異なれば revision を進める。
   - `Discard`: 当該 source の最新を投入対象から外し、再解決まで載せない。
3. 投入対象 = current epoch に属し非 stale な snapshot 集合。stale（epoch_id 不一致／より新しい revision あり）は除外。

### C. instruction 解決（B の一部）

1. worktree ルート → 対象ディレクトリへ降りる各階層、および read file 近傍経路の `AGENTS.md` / `CLAUDE.md` を `InstructionSourcePort` で走査・読み取り。
2. workflow facet instruction（`compose_facets` 由来）を `WorkflowFacet` origin として合流。
3. dedup（正規化パス／内容 fingerprint）で重複を 1 件に畳む。
4. `scope_depth` 昇順（広い→狭い）で並べ、`ProjectInstructions` snapshot の payload として確定。

### D. Agent への投入

- `build_init_cmd()` / `compose_system_prompt()`（`bridge_common/shared.rs`）が SystemContextBuilder の出力を受け取り、dedup 済み instruction を含む system context を backend へ載せる。backend/model 切替時は旧 instruction / backend model identity payload が `Discard` 済みのため残留しない。runtime system prompt 本文の置換は `system_prompt_fingerprint` により別系統で行う。
- workflow facet の `compose_from_parts()`（`facet.rs`）は instruction を `user_message` へ無条件直結する現経路を、SystemContextBuilder の dedup を通す経路に置き換える。

## エラー処理

behavior「Rust 実装の正常系・異常系」に対応。

- **instruction file 読み取り失敗**（behavior 異常系）: 当該ファイルをスキップし、読めた instruction のみ投入する。失敗は他 source の保持・投入へ波及させない（source 単位で隔離）。`instruction_resolver` は `Result` ではなく「収集できたものの集合＋スキップ件数」を返し、致命化しない。
- **context source の取得結果欠落**（behavior 異常系）: 当該 source の `ContextSourceState.latest` を `None`（欠落）として扱い、他 source の epoch / revision 判定は継続。欠落 source は投入対象に含めないだけで epoch 判定を破綻させない。
- **epoch 採番の整合**: epoch identity の取得に失敗した場合（worktree path 不明等）は新規 epoch を作らず直前 epoch を据え置く（context を全消去しない安全側）。
- 各モジュールは専用エラー型を持ち（コーディング規約）、usecase 境界で文字列化して返す既存方針に合わせる。

## テスト方針

### Rust 単体テスト（domain：純粋ロジック）

- **replacement マトリクス**: 各 `ReplacementTrigger` × `ContextSourceKind` の `ReplacementAction` が表どおりであること（backend/model 切替で instructions / backend model identity payload が `Discard`、worktree 切替で repo 系が `Rebuild`、None で全 `Retain`）。
- **stale 判定**: epoch_id 不一致・より新しい revision 存在の双方で stale になること。一致版は非 stale。
- **instruction dedup**: 同一パス（複数経路）・同一内容 fingerprint（異なる経路）で 1 件に畳まれること。`scope_depth` 順序が広い→狭いであること。

### Rust 単体テスト（usecase：オーケストレーション、port は fake）

- **探索範囲**: fake fs で worktree 階層・read file 近傍の `AGENTS.md` / `CLAUDE.md` が収集され、worktree ルート外は収集されないこと。
- **正常系**（behavior）: 各 source が epoch / revision を伴って保持され、current epoch の版を投入対象として取り出せること。
- **異常系**（behavior）: instruction の 1 ファイルが読めなくても他 instruction / 他 source が維持されること。terminal log summary 欠落時に他 source の epoch 判定が機能すること。
- **backend / model / worktree 切替**: 切替後に旧 instruction / 旧 backend model identity payload が投入対象から消え、新版に置き換わること（failure mode 2）。
- **復帰整合（#1190）**: 会話復帰の成否（`ContextCarryState`）に依らず、復帰時 current epoch の version で system context が構成されること（failure mode 1 / AC3）。

### フロントエンド

- 新規ロジックを持たないため、ロジック追加テストは不要。生入力（editor_context / mentions）の受け渡し経路に変更が及ぶ場合のみ既存テストを更新。

### 配置

- domain / usecase の各モジュール内に `#[cfg(test)] mod tests`（プロジェクト規約）。port は trait の fake 実装でテストする。

## リスクと代替案

### リスク

- **既存 system prompt 経路への影響**: `compose_system_prompt()` / `build_init_cmd()` は全 backend の投入経路の根。SystemContextBuilder 統合で投入内容が変わるため、回帰（system prompt 欠落・二重投入）に注意。投入前後の snapshot を比較するテストで担保する。
- **workflow facet 経路の置き換え**: `compose_from_parts()` の instruction 直結を dedup 経路へ移すと、workflow step の prompt 構成が変わりうる。workflow 系テストの回帰確認が必要。
- **epoch 採番の過剰更新**: backend/model/worktree の取得タイミング差で不要な epoch 更新が起きると repo 系を過剰に `Rebuild` し性能劣化する。identity 比較を厳密化し、None トリガで全 Retain を保証する。
- **instruction file 変更検知の精度**: ファイル変更検知（`watcher.rs` 既存）と revision 更新の結線が緩いと、変更が反映されない／過剰反映する。最小実装では「メッセージ送信時に対象 instruction の fingerprint を再算出して差分検知」する（watcher 連携は別途）。

### 代替案（不採用理由つき）

- **epoch を単一カウンタにし全 source を一括世代管理**（revision を持たない）: 実装は単純だが、instruction file 変更のたびに repo summary / diff まで `Rebuild` され失敗 mode を別途生む。source 別 revision を採る本案を採用。
- **instruction 解決を frontend で実施**: R6 / AC4（Rust 集約）に反するため不採用。
- **OpenCode の `SystemContext.Source<A>` / `SessionContextEpoch` をそのまま移植**（`prepare` / `initialize` / `requestReplacement` / `current`）: 抽象は参考にするが、Releash の既存型（`ChatSession` / `AgentEditorContext` / facet）に合わせた最小型に絞る。完全移植は過剰。

## 仮定

- **A1**: epoch identity は `(backend_id, model_id, worktree_path)` の組とする。instruction file 変更は epoch を進めず instruction source の revision のみ進める（本書「epoch 採番の単位」）。
- **A2**: instruction 対象ファイルは `AGENTS.md` / `CLAUDE.md` の 2 種を既定リストとする（requirements A4）。`CONTEXT.md` 等は定数リスト拡張で追加可能とし、本要求の既定には含めない（OpenCode 参照を踏まえた design 確定）。
- **A3**: instruction 探索の上限境界は worktree ルートとする（worktree 外・親リポジトリは走査しない）。
- **A4**: read file 近傍 instruction の「近傍」は、read した file のディレクトリから worktree ルートまでの経路上の `AGENTS.md` / `CLAUDE.md` とする。
- **A5**: snapshot の payload（取得結果本文）は `ChatSession.context_epoch` / `meta.json` には永続化しない。一方で Retain のために必要な payload cache は、source kind / fingerprint / payload の最小形で `private_context.json` の `SessionPrivateContext.context_epoch_payloads` へ保存し、load 時に `ContextEpochMeta` の runtime payload へ hydrate する。payload cache が無い source は既存取得結果から `Rebuild` する。
- **A6**: 各 context source の取得アルゴリズム自体（repo summary 生成・diff 取得・terminal log 要約）は新規実装せず、既存取得結果を本モデルの保持単位・replacement ルールに載せる（requirements R7 / 非スコープ）。
- **A7**: instruction file 変更検知は最小実装として「メッセージ送信時の fingerprint 再算出による差分検知」を採り、`watcher.rs` とのリアルタイム連携は本要求の対象外とする。

## Open Questions

なし。

behavior が design 責務に委ねた具体化事項（epoch / revision 版管理アルゴリズム＝本書「epoch / revision モデル」、探索範囲の階層境界＝A3 / A4、`CONTEXT.md` 等の追加対象＝A2、replacement の再構築粒度＝replacement マトリクス）は本書で確定済み。requirements の Q1（実装範囲）は「Rust 実装まで含む」で確定済み。
