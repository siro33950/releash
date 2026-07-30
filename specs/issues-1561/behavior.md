> **要求は requirements.md が正、配置は規約が正。**
> 本書が定める振る舞いは満たすべきものである。一方、本書に現れる個別のファイル・型の配置や分類は判断の結果ではなく参考にすぎない。どのコードをどの層へ置くかは `docs/architecture/` の規約（DOMAIN / USECASE / GATEWAY / INFRASTRUCTURE / CONTROLLER / TEST）を各コードに当てて決めること。本書の配置記述が規約と食い違う場合は規約に従う。

# Behavior

本変更は構造の再配置であり、外部から観測される session の振る舞いを変えない（requirements.md Non-goals）。したがって振る舞い定義は「変わらないこと」を対象とする。

R-001 / R-002 / R-003 / R-007 は構造要件であり、対応する外部観測可能な振る舞いを持たない。表現主体の所在、状態定義の一本化、層の分解はいずれもコード構造の性質であって、利用者や外部 surface から観測できる事象ではない。これらを Gherkin として書くと「文書に記述がある」「特定の型が存在しない」という実装整合性の主張になり、振る舞い定義の対象から外れる。検証手段は design.md の Cross-cutting concerns / Verification に定める。

## B-001: 構造整理を跨いだ既存保証の維持

GIVEN 整理前の実装で `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` の不変条件 I1〜I17 が満たされている
WHEN session lifecycle の判断主体が domain 集約へ移され、usecase / gateway / infrastructure の責務が再配置される
THEN I1〜I17 の各保証は整理後も同じ意味と水準で観測される
AND normal、failure、crash、restart の各経路で観測される結果は整理前と同一である

I1〜I17 の内容は正本が所有する。本書はそれを再掲せず、参照によって維持対象を指定する。個別の受け入れ判定は正本の定義と既存テストによる。

## B-002: Audit finding の再分類

GIVEN 既存 audit 台帳の 66 件の finding と整理後の Session 構造がある
WHEN 再監査が完了する
THEN 各 finding は「構造整理で解消」「残存」「新規発見」のいずれかへ再分類されている
AND #1565 で是正済みの finding が再分類結果へ反映されている

## B-003: Phase 3 以降の Issue 再編

GIVEN audit finding の再分類結果が確定している
WHEN milestone 84 の phase plan が更新される
THEN Phase 3 以降の Issue 群の吸収、close、または再スコープが再分類結果に基づいて反映されている
AND Phase 3 以降の行動修復は実施されず、各 Issue の契約内容は変更されていない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID | 備考 |
| --- | --- | --- |
| R-001 | — | 構造要件。観測可能な振る舞いを持たない |
| R-002 | — | 構造要件。観測可能な振る舞いを持たない |
| R-003 | — | 構造要件。観測可能な振る舞いを持たない |
| R-004 | B-001 | 維持対象の定義は `agent-chat-ideal-lifecycle.md` I1〜I17 が正本 |
| R-005 | B-002 | |
| R-006 | B-003 | |
| R-007 | — | 構造要件。観測可能な振る舞いを持たない |
