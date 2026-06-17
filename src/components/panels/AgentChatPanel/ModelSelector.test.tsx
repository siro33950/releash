import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ModelSelector } from "./ModelSelector";

const models = [{ value: "claude-4" }, { value: "claude-3.5" }];

describe("ModelSelector", () => {
	it("shows the current model id as the trigger label", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-3.5"
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"claude-3.5",
		);
	});

	it("does not show an Unset option in the dropdown", async () => {
		const user = userEvent.setup();
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		expect(screen.queryByText("Unset")).toBeNull();
		expect(screen.queryByTestId("model-selector-clear")).toBeNull();
	});

	it("shows selected model value when a model is selected", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"claude-4",
		);
	});

	it("calls onModelChange with model value when a model is selected", async () => {
		const user = userEvent.setup();
		const onModelChange = vi.fn();
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-3.5"
				onModelChange={onModelChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		await user.click(screen.getByText("claude-4"));
		expect(onModelChange).toHaveBeenCalledWith("claude-4");
	});

	it("never calls onModelChange with null (no unset path)", async () => {
		const user = userEvent.setup();
		const onModelChange = vi.fn();
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={onModelChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		await user.click(screen.getByText("claude-3.5"));
		expect(onModelChange).toHaveBeenCalledWith("claude-3.5");
		for (const call of onModelChange.mock.calls) {
			expect(call[0]).not.toBeNull();
		}
	});

	it("enables trigger when models list is non-empty", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toBeEnabled();
	});

	it("does not include an Auto fallback option in the dropdown", async () => {
		const user = userEvent.setup();
		const onModelChange = vi.fn();
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={onModelChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		// 候補に "Auto" は含まれない（暗黙のフォールバック候補を提示しない）
		expect(screen.queryByText("Auto")).toBeNull();
	});

	it("disables trigger when disabled prop is true", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={vi.fn()}
				disabled={true}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toBeDisabled();
	});

	it("keeps trigger enabled when models list is empty", () => {
		render(
			<ModelSelector
				models={[]}
				currentModelId=""
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		// 空一覧でも選択UI 自体は開ける（仕様: モデル一覧が空でも選択UIは提示）
		expect(screen.getByTestId("model-selector-trigger")).toBeEnabled();
	});

	it("shows zero candidates when models list is empty", async () => {
		const user = userEvent.setup();
		render(
			<ModelSelector
				models={[]}
				currentModelId=""
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		// 候補は 0 件として提示され、暗黙のフォールバック候補や代替文言は含まれない。
		expect(screen.queryAllByRole("menuitem")).toHaveLength(0);
		expect(screen.queryByText("Auto")).toBeNull();
		expect(screen.queryByText("候補なし")).toBeNull();
	});

	it("renders dangerous model identifiers as plain text", async () => {
		// spec: 表示時に副作用を起こしうる文字を含むモデル識別子は文字列として
		// のみ表示され、画面上で実行・解釈されない。
		const user = userEvent.setup();
		const dangerous = "<script>window.__pwned=true</script>";
		const onerrorImg = '" onerror="alert(1)';
		const danger = [{ value: dangerous }, { value: onerrorImg }];

		render(
			<ModelSelector
				models={danger}
				currentModelId={dangerous}
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);

		const trigger = screen.getByTestId("model-selector-trigger");
		expect(trigger).toHaveTextContent(dangerous);

		await user.click(trigger);

		// candidate も textContent として表示され、script タグ / onerror 属性が
		// DOM 上に副作用を持つ要素として現れない。
		const items = screen.getAllByRole("menuitem");
		const itemTexts = items.map((el) => el.textContent ?? "");
		expect(itemTexts).toContain(dangerous);
		expect(itemTexts).toContain(onerrorImg);

		// script タグや onerror 属性付き img が DOM に挿入されていない（React の
		// 既定のテキストエスケープにより文字列として描画される）。
		// onerror is technically a valid attribute name on many HTML elements,
		// so we only assert that no element actually carries it as an attribute
		// or that no <script> tag appears in the rendered tree.
		expect(document.querySelectorAll("script").length).toBe(0);
		expect(document.querySelectorAll("[onerror]").length).toBe(0);
		// And the dangerous payload did not run.
		expect(
			(window as unknown as { __pwned?: boolean }).__pwned,
		).toBeUndefined();
	});
});
