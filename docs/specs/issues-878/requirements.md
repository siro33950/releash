# Requirements

## Type

Final sweep ISSUE。親 ISSUE ではない。

関連: #878 / #985 / #986 / #1130 / #1131 / #1132 / #1133 / #1134 / #1217 / #1302 / #1303 / #1304 / #1305 / #1306 / #1307

## 背景と目的

Issue #878 は、clean architecture 移行および frontend logic migration の完了後に実施する最後の dead-code / stale surface sweep である。active behavior を新 layer へ移す ISSUE ではなく、先行 ISSUE により未使用になった旧 surface、互換経路、警告抑制、到達不能コードを削除する。

本 ISSUE の目的は、現在の UI / CLI / API から到達できる active code だけを残し、それ以外の production code を削除することである。単に compiler warning が出ている箇所だけでなく、警告が抑制されているコード、現在 import されていないコード、型や関数としては参照が残っているが UI / CLI / API から到達できない compatibility path も削除対象とする。

## スコープ

- 未使用 Tauri command / command registration。
- Rust 側 command が存在しない frontend invocation（例: `kill_lsp_by_worktree`）。
- UI / workflow 経路が残っていない OneShot PTY remnants。
- `docs/specs/issues-1217/dead-code-candidates.md` に記録された compatibility path。
- migration 後に残った旧 root module declaration。
- 不要になった `#[allow(...)]` / `#[expect(...)]` 等の警告抑制。
- 未使用 protocol DTO / handler / gateway shim / presenter shim。
- 削除済み compatibility surface だけを検証していた test。
- #1302-#1307 により不要になった frontend helper / invoke wrapper / compatibility path。
- 削除対象 code のみに依存していた Cargo dependency / frontend dependency。

## 非スコープ

- active module や frontend-owned application behavior を clean architecture へ移す作業。これは先行 ISSUE（#985 / #986 / #1130 / #1131 / #1132 / #1133 / #1134 / #1302-#1307）で完了している前提とする。
- 現在の UI / CLI / API から到達可能な active behavior の仕様変更。
- 新しい UI、Tauri command、CLI subcommand、WebSocket / remote access surface の追加。
- current surface の backward compatibility を保つために必要な code の削除。
- test-only helper の全面削除。ただし production code に残す理由が test のみである場合は、`#[cfg(test)]` へ閉じるか削除する。

## 要求事項

### R1. active surface を source of truth とする

- 削除可否は、現在の UI / CLI / API から到達できるかで判定すること。
- frontend の `invoke` / event listener、Tauri command registration、CLI subcommand、Rust usecase / gateway の production call graph を確認し、到達できない production code は削除すること。
- test からのみ参照される production code は active surface とみなさないこと。

### R2. stale root module declaration の削除

- migration 後に残った旧 root module declaration を削除すること。
- 完了時点で、以下の stale root module declaration が残っていないこと。
  - `git_host`
  - `notion`
  - `protocol`
  - `review_comments`
  - `ws_bridge`
  - `ws_server`
- 削除済み root module への直接 command registration が残っていないこと。

### R3. 未使用 command / invoke / helper の削除

- current frontend / CLI / API から呼ばれていない command、invoke wrapper、frontend helper、compatibility path を削除すること。
- Rust 側 command が存在しない `kill_lsp_by_worktree` frontend invocation は削除すること。
- #1302-#1307 の移行で Rust read model / command へ置き換わった frontend path / branch / repository status / agent stream / diff / markdown / terminal helper の旧経路を削除すること。

### R4. 未使用 protocol / WebSocket / remote access surface の削除

- current UI / CLI / API から利用されていない WebSocket server / bridge / middleware / remote access 経路を削除すること。
- 削除対象の protocol DTO、handler、auth / rate-limit / session middleware、broadcaster、gateway shim、presenter shim を production code から除去すること。
- 削除した surface のためだけに残っていた tests も削除すること。

### R5. PTY remnants の削除

- UI / workflow 経路が残っていない OneShot PTY remnants を削除すること。
- `PtyKind` 等、current interactive terminal surface で不要になった value object / DTO / branch は削除すること。
- OneShot PTY を残す場合は、owning module に active surface として必要な根拠を明記すること。ただし active 根拠がない場合は削除を優先する。

### R6. compatibility path の削除

- `docs/specs/issues-1217/dead-code-candidates.md` に記録された compatibility path を再検証し、current UI / CLI / API から到達できないものを削除すること。
- legacy bridge branch、development-only resolver、旧 resource path handling 等は、active runtime path として必要な根拠がない限り削除すること。

### R7. 警告抑制の削除

- dead code / unused import / unused item / warnings を抑制する `#[allow(...)]` / `#[expect(...)]` を削除すること。
- warning suppression を外した結果 compile warning が出る場合は、警告を再抑制せず、対象 code または import を削除すること。
- `cargo clippy -- -D warnings` と `cargo clippy --tests -- -D warnings` が、migration 由来の dead-code / unused warning なしで通ること。

### R8. dependency cleanup

- 削除した surface のみに使われていた Cargo dependency / feature / frontend dependency を削除すること。
- lockfile は実際の dependency graph と整合していること。

### R9. observable behavior の不変

- current UI / CLI / API の observable behavior は変更しないこと。
- 削除対象は current surface から到達できない code に限定し、既存の active command / screen / CLI workflow の入出力 schema と error behavior を維持すること。

### R10. 検証

- Rust 側で以下が通ること。
  - `cargo fmt --check`
  - `cargo clippy -- -D warnings`
  - `cargo clippy --tests -- -D warnings`
  - `cargo test`
- frontend / integration 側で以下が通ること。
  - `pnpm lint`
  - `pnpm test`
  - `pnpm build`
  - `pnpm test:integration`
- 代表的な residue search を実行し、削除対象の識別子・警告抑制・削除済み module への参照が残っていないことを確認すること。

## 受け入れ基準の概要

- stale root module declaration（`git_host` / `notion` / `protocol` / `review_comments` / `ws_bridge` / `ws_server`）と、削除済み root module への直接 command registration が残っていない。
- `rg 'kill_lsp_by_worktree' src src-tauri/src` が結果を返さない。
- OneShot PTY / `PtyKind` / current surface から到達できない PTY branch が残っていない、または active 根拠が owning module に明記されている。
- `docs/specs/issues-1217/dead-code-candidates.md` の候補が再検証され、到達不能な compatibility path は削除されている。
- 未使用 protocol DTO / handler / gateway shim / presenter shim / remote access / WebSocket / middleware 経路が production code に残っていない。
- dead code / unused import / unused item / warnings を抑制する `#[allow(...)]` / `#[expect(...)]` が残っていない。
- 削除済み compatibility surface だけを検証していた tests が残っていない。
- 削除した code のみに依存していた dependency が削除され、lockfile が更新されている。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo clippy --tests -- -D warnings` / `cargo test` / `pnpm lint` / `pnpm test` / `pnpm build` / `pnpm test:integration` が通る。

## 仮定

- **A1**: #878 は依存 ISSUE（#985 / #986 / #1130 / #1131 / #1132 / #1133 / #1134 / #1302-#1307）が完了した後に実施する final sweep である。
- **A2**: current UI / CLI / API は、checked-in frontend、Tauri command registration、CLI subcommand、production Rust call graph から判定する。過去の compatibility surface や外部から直接使われていた可能性だけでは retention の根拠にしない。
- **A3**: test-only code は `#[cfg(test)]` に閉じる。production module に残す理由が test だけである code は削除対象とする。
- **A4**: current surface から到達できない WebSocket / remote access / compatibility DTO は backward compatibility の対象外とする。

## Open Questions

なし。
